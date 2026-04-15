use std::{env, fs};

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use regex::Regex;

use crate::{RuntimeConfig, domain::paths::ScoopPaths};

pub fn resolve_prefix(config: &RuntimeConfig, app: &str) -> anyhow::Result<Option<Utf8PathBuf>> {
    match resolve_prefix_in_paths(config.paths(), app) {
        Some(result) => result.map(Some),
        None => match resolve_prefix_in_paths(config.global_paths(), app) {
            Some(result) => result.map(Some),
            None => Ok(None),
        },
    }
}

pub fn resolve_which(config: &RuntimeConfig, command: &str) -> anyhow::Result<Option<Utf8PathBuf>> {
    if let Some(path) = resolve_command_from_path(command)? {
        return resolve_command_path(config, &path);
    }

    match resolve_which_in_paths(config.paths(), command)? {
        Some(path) => Ok(Some(path)),
        None => match resolve_which_in_paths(config.global_paths(), command)? {
            Some(path) => Ok(Some(path)),
            None => Ok(None),
        },
    }
}

fn resolve_prefix_in_paths(paths: &ScoopPaths, app: &str) -> Option<anyhow::Result<Utf8PathBuf>> {
    let current_dir = paths.current_dir(app);
    current_dir.exists().then_some(Ok(current_dir))
}

fn resolve_which_in_paths(
    paths: &ScoopPaths,
    command: &str,
) -> anyhow::Result<Option<Utf8PathBuf>> {
    let shims = paths.shims();
    if !shims.exists() {
        return Ok(None);
    }

    let command_path = Utf8Path::new(command);
    let base_name = command_path.file_stem().unwrap_or(command);
    let extension = command_path.extension();

    Ok(shim_candidates(base_name, extension)
        .into_iter()
        .map(|candidate| shims.join(candidate))
        .find(|candidate| candidate.exists())
        .and_then(|shim_path| parse_shim_target(&shim_path).ok()))
}

fn shim_candidates(base_name: &str, extension: Option<&str>) -> Vec<String> {
    match extension {
        Some("exe") => vec![format!("{base_name}.exe"), format!("{base_name}.shim")],
        Some("cmd") => vec![format!("{base_name}.cmd")],
        Some("bat") => vec![format!("{base_name}.bat")],
        Some("ps1") => vec![format!("{base_name}.ps1")],
        Some("shim") => vec![format!("{base_name}.shim")],
        Some(other) => vec![format!("{base_name}.{other}")],
        None => vec![
            format!("{base_name}.ps1"),
            format!("{base_name}.exe"),
            format!("{base_name}.cmd"),
            format!("{base_name}.bat"),
            format!("{base_name}.shim"),
        ],
    }
}

fn resolve_command_from_path(command: &str) -> anyhow::Result<Option<Utf8PathBuf>> {
    let command_path = Utf8Path::new(command);
    let search_path = env::var_os("PATH").unwrap_or_default();
    let has_extension = command_path.extension().is_some();
    let candidates = command_search_candidates(command, has_extension);

    for directory in env::split_paths(&search_path) {
        let directory = Utf8PathBuf::from_path_buf(directory)
            .map_err(|_| anyhow::anyhow!("PATH contains a non-UTF-8 directory"))?;
        for candidate in &candidates {
            let path = directory.join(candidate);
            if path.is_file() {
                return Ok(Some(path));
            }
        }
    }

    Ok(None)
}

fn command_search_candidates(command: &str, has_extension: bool) -> Vec<String> {
    if has_extension {
        return vec![command.to_owned()];
    }

    let mut extensions = vec![String::from(".ps1")];
    let path_extensions =
        env::var("PATHEXT").unwrap_or_else(|_| String::from(".COM;.EXE;.BAT;.CMD"));
    for extension in path_extensions
        .split(';')
        .filter(|extension| !extension.is_empty())
    {
        let normalized = extension.to_ascii_lowercase();
        if !extensions
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&normalized))
        {
            extensions.push(normalized);
        }
    }

    extensions
        .into_iter()
        .map(|extension| format!("{command}{extension}"))
        .collect()
}

fn resolve_command_path(
    config: &RuntimeConfig,
    command_path: &Utf8Path,
) -> anyhow::Result<Option<Utf8PathBuf>> {
    let local_shims = config.paths().shims();
    let global_shims = config.global_paths().shims();

    if path_starts_with_ignore_case(command_path, &local_shims)
        || path_starts_with_ignore_case(command_path, &global_shims)
    {
        return Ok(parse_shim_target(command_path).ok());
    }

    Ok(is_application_path(command_path).then(|| command_path.to_path_buf()))
}

fn path_starts_with_ignore_case(path: &Utf8Path, prefix: &Utf8Path) -> bool {
    let path = path.as_str().replace('/', "\\").to_ascii_lowercase();
    let prefix = prefix.as_str().replace('/', "\\").to_ascii_lowercase();
    path == prefix || path.starts_with(&(prefix + "\\"))
}

fn is_application_path(path: &Utf8Path) -> bool {
    matches!(
        path.extension(),
        Some(extension)
            if extension.eq_ignore_ascii_case("exe")
                || extension.eq_ignore_ascii_case("com")
                || extension.eq_ignore_ascii_case("cmd")
                || extension.eq_ignore_ascii_case("bat")
    )
}

fn parse_shim_target(path: &Utf8Path) -> anyhow::Result<Utf8PathBuf> {
    match path.extension() {
        Some("exe") => {
            let shim_metadata = path.with_extension("shim");
            parse_shim_target(&shim_metadata)
        }
        Some("shim") => parse_dot_shim_target(path),
        Some("cmd") | Some("bat") | Some("ps1") => parse_script_shim_target(path),
        _ => Ok(path.to_path_buf()),
    }
}

pub(crate) fn read_shim_target(path: &Utf8Path) -> anyhow::Result<Utf8PathBuf> {
    parse_shim_target(path)
}

fn parse_dot_shim_target(path: &Utf8Path) -> anyhow::Result<Utf8PathBuf> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read shim {}", path))?;
    let target = source
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("path = "))
        .map(|value| value.trim().trim_matches('"'))
        .filter(|value| !value.is_empty())
        .with_context(|| format!("failed to find target in {}", path))?;

    resolve_shim_target(path, target)
}

fn parse_script_shim_target(path: &Utf8Path) -> anyhow::Result<Utf8PathBuf> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read shim {}", path))?;
    let comment_pattern =
        Regex::new(r"(?m)^(?:@rem|#)\s*(.+)$").expect("script shim comment regex should compile");
    if let Some(target) = comment_pattern
        .captures(&source)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().trim())
        .filter(|value| !value.is_empty())
    {
        return resolve_shim_target(path, target);
    }

    let quoted_pattern = Regex::new(r#"['"]([^@&\r\n]+?)['"]"#)
        .expect("script shim quoted target regex should compile");
    quoted_pattern
        .captures_iter(&source)
        .last()
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str())
        .and_then(|value| resolve_shim_target(path, value).ok())
        .with_context(|| format!("failed to find target in {}", path))
}

fn resolve_shim_target(path: &Utf8Path, target: &str) -> anyhow::Result<Utf8PathBuf> {
    let raw_target = Utf8PathBuf::from(target);
    let resolved_target = if raw_target.is_absolute() {
        raw_target
    } else {
        path.parent()
            .unwrap_or_else(|| Utf8Path::new("."))
            .join(raw_target)
    };

    resolved_target
        .is_file()
        .then_some(resolved_target)
        .with_context(|| format!("failed to resolve shim target in {}", path))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::{Utf8Path, Utf8PathBuf};
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{resolve_prefix, resolve_which};

    #[test]
    fn resolves_local_then_global_prefixes() {
        let fixture = Fixture::new();
        fixture.current_dir("local", "git");
        fixture.current_dir("global", "nodejs");

        let config = fixture.config();
        assert_eq!(
            resolve_prefix(&config, "git")
                .expect("prefix should resolve")
                .expect("git should exist"),
            Utf8PathBuf::from(fixture.local_root())
                .join("apps")
                .join("git")
                .join("current")
        );
        assert_eq!(
            resolve_prefix(&config, "nodejs")
                .expect("prefix should resolve")
                .expect("nodejs should exist"),
            Utf8PathBuf::from(fixture.global_root())
                .join("apps")
                .join("nodejs")
                .join("current")
        );
        assert!(
            resolve_prefix(&config, "missing")
                .expect("prefix lookup should succeed")
                .is_none()
        );
    }

    #[test]
    fn resolves_exe_and_script_shims() {
        let fixture = Fixture::new();
        fixture.shim(
            "local",
            "fixturegit.shim",
            &format!(
                "path = \"{}\"\n",
                fixture
                    .local_root()
                    .join("apps")
                    .join("fixturegit")
                    .join("current")
                    .join("cmd")
                    .join("fixturegit.exe")
            ),
        );
        fixture.shim(
            "local",
            "fixturegitignore.cmd",
            &format!(
                "@rem {}\r\n@echo off\r\n",
                fixture
                    .local_root()
                    .join("apps")
                    .join("fixturegitignore")
                    .join("current")
                    .join("fixturegitignore.ps1")
            ),
        );
        fixture.shim(
            "global",
            "fixturescoop.ps1",
            &format!(
                "# {}\r\n",
                fixture
                    .global_root()
                    .join("apps")
                    .join("fixturescoop")
                    .join("current")
                    .join("bin")
                    .join("fixturescoop.ps1")
            ),
        );
        fixture.file(
            "local",
            "apps\\fixturegit\\current\\cmd\\fixturegit.exe",
            &[],
        );
        fixture.file(
            "local",
            "apps\\fixturegitignore\\current\\fixturegitignore.ps1",
            b"Write-Output 'fixture'",
        );
        fixture.file(
            "global",
            "apps\\fixturescoop\\current\\bin\\fixturescoop.ps1",
            b"Write-Output 'fixture'",
        );

        let config = fixture.config();
        assert_eq!(
            resolve_which(&config, "fixturegit")
                .expect("which should resolve")
                .expect("fixture git shim should exist"),
            fixture
                .local_root()
                .join("apps")
                .join("fixturegit")
                .join("current")
                .join("cmd")
                .join("fixturegit.exe")
        );
        assert_eq!(
            resolve_which(&config, "fixturegitignore")
                .expect("which should resolve")
                .expect("fixture gitignore shim should exist"),
            fixture
                .local_root()
                .join("apps")
                .join("fixturegitignore")
                .join("current")
                .join("fixturegitignore.ps1")
        );
        assert_eq!(
            resolve_which(&config, "fixturescoop")
                .expect("which should resolve")
                .expect("fixture scoop shim should exist"),
            fixture
                .global_root()
                .join("apps")
                .join("fixturescoop")
                .join("current")
                .join("bin")
                .join("fixturescoop.ps1")
        );
        assert!(
            resolve_which(&config, "missing")
                .expect("which lookup should succeed")
                .is_none()
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
            fs::create_dir_all(local_root.join("apps")).expect("local apps dir should exist");
            fs::create_dir_all(local_root.join("shims")).expect("local shims dir should exist");
            fs::create_dir_all(global_root.join("apps")).expect("global apps dir should exist");
            fs::create_dir_all(global_root.join("shims")).expect("global shims dir should exist");

            Self {
                _temp: temp,
                local_root,
                global_root,
            }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::new(self.local_root.clone(), self.global_root.clone())
        }

        fn local_root(&self) -> &Utf8Path {
            &self.local_root
        }

        fn global_root(&self) -> &Utf8Path {
            &self.global_root
        }

        fn current_dir(&self, scope: &str, app: &str) {
            let root = self.root(scope);
            fs::create_dir_all(root.join("apps").join(app).join("current"))
                .expect("current dir should exist");
        }

        fn shim(&self, scope: &str, name: &str, content: &str) {
            let root = self.root(scope);
            fs::write(root.join("shims").join(name), content).expect("shim should be written");
            if let Some(base_name) = name.strip_suffix(".shim") {
                fs::write(root.join("shims").join(format!("{base_name}.exe")), [])
                    .expect("shim exe should be written");
            }
        }

        fn file(&self, scope: &str, relative_path: &str, content: &[u8]) {
            let root = self.root(scope);
            let path = root.join(relative_path);
            fs::create_dir_all(path.parent().expect("fixture file should have a parent"))
                .expect("fixture parent should exist");
            fs::write(path, content).expect("fixture file should exist");
        }

        fn root(&self, scope: &str) -> &Utf8PathBuf {
            match scope {
                "local" => &self.local_root,
                "global" => &self.global_root,
                _ => panic!("unknown scope"),
            }
        }
    }
}
