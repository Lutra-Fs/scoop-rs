use std::fs;

use anyhow::{Context, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;
use sysinfo::{ProcessesToUpdate, System};

use crate::{
    RuntimeConfig,
    domain::install_context::{HookType, InstallContext, InstallContextPaths},
    infra::{
        environment::{EnvScope, remove_path, scoop_path_env_var, set_env_var},
        shortcuts::shortcut_root,
    },
};

use super::install::{
    arch_specific_strings, arch_specific_value, current_version, is_admin, is_in_dir,
    load_manifest_json, persist_entries, remove_existing_path, remove_existing_path_if_exists,
    run_manifest_hook, script_value, shim_entries, windows_path,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UninstallOptions {
    pub global: bool,
    pub purge: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallOutcome {
    Uninstalled { app: String, version: String },
    NotInstalled { app: String, hint: Option<String> },
    RunningProcess { app: String, processes: Vec<String> },
}

pub fn uninstall_apps(
    config: &RuntimeConfig,
    apps: &[String],
    options: &UninstallOptions,
) -> anyhow::Result<Vec<UninstallOutcome>> {
    if options.global && !is_admin()? {
        bail!("You need admin rights to uninstall global apps.");
    }
    let confirmed = confirm_installation_status(config, apps, options)?;
    let mut outcomes = Vec::new();
    for (app, global) in confirmed {
        let opts = UninstallOptions {
            global,
            purge: options.purge,
        };
        outcomes.push(uninstall_single_app(config, &app, &opts)?);
    }
    Ok(outcomes)
}

/// Verify each requested app is actually installed in the expected scope.
/// Returns `(app_name, is_global)` pairs for valid apps and
/// `NotInstalled` outcomes for the rest.
fn confirm_installation_status(
    config: &RuntimeConfig,
    apps: &[String],
    options: &UninstallOptions,
) -> anyhow::Result<Vec<(String, bool)>> {
    let mut confirmed = Vec::new();
    for app in apps {
        if app == "scoop" {
            bail!("Uninstalling scoop itself is not supported in scoop-rs.");
        }
        let local_paths = config.paths();
        let global_paths = config.global_paths();
        if options.global {
            if global_paths.app_dir(app).exists() {
                confirmed.push((app.clone(), true));
            } else if local_paths.app_dir(app).exists() {
                bail!(
                    "'{app}' isn't installed globally, but it may be installed locally.\nTry again without the --global (or -g) flag instead."
                );
            } else {
                bail!("'{app}' isn't installed.");
            }
        } else if local_paths.app_dir(app).exists() {
            confirmed.push((app.clone(), false));
        } else if global_paths.app_dir(app).exists() {
            bail!(
                "'{app}' isn't installed locally, but it may be installed globally.\nTry again with the --global (or -g) flag instead."
            );
        } else {
            bail!("'{app}' isn't installed.");
        }
    }
    Ok(confirmed)
}

fn uninstall_single_app(
    config: &RuntimeConfig,
    app_name: &str,
    options: &UninstallOptions,
) -> anyhow::Result<UninstallOutcome> {
    let paths = if options.global {
        config.global_paths()
    } else {
        config.paths()
    };
    let app_dir = paths.app_dir(app_name);
    let version = match current_version(paths.root(), app_name)? {
        Some(v) => v,
        None => {
            // Failed install state – clean up and report
            let _ = fs::remove_dir_all(&app_dir);
            bail!("'{app_name}' isn't installed correctly.");
        }
    };

    let version_dir = paths.version_dir(app_name, &version);
    let persist_dir = paths.persist().join(app_name);

    let manifest = load_installed_manifest(&version_dir);
    let install_info = load_install_info(&version_dir);
    let architecture = install_info
        .as_ref()
        .and_then(|info| info.get("architecture").and_then(Value::as_str))
        .unwrap_or("64bit")
        .to_owned();

    // Pre-uninstall hook
    if let Some(ref manifest) = manifest {
        let ctx = make_context(
            app_name,
            &version,
            &architecture,
            options.global,
            &version_dir,
            &persist_dir,
            manifest,
        );
        run_manifest_hook(HookType::PreUninstall, manifest, &architecture, &ctx)?;
    }

    let running_processes = test_running_process(&app_dir)?;
    if !running_processes.is_empty() {
        return Ok(UninstallOutcome::RunningProcess {
            app: app_name.to_owned(),
            processes: running_processes,
        });
    }

    // Run uninstaller executable / script
    if let Some(ref manifest) = manifest {
        run_uninstaller(manifest, &architecture, &version_dir, app_name)?;
    }

    // Remove shims
    if let Some(ref manifest) = manifest {
        remove_shims(paths.shims(), manifest, &architecture, app_name)?;
    }

    // Remove shortcuts
    if let Some(ref manifest) = manifest {
        remove_shortcuts(manifest, &architecture, options.global)?;
    }

    // Unlink `current` → obtain reference dir
    let ref_dir = unlink_current_dir(
        &app_dir,
        &version_dir,
        config.settings().no_junction.unwrap_or(false),
    )?;

    // Uninstall PowerShell module
    if let Some(ref manifest) = manifest {
        uninstall_psmodule(manifest, &ref_dir, paths)?;
    }

    // Undo env_add_path
    if let Some(ref manifest) = manifest {
        env_remove_paths(config, manifest, &ref_dir, options.global, &architecture)?;
    }

    // Undo env_set
    if let Some(ref manifest) = manifest {
        env_remove_vars(manifest, options.global, &architecture)?;
    }

    // Unlink persist data in version dir, then remove version dir
    if let Some(ref manifest) = manifest {
        unlink_persist_data(manifest, &version_dir)?;
    }
    remove_dir_force(&version_dir)?;

    // Post-uninstall hook (fire even though version dir is gone;
    // upstream does this and some manifests rely on $dir still being set)
    if let Some(ref manifest) = manifest {
        let ctx = make_context(
            app_name,
            &version,
            &architecture,
            options.global,
            &version_dir,
            &persist_dir,
            manifest,
        );
        // Ignore errors from post-uninstall since the dir is already removed
        let _ = run_manifest_hook(HookType::PostUninstall, manifest, &architecture, &ctx);
    }

    // Remove older versions
    remove_old_versions(&app_dir, manifest.as_ref())?;

    // Remove current link if still present
    let current_dir = app_dir.join("current");
    remove_existing_path_if_exists(&current_dir)?;

    // Remove app dir if now empty
    if app_dir.exists() && is_dir_empty(&app_dir)? {
        let _ = fs::remove_dir_all(&app_dir);
    }

    // Purge persist data
    if options.purge && persist_dir.exists() {
        fs::remove_dir_all(&persist_dir)
            .with_context(|| format!("Couldn't remove '{}'", persist_dir))?;
    }

    Ok(UninstallOutcome::Uninstalled {
        app: app_name.to_owned(),
        version,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_context(
    app: &str,
    version: &str,
    architecture: &str,
    global: bool,
    version_dir: &Utf8Path,
    persist_dir: &Utf8Path,
    manifest: &Value,
) -> InstallContext {
    InstallContext::new(
        app.to_owned(),
        version.to_owned(),
        architecture.to_owned(),
        global,
        InstallContextPaths {
            dir: version_dir.to_owned(),
            original_dir: version_dir.to_owned(),
            persist_dir: persist_dir.to_owned(),
        },
        manifest.clone(),
    )
}

fn load_installed_manifest(version_dir: &Utf8Path) -> Option<Value> {
    let path = version_dir.join("manifest.json");
    load_manifest_json(&path).ok()
}

fn load_install_info(version_dir: &Utf8Path) -> Option<Value> {
    let path = version_dir.join("install.json");
    load_manifest_json(&path).ok()
}

pub(crate) fn test_running_process(app_dir: &Utf8Path) -> anyhow::Result<Vec<String>> {
    if let Some(raw) = std::env::var_os("SCOOP_RS_RUNNING_PROCESS_PATHS") {
        let raw = raw.to_string_lossy();
        return Ok(filter_running_processes(app_dir, raw.as_ref()));
    }

    #[cfg(windows)]
    {
        native_running_processes(app_dir)
    }
    #[cfg(not(windows))]
    {
        let _ = app_dir;
        Ok(Vec::new())
    }
}

fn filter_running_processes(app_dir: &Utf8Path, raw: &str) -> Vec<String> {
    let root = windows_path(app_dir);
    let root = root.trim_end_matches('\\').to_ascii_lowercase();
    raw.split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| {
            let value = value.to_ascii_lowercase();
            value == root || value.starts_with(&(root.clone() + "\\"))
        })
        .map(str::to_owned)
        .collect()
}

#[cfg(windows)]
fn native_running_processes(app_dir: &Utf8Path) -> anyhow::Result<Vec<String>> {
    let root = windows_path(app_dir);
    let root = root.trim_end_matches('\\').to_ascii_lowercase();
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let mut matches = Vec::new();
    for process in system.processes().values() {
        let Some(path) = process.exe() else {
            continue;
        };
        let path = path.to_string_lossy().into_owned();
        if path_matches_root(&path, &root) {
            matches.push(path);
        }
    }
    matches.sort();
    matches.dedup();
    Ok(matches)
}

#[cfg(windows)]
fn path_matches_root(path: &str, root: &str) -> bool {
    let path = path.trim().trim_end_matches('\\').to_ascii_lowercase();
    path == root || path.starts_with(&(root.to_owned() + "\\"))
}

pub(crate) fn run_uninstaller(
    manifest: &Value,
    architecture: &str,
    version_dir: &Utf8Path,
    app_name: &str,
) -> anyhow::Result<()> {
    let Some(uninstaller) = arch_specific_value(manifest, architecture, "uninstaller") else {
        return Ok(());
    };

    if let Some(script) = uninstaller
        .get("script")
        .map(script_value)
        .filter(|s| !s.trim().is_empty())
    {
        let ctx = InstallContext::new(
            app_name.to_owned(),
            String::new(),
            architecture.to_owned(),
            false,
            InstallContextPaths {
                dir: version_dir.to_owned(),
                original_dir: version_dir.to_owned(),
                persist_dir: Utf8PathBuf::new(),
            },
            manifest.clone(),
        );
        crate::infra::powershell::run_install_hook(HookType::Uninstaller, &script, &ctx)?;
    }

    let file = uninstaller.get("file").and_then(Value::as_str);
    let args = uninstaller
        .get("args")
        .map(|value| uninstaller_argument_values(value, version_dir))
        .unwrap_or_default();
    if file.is_none() && args.is_empty() {
        return Ok(());
    }

    let file_name = file.context("uninstaller filename could not be determined")?;
    let program = version_dir.join(file_name);
    if !is_in_dir(version_dir, &program) {
        bail!(
            "Error in manifest: Uninstaller {} is outside the app directory.",
            program
        );
    }
    if !program.exists() {
        bail!("Uninstaller {} is missing.", program);
    }

    let success = if program
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ps1"))
    {
        std::process::Command::new("pwsh")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                program.as_str(),
            ])
            .args(&args)
            .status()
            .with_context(|| format!("failed to run uninstaller {}", program))?
            .success()
    } else {
        std::process::Command::new(program.as_str())
            .args(&args)
            .status()
            .with_context(|| format!("failed to run uninstaller {}", program))?
            .success()
    };
    if !success {
        bail!("Uninstallation aborted.");
    }
    Ok(())
}

/// Substitute `$dir`, `$global`, `$version` in uninstaller args –
/// upstream uses a smaller set of substitutions for uninstaller args.
fn uninstaller_argument_values(value: &Value, dir: &Utf8Path) -> Vec<String> {
    let dir_windows = windows_path(dir);
    let expand = |s: &str| s.replace("$dir", &dir_windows);
    match value {
        Value::String(s) => vec![expand(s)],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(expand)
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn remove_shims(
    shims_dir: Utf8PathBuf,
    manifest: &Value,
    architecture: &str,
    app_name: &str,
) -> anyhow::Result<()> {
    let Some(bin) = arch_specific_value(manifest, architecture, "bin") else {
        return Ok(());
    };
    for shim in shim_entries(bin) {
        remove_single_shim(&shims_dir, &shim.name, app_name)?;
    }
    Ok(())
}

/// Remove shim files for a single shim name, handling the alternate-backup
/// naming that upstream uses when multiple apps register the same shim.
fn remove_single_shim(shims_dir: &Utf8Path, name: &str, app_name: &str) -> anyhow::Result<()> {
    let lower = name.to_ascii_lowercase();
    for suffix in ["", ".shim", ".cmd", ".ps1"] {
        let shim_path = shims_dir.join(format!("{lower}{suffix}"));
        let alt_path = shims_dir.join(format!("{lower}{suffix}.{app_name}"));

        if alt_path.is_file() {
            let _ = fs::remove_file(&alt_path);
        } else if shim_path.is_file() {
            let _ = fs::remove_file(&shim_path);
            // If there are backed-up shims from other apps, restore the most
            // recent one (upstream sorts by LastWriteTimeUtc).
            if suffix == ".shim" {
                restore_backup_shim(shims_dir, &lower)?;
            }
        }
    }
    // Also remove the .exe shim stub if .shim was removed and no backup restored
    let exe_path = shims_dir.join(format!("{lower}.exe"));
    let shim_file = shims_dir.join(format!("{lower}.shim"));
    if exe_path.is_file() && !shim_file.is_file() {
        let _ = fs::remove_file(&exe_path);
    }
    Ok(())
}

fn restore_backup_shim(shims_dir: &Utf8Path, base_name: &str) -> anyhow::Result<()> {
    let pattern = format!("{base_name}.");
    let mut candidates: Vec<(std::time::SystemTime, Utf8PathBuf)> = Vec::new();
    if let Ok(entries) = fs::read_dir(shims_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.starts_with(&pattern)
                && !name.ends_with(".shim")
                && !name.ends_with(".cmd")
                && !name.ends_with(".ps1")
                && !name.ends_with(".exe")
                && let Ok(meta) = entry.metadata()
                && let Ok(modified) = meta.modified()
                && let Ok(path) = Utf8PathBuf::from_path_buf(entry.path())
            {
                candidates.push((modified, path));
            }
        }
    }
    if let Some((_, newest)) = candidates.into_iter().max_by_key(|(t, _)| *t) {
        let target = shims_dir.join(base_name);
        let _ = fs::rename(&newest, &target);
    }
    Ok(())
}

fn remove_shortcuts(manifest: &Value, architecture: &str, global: bool) -> anyhow::Result<()> {
    let Some(shortcuts) = arch_specific_value(manifest, architecture, "shortcuts") else {
        return Ok(());
    };
    let Value::Array(shortcuts) = shortcuts else {
        return Ok(());
    };
    let root = shortcut_root(global)?;
    for shortcut in shortcuts {
        let Value::Array(parts) = shortcut else {
            continue;
        };
        let Some(name) = parts.get(1).and_then(Value::as_str) else {
            continue;
        };
        let shortcut_path = root.join(format!("{name}.lnk"));
        if shortcut_path.is_file() {
            let _ = fs::remove_file(&shortcut_path);
        }
    }
    Ok(())
}

pub(crate) fn unlink_current_dir(
    app_dir: &Utf8Path,
    version_dir: &Utf8Path,
    no_junction: bool,
) -> anyhow::Result<Utf8PathBuf> {
    if no_junction {
        return Ok(version_dir.to_owned());
    }
    let current_dir = app_dir.join("current");
    if current_dir.exists() {
        remove_existing_path(&current_dir)?;
        return Ok(current_dir);
    }
    Ok(version_dir.to_owned())
}

pub(crate) fn uninstall_psmodule(
    manifest: &Value,
    _ref_dir: &Utf8Path,
    paths: &crate::domain::paths::ScoopPaths,
) -> anyhow::Result<()> {
    let Some(psmodule) = manifest.get("psmodule").and_then(Value::as_object) else {
        return Ok(());
    };
    let Some(module_name) = psmodule.get("name").and_then(Value::as_str) else {
        return Ok(());
    };
    let modules_dir = paths.root().join("modules");
    let link_path = modules_dir.join(module_name);
    if link_path.exists() {
        remove_existing_path_if_exists(&link_path)?;
    }
    Ok(())
}

pub(crate) fn env_remove_paths(
    config: &RuntimeConfig,
    manifest: &Value,
    ref_dir: &Utf8Path,
    global: bool,
    architecture: &str,
) -> anyhow::Result<()> {
    let additions = arch_specific_strings(manifest, architecture, "env_add_path");
    if additions.is_empty() {
        return Ok(());
    }

    let mut paths = Vec::new();
    for addition in additions {
        let candidate = ref_dir.join(&addition);
        if is_in_dir(ref_dir, &candidate) {
            paths.push(windows_path(&candidate));
        }
    }
    if paths.is_empty() {
        return Ok(());
    }

    let settings = config.settings();
    let target_env_var = scoop_path_env_var(&settings);
    let scope = if global {
        EnvScope::System
    } else {
        EnvScope::User
    };
    // Remove from both PATH and the isolated env var
    let _ = remove_path(scope, "PATH", &paths);
    if target_env_var != "PATH" {
        let _ = remove_path(scope, &target_env_var, &paths);
    }
    Ok(())
}

pub(crate) fn env_remove_vars(
    manifest: &Value,
    global: bool,
    architecture: &str,
) -> anyhow::Result<()> {
    let Some(env_set) = arch_specific_value(manifest, architecture, "env_set") else {
        return Ok(());
    };
    let Some(object) = env_set.as_object() else {
        return Ok(());
    };
    let scope = if global {
        EnvScope::System
    } else {
        EnvScope::User
    };
    for name in object.keys() {
        set_env_var(scope, name, None)?;
    }
    Ok(())
}

pub(crate) fn unlink_persist_data(manifest: &Value, dir: &Utf8Path) -> anyhow::Result<()> {
    let Some(persist) = manifest.get("persist") else {
        return Ok(());
    };
    for (source_rel, _) in persist_entries(persist) {
        let source_rel = source_rel.trim_end_matches(['/', '\\']);
        if source_rel.is_empty() {
            continue;
        }
        let source = dir.join(source_rel);
        if !source.exists() {
            continue;
        }
        // Check if it's a link (junction or hard link)
        let Ok(meta) = fs::symlink_metadata(&source) else {
            continue;
        };
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || meta.is_file() {
                remove_existing_path_if_exists(&source)?;
            }
        }
        #[cfg(not(windows))]
        {
            if meta.file_type().is_symlink() || meta.is_file() {
                remove_existing_path_if_exists(&source)?;
            }
        }
    }
    Ok(())
}

fn remove_old_versions(app_dir: &Utf8Path, manifest: Option<&Value>) -> anyhow::Result<()> {
    let Ok(entries) = fs::read_dir(app_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "current" {
            continue;
        }
        let path = Utf8PathBuf::try_from(entry.path())
            .unwrap_or_else(|_| Utf8PathBuf::from(entry.path().to_string_lossy().as_ref()));
        if !path.is_dir() {
            continue;
        }
        // Unlink persist data in old version dirs too
        if let Some(manifest) = manifest {
            let _ = unlink_persist_data(manifest, &path);
        }
        let _ = fs::remove_dir_all(&path);
    }
    Ok(())
}

fn remove_dir_force(path: &Utf8Path) -> anyhow::Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .with_context(|| format!("Couldn't remove '{}'; it may be in use.", path))?;
    }
    Ok(())
}

fn is_dir_empty(path: &Utf8Path) -> anyhow::Result<bool> {
    Ok(fs::read_dir(path)
        .with_context(|| format!("failed to read directory {}", path))?
        .next()
        .is_none())
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use camino::Utf8PathBuf;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;

    use crate::RuntimeConfig;
    use crate::app::install::{InstallOptions, InstallOutcome, install_app};

    use super::{UninstallOptions, UninstallOutcome, uninstall_apps};

    /// Install a demo app from a zip payload, then uninstall it and assert
    /// that all side effects are cleaned up.
    #[test]
    fn uninstalls_previously_installed_app_completely() {
        let fixture = Fixture::new();
        let archive = fixture.write_zip(
            "payloads\\demo.zip",
            &[("demo.exe", b"demo binary"), ("README.txt", b"hello")],
        );
        let hash = crate::infra::hash::sha256_file(&archive).expect("hash should compute");
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            &format!(
                r#"{{
                    "version":"1.2.3",
                    "url":"{}",
                    "hash":"{}",
                    "bin":"demo.exe"
                }}"#,
                escape_json_path(&archive),
                hash
            ),
        );

        // Install first
        let outcome = install_app(&fixture.config(), "demo", &InstallOptions::default())
            .expect("install should succeed");
        assert!(matches!(outcome, InstallOutcome::Installed(_)));

        // Verify install side effects exist
        assert!(fixture.local_root.join("apps/demo/current").exists());
        assert!(fixture.local_root.join("shims/demo.cmd").is_file());

        // Uninstall
        let outcomes = uninstall_apps(
            &fixture.config(),
            &[String::from("demo")],
            &UninstallOptions::default(),
        )
        .expect("uninstall should succeed");

        assert_eq!(outcomes.len(), 1);
        let UninstallOutcome::Uninstalled {
            ref app,
            ref version,
        } = outcomes[0]
        else {
            panic!("expected Uninstalled outcome, got {:?}", outcomes[0]);
        };
        assert_eq!(app, "demo");
        assert_eq!(version, "1.2.3");

        // Verify side effects are cleaned up
        assert!(!fixture.local_root.join("apps/demo").exists());
        assert!(!fixture.local_root.join("shims/demo.cmd").is_file());
    }

    #[test]
    fn uninstall_not_installed_app_reports_error() {
        let fixture = Fixture::new();
        let error = uninstall_apps(
            &fixture.config(),
            &[String::from("missing")],
            &UninstallOptions::default(),
        )
        .expect_err("should fail for missing app");
        assert!(error.to_string().contains("isn't installed"));
    }

    #[test]
    fn uninstall_with_purge_removes_persist_dir() {
        let fixture = Fixture::new();
        let archive = fixture.write_zip(
            "payloads\\demo.zip",
            &[("demo.exe", b"demo"), ("data.txt", b"persist me")],
        );
        let hash = crate::infra::hash::sha256_file(&archive).expect("hash should compute");
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            &format!(
                r#"{{
                    "version":"1.0.0",
                    "url":"{}",
                    "hash":"{}",
                    "bin":"demo.exe",
                    "persist":"data.txt"
                }}"#,
                escape_json_path(&archive),
                hash,
            ),
        );

        install_app(&fixture.config(), "demo", &InstallOptions::default())
            .expect("install should succeed");
        assert!(fixture.local_root.join("persist/demo").exists());

        // Uninstall without purge – persist should remain
        uninstall_apps(
            &fixture.config(),
            &[String::from("demo")],
            &UninstallOptions::default(),
        )
        .expect("uninstall should succeed");
        assert!(fixture.local_root.join("persist/demo").exists());

        // Re-install and uninstall with purge
        install_app(&fixture.config(), "demo", &InstallOptions::default())
            .expect("reinstall should succeed");
        uninstall_apps(
            &fixture.config(),
            &[String::from("demo")],
            &UninstallOptions {
                global: false,
                purge: true,
            },
        )
        .expect("uninstall with purge should succeed");
        assert!(!fixture.local_root.join("persist/demo").exists());
    }

    // ------ Fixture helpers (same pattern as install tests) ------

    struct Fixture {
        _temp: TempDir,
        local_root: Utf8PathBuf,
        global_root: Utf8PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir");
            let base = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            let local_root = base.join("local");
            let global_root = base.join("global");
            for dir in [
                local_root.join("buckets"),
                local_root.join("apps"),
                local_root.join("shims"),
                local_root.join("cache"),
                global_root.join("apps"),
                global_root.join("shims"),
            ] {
                fs::create_dir_all(&dir).expect("fixture dir");
            }
            Self {
                _temp: temp,
                local_root,
                global_root,
            }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::with_cache(
                self.local_root.clone(),
                self.global_root.clone(),
                self.local_root.join("cache"),
            )
        }

        fn write(&self, scope: &str, relative_path: &str, content: &str) {
            let root = match scope {
                "local" => &self.local_root,
                "global" => &self.global_root,
                _ => panic!("unknown scope"),
            };
            let path = root.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("parent dir");
            }
            fs::write(&path, content).expect("write fixture file");
        }

        fn write_zip(&self, filename: &str, files: &[(&str, &[u8])]) -> Utf8PathBuf {
            let path = self.local_root.join(filename);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("zip parent dir");
            }
            let file = fs::File::create(&path).expect("create zip");
            let mut writer = zip::ZipWriter::new(file);
            for (name, content) in files {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start zip entry");
                writer.write_all(content).expect("write zip content");
            }
            writer.finish().expect("finish zip");
            path
        }
    }

    fn escape_json_path(path: &Utf8PathBuf) -> String {
        path.as_str().replace('\\', "\\\\")
    }
}
