use std::{env, fs};

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use regex::Regex;

use crate::{RuntimeConfig, app::resolve::read_shim_target};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimInfo {
    pub name: String,
    pub path: String,
    pub source: String,
    pub kind: String,
    pub alternatives: Vec<String>,
    pub is_global: bool,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShimLookup {
    Found(ShimInfo),
    Missing { other_scope_exists: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlterShimOutcome {
    Altered {
        name: String,
        from: String,
        to: String,
    },
    NoAlternatives {
        name: String,
    },
    Missing {
        other_scope_exists: bool,
    },
}

pub fn add_shim(
    config: &RuntimeConfig,
    name: &str,
    command_path: &str,
    args: &[String],
    global: bool,
) -> anyhow::Result<()> {
    let shims_dir = shims_dir(config, global);
    fs::create_dir_all(&shims_dir).with_context(|| format!("failed to create {}", shims_dir))?;

    let resolved_target = resolve_command_target(config, command_path, global)?
        .ok_or_else(|| anyhow::anyhow!("Command path does not exist: {command_path}"))?;
    let lower = name.to_ascii_lowercase();

    if let Some(existing) = find_shim_path(&shims_dir, &lower)? {
        backup_or_remove_existing(config, &shims_dir, &lower, &existing)?;
    }

    match resolved_target
        .extension()
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("ps1") => {
            write_ps1_shim(&shims_dir, &lower, &resolved_target, args)?;
            write_cmd_shim(&shims_dir, &lower, &resolved_target, args)?;
        }
        _ => {
            write_metadata_shim(&shims_dir, &lower, &resolved_target, args)?;
            write_cmd_shim(&shims_dir, &lower, &resolved_target, args)?;
        }
    }

    Ok(())
}

pub fn remove_shims(
    config: &RuntimeConfig,
    names: &[String],
    global: bool,
) -> anyhow::Result<Vec<String>> {
    let shims_dir = shims_dir(config, global);
    if !shims_dir.exists() {
        return Ok(names.to_vec());
    }

    let mut missing = Vec::new();
    for name in names {
        let lower = name.to_ascii_lowercase();
        let removed = remove_matching_shim_files(&shims_dir, &lower)?;
        if !removed {
            missing.push(name.clone());
        }
    }
    Ok(missing)
}

pub fn list_shims(
    config: &RuntimeConfig,
    patterns: &[String],
    global_only: bool,
) -> anyhow::Result<Vec<ShimInfo>> {
    let patterns = patterns
        .iter()
        .map(|pattern| Regex::new(pattern).with_context(|| format!("Invalid pattern: {pattern}")))
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut rows = Vec::new();
    if !global_only {
        rows.extend(list_scope_shims(config, false, &patterns)?);
    }
    rows.extend(list_scope_shims(config, true, &patterns)?);
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(rows)
}

pub fn shim_info(config: &RuntimeConfig, name: &str, global: bool) -> anyhow::Result<ShimLookup> {
    let lower = name.to_ascii_lowercase();
    let shim_dir = shims_dir(config, global);
    if let Some(path) = find_shim_path(&shim_dir, &lower)? {
        return Ok(ShimLookup::Found(build_shim_info(config, &path, global)?));
    }
    Ok(ShimLookup::Missing {
        other_scope_exists: find_shim_path(&shims_dir(config, !global), &lower)?.is_some(),
    })
}

pub fn alter_shim(
    config: &RuntimeConfig,
    name: &str,
    global: bool,
) -> anyhow::Result<AlterShimOutcome> {
    let lower = name.to_ascii_lowercase();
    let shim_dir = shims_dir(config, global);
    let Some(current_path) = find_shim_path(&shim_dir, &lower)? else {
        return Ok(AlterShimOutcome::Missing {
            other_scope_exists: find_shim_path(&shims_dir(config, !global), &lower)?.is_some(),
        });
    };
    let info = build_shim_info(config, &current_path, global)?;
    let Some(next) = info
        .alternatives
        .iter()
        .find(|alt| **alt != info.source)
        .cloned()
    else {
        return Ok(AlterShimOutcome::NoAlternatives {
            name: info.name.clone(),
        });
    };

    for suffix in [".shim", ".cmd", ".ps1"] {
        let current_file = shim_dir.join(format!("{lower}{suffix}"));
        let backup_file = shim_dir.join(format!("{lower}{suffix}.{}", info.source));
        let next_file = shim_dir.join(format!("{lower}{suffix}.{next}"));
        if current_file.is_file() {
            let _ = fs::rename(&current_file, &backup_file);
        }
        if next_file.is_file() {
            let _ = fs::rename(&next_file, &current_file);
        }
    }

    Ok(AlterShimOutcome::Altered {
        name: info.name,
        from: info.source,
        to: next,
    })
}

fn list_scope_shims(
    config: &RuntimeConfig,
    global: bool,
    patterns: &[Regex],
) -> anyhow::Result<Vec<ShimInfo>> {
    let shims_dir = shims_dir(config, global);
    if !shims_dir.exists() {
        return Ok(Vec::new());
    }

    let mut selected = std::collections::BTreeMap::<String, Utf8PathBuf>::new();
    for entry in
        fs::read_dir(&shims_dir).with_context(|| format!("failed to read {}", shims_dir))?
    {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|_| anyhow::anyhow!("shim path must be valid UTF-8"))?;
        if !path.is_file() {
            continue;
        }
        let Some(priority) = shim_priority(&path) else {
            continue;
        };
        let Some(name) = shim_name(&path) else {
            continue;
        };
        if !patterns.is_empty() && !patterns.iter().any(|pattern| pattern.is_match(&name)) {
            continue;
        }
        match selected.get(&name) {
            Some(existing) if shim_priority(existing).unwrap_or(usize::MAX) <= priority => {}
            _ => {
                selected.insert(name, path);
            }
        }
    }

    selected
        .into_values()
        .map(|path| build_shim_info(config, &path, global))
        .collect()
}

fn build_shim_info(
    config: &RuntimeConfig,
    path: &Utf8Path,
    global: bool,
) -> anyhow::Result<ShimInfo> {
    let name = shim_name(path).ok_or_else(|| anyhow::anyhow!("invalid shim path {}", path))?;
    let target = read_shim_target(path)?;
    let source = infer_source(config, &target).unwrap_or_else(|| String::from("External"));
    let alternatives = alternatives_for(config, path, &name, &source, global)?;
    let launch_path = preferred_launch_path(path);
    let is_hidden = path_lookup(&name)
        .map(|resolved| !same_path(&resolved, &launch_path))
        .unwrap_or(true);

    Ok(ShimInfo {
        name,
        path: launch_path.to_string(),
        source,
        kind: if path.extension() == Some("ps1") {
            String::from("ExternalScript")
        } else {
            String::from("Application")
        },
        alternatives,
        is_global: global,
        is_hidden,
    })
}

fn alternatives_for(
    config: &RuntimeConfig,
    path: &Utf8Path,
    name: &str,
    source: &str,
    global: bool,
) -> anyhow::Result<Vec<String>> {
    let mut alternatives = Vec::new();
    if source != "External" {
        alternatives.push(source.to_owned());
    }
    let prefix = format!("{name}{}", preferred_extension(path));
    for entry in fs::read_dir(shims_dir(config, global))
        .with_context(|| format!("failed to read {}", shims_dir(config, global)))?
    {
        let entry = entry?;
        let entry_name = entry.file_name().to_string_lossy().to_string();
        if let Some(suffix) = entry_name.strip_prefix(&format!("{prefix}."))
            && !alternatives
                .iter()
                .any(|value| value.eq_ignore_ascii_case(suffix))
        {
            alternatives.push(suffix.to_owned());
        }
    }
    Ok(alternatives)
}

fn preferred_launch_path(path: &Utf8Path) -> Utf8PathBuf {
    match path.extension() {
        Some("ps1") | Some("shim") => path.with_extension("cmd"),
        _ => path.to_path_buf(),
    }
}

fn preferred_extension(path: &Utf8Path) -> &'static str {
    match path.extension() {
        Some("ps1") => ".ps1",
        Some("cmd") => ".cmd",
        _ => ".shim",
    }
}

fn backup_or_remove_existing(
    config: &RuntimeConfig,
    shims_dir: &Utf8Path,
    lower: &str,
    existing_path: &Utf8Path,
) -> anyhow::Result<()> {
    let info = build_shim_info(
        config,
        existing_path,
        same_path(shims_dir, &config.global_paths().shims()),
    )?;
    if info.source != "External" {
        for suffix in [".shim", ".cmd", ".ps1"] {
            let current = shims_dir.join(format!("{lower}{suffix}"));
            if current.is_file() {
                let backup = shims_dir.join(format!("{lower}{suffix}.{}", info.source));
                let _ = fs::rename(&current, &backup);
            }
        }
    } else {
        remove_matching_shim_files(shims_dir, lower)?;
    }
    Ok(())
}

fn remove_matching_shim_files(shims_dir: &Utf8Path, lower: &str) -> anyhow::Result<bool> {
    let mut removed = false;
    for entry in fs::read_dir(shims_dir).with_context(|| format!("failed to read {}", shims_dir))? {
        let entry = entry?;
        let entry_name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if entry_name == lower
            || entry_name.starts_with(&format!("{lower}.shim"))
            || entry_name.starts_with(&format!("{lower}.cmd"))
            || entry_name.starts_with(&format!("{lower}.ps1"))
        {
            removed = true;
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(removed)
}

fn write_metadata_shim(
    shims_dir: &Utf8Path,
    lower: &str,
    target: &Utf8Path,
    args: &[String],
) -> anyhow::Result<()> {
    let shim_path = shims_dir.join(format!("{lower}.shim"));
    let mut content = format!("path = \"{}\"\r\n", target);
    if !args.is_empty() {
        content.push_str(&format!("args = {}\r\n", args.join(" ")));
    }
    fs::write(&shim_path, content).with_context(|| format!("failed to write {}", shim_path))
}

fn write_cmd_shim(
    shims_dir: &Utf8Path,
    lower: &str,
    target: &Utf8Path,
    args: &[String],
) -> anyhow::Result<()> {
    let cmd_path = shims_dir.join(format!("{lower}.cmd"));
    let cmd = format!(
        "@rem {target}\r\n@\"{target}\"{} %*",
        if args.is_empty() {
            String::new()
        } else {
            format!(" {}", args.join(" "))
        }
    );
    fs::write(&cmd_path, cmd).with_context(|| format!("failed to write {}", cmd_path))
}

fn write_ps1_shim(
    shims_dir: &Utf8Path,
    lower: &str,
    target: &Utf8Path,
    args: &[String],
) -> anyhow::Result<()> {
    let ps1_path = shims_dir.join(format!("{lower}.ps1"));
    let ps1 = format!(
        "# {target}\r\n$path = \"{target}\"\r\nif ($MyInvocation.ExpectingInput) {{ $input | & $path{} @args }} else {{ & $path{} @args }}\r\nexit $LASTEXITCODE",
        render_powershell_args(args),
        render_powershell_args(args),
    );
    fs::write(&ps1_path, ps1).with_context(|| format!("failed to write {}", ps1_path))
}

fn render_powershell_args(args: &[String]) -> String {
    if args.is_empty() {
        String::new()
    } else {
        format!(
            " {}",
            args.iter()
                .map(|arg| format!("\"{}\"", arg.replace('"', "`\"")))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}

fn resolve_command_target(
    config: &RuntimeConfig,
    command_path: &str,
    global: bool,
) -> anyhow::Result<Option<Utf8PathBuf>> {
    if looks_like_path(command_path) {
        return canonical_utf8(command_path).transpose();
    }
    let lower = command_path.to_ascii_lowercase();
    if let Some(path) = find_shim_path(&shims_dir(config, global), &lower)? {
        return Ok(Some(read_shim_target(&path)?));
    }
    if let Some(path) = find_shim_path(&shims_dir(config, !global), &lower)? {
        return Ok(Some(read_shim_target(&path)?));
    }
    Ok(path_lookup(command_path))
}

fn path_lookup(command: &str) -> Option<Utf8PathBuf> {
    let path = env::var("PATH").ok()?;
    let pathext = env::var("PATHEXT").unwrap_or_else(|_| String::from(".COM;.EXE;.BAT;.CMD"));
    let extensions = if command.contains('.') {
        vec![String::new()]
    } else {
        pathext
            .split(';')
            .map(|ext| ext.to_ascii_lowercase())
            .collect::<Vec<_>>()
    };

    for dir in env::split_paths(&path) {
        for extension in &extensions {
            let candidate = if extension.is_empty() {
                dir.join(command)
            } else {
                dir.join(format!("{command}{extension}"))
            };
            if candidate.is_file() {
                return Utf8PathBuf::from_path_buf(candidate).ok();
            }
        }
    }
    None
}

fn infer_source(config: &RuntimeConfig, target: &Utf8Path) -> Option<String> {
    let normalized_target = target.as_str().replace('\\', "/").to_ascii_lowercase();
    for root in [config.paths().root(), config.global_paths().root()] {
        let marker = format!(
            "{}/apps/",
            root.as_str().replace('\\', "/").trim_end_matches('/')
        )
        .to_ascii_lowercase();
        if let Some(relative) = normalized_target.strip_prefix(&marker) {
            return relative
                .split('/')
                .next()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
        }
    }
    None
}

fn shims_dir(config: &RuntimeConfig, global: bool) -> Utf8PathBuf {
    if global {
        config.global_paths().shims()
    } else {
        config.paths().shims()
    }
}

fn find_shim_path(shims_dir: &Utf8Path, lower: &str) -> anyhow::Result<Option<Utf8PathBuf>> {
    for extension in ["shim", "ps1", "cmd"] {
        let path = shims_dir.join(format!("{lower}.{extension}"));
        if path.is_file() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn shim_name(path: &Utf8Path) -> Option<String> {
    let file_name = path.file_name()?;
    Some(
        file_name
            .strip_suffix(".shim")
            .or_else(|| file_name.strip_suffix(".ps1"))
            .or_else(|| file_name.strip_suffix(".cmd"))?
            .to_owned(),
    )
}

fn shim_priority(path: &Utf8Path) -> Option<usize> {
    match path.extension() {
        Some("shim") => Some(0),
        Some("ps1") => Some(1),
        Some("cmd") => Some(2),
        _ => None,
    }
}

fn looks_like_path(value: &str) -> bool {
    value.contains('\\') || value.contains('/') || value.contains(':')
}

fn canonical_utf8(path: &str) -> Option<anyhow::Result<Utf8PathBuf>> {
    if !Utf8Path::new(path).exists() {
        return None;
    }
    Some(
        fs::canonicalize(path)
            .with_context(|| format!("failed to resolve {}", path))
            .map(|resolved| {
                let rendered = resolved.to_string_lossy();
                Utf8PathBuf::from(rendered.strip_prefix(r"\\?\").unwrap_or(rendered.as_ref()))
            }),
    )
}

fn same_path(left: &Utf8Path, right: &Utf8Path) -> bool {
    left.as_str().eq_ignore_ascii_case(right.as_str())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{AlterShimOutcome, add_shim, alter_shim, list_shims, remove_shims, shim_info};

    #[test]
    fn adds_lists_and_removes_custom_shim() {
        let fixture = Fixture::new();
        fixture.write("local", "tools\\demo.exe", b"binary");

        add_shim(
            &fixture.config(),
            "demo",
            &format!("{}\\tools\\demo.exe", fixture.local_root),
            &[],
            false,
        )
        .expect("shim should be added");

        let rows = list_shims(&fixture.config(), &[], false).expect("shims should list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "demo");

        let missing =
            remove_shims(&fixture.config(), &[String::from("demo")], false).expect("rm works");
        assert!(missing.is_empty());
        assert!(
            list_shims(&fixture.config(), &[], false)
                .expect("shims should list")
                .is_empty()
        );
    }

    #[test]
    fn info_reports_other_scope_when_name_exists_elsewhere() {
        let fixture = Fixture::new();
        fixture.write("global", "shims\\demo.cmd", b"@rem C:\\demo.exe");

        let info = shim_info(&fixture.config(), "demo", false).expect("info should work");
        assert_eq!(
            info,
            super::ShimLookup::Missing {
                other_scope_exists: true
            }
        );
    }

    #[test]
    fn alter_switches_to_backup_alternative() {
        let fixture = Fixture::new();
        fixture.write("local", "apps\\demo\\current\\demo.exe", b"binary");
        fixture.write("local", "apps\\other\\current\\demo.exe", b"binary");
        add_shim(
            &fixture.config(),
            "demo",
            &format!("{}\\apps\\demo\\current\\demo.exe", fixture.local_root),
            &[],
            false,
        )
        .expect("shim should be added");
        add_shim(
            &fixture.config(),
            "demo",
            &format!("{}\\apps\\other\\current\\demo.exe", fixture.local_root),
            &[],
            false,
        )
        .expect("shim should be replaced");

        let outcome = alter_shim(&fixture.config(), "demo", false).expect("alter should work");
        assert_eq!(
            outcome,
            AlterShimOutcome::Altered {
                name: String::from("demo"),
                from: String::from("other"),
                to: String::from("demo"),
            }
        );
    }

    struct Fixture {
        _temp: TempDir,
        local_root: Utf8PathBuf,
        global_root: Utf8PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            let local_root = root.join("local");
            let global_root = root.join("global");
            fs::create_dir_all(local_root.join("shims")).expect("local shims should exist");
            fs::create_dir_all(global_root.join("shims")).expect("global shims should exist");
            Self {
                _temp: temp,
                local_root,
                global_root,
            }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::new(self.local_root.clone(), self.global_root.clone())
        }

        fn write(&self, scope: &str, relative: &str, content: &[u8]) {
            let root = match scope {
                "local" => &self.local_root,
                "global" => &self.global_root,
                _ => panic!("unknown scope"),
            };
            let path = root.join(relative);
            fs::create_dir_all(path.parent().expect("fixture file should have a parent"))
                .expect("fixture parent should exist");
            fs::write(path, content).expect("fixture file should be written");
        }
    }
}
