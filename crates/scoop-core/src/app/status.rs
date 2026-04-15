use std::{
    fs,
    time::{Instant, SystemTime},
};

use anyhow::Context;
use camino::Utf8Path;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    RuntimeConfig,
    domain::version::compare_versions,
    infra::buckets::{bucket_root, local_bucket_names},
    infra::git::{git_available, test_update_status},
    infra::profiling,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRow {
    pub name: String,
    pub installed_version: String,
    pub latest_version: String,
    pub missing_dependencies: Vec<String>,
    pub info: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub scoop_out_of_date: bool,
    pub bucket_out_of_date: bool,
    pub network_failure: bool,
    pub rows: Vec<StatusRow>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct InstallMetadata {
    #[serde(default)]
    bucket: Option<String>,
    #[serde(default)]
    hold: Option<bool>,
}

pub fn collect_status(config: &RuntimeConfig, local_only: bool) -> anyhow::Result<StatusReport> {
    let _profile = profiling::scope("status total");
    let mut network_failure = false;
    let no_remotes = local_only || !git_available();
    let scoop_out_of_date = if no_remotes {
        false
    } else {
        let started = Instant::now();
        let status = test_update_status(&config.paths().current_dir("scoop"));
        profiling::emit_duration("status scoop_freshness", started.elapsed());
        network_failure |= status.network_failure;
        status.needs_update
    };
    let bucket_out_of_date = if no_remotes || scoop_out_of_date {
        false
    } else {
        let started = Instant::now();
        let mut needs_update = false;
        for bucket in local_bucket_names(config)? {
            let status = test_update_status(&bucket_root(config, Some(&bucket)));
            network_failure |= status.network_failure;
            if status.needs_update {
                needs_update = true;
                break;
            }
        }
        profiling::emit_duration("status bucket_freshness", started.elapsed());
        needs_update
    };

    let mut rows = Vec::new();
    let local_started = Instant::now();
    rows.extend(scan_scope(config.paths().root(), config)?);
    profiling::emit_duration("status scan_local_scope", local_started.elapsed());
    let global_started = Instant::now();
    rows.extend(scan_scope(config.global_paths().root(), config)?);
    profiling::emit_duration("status scan_global_scope", global_started.elapsed());

    Ok(StatusReport {
        scoop_out_of_date,
        bucket_out_of_date,
        network_failure,
        rows,
    })
}

fn scan_scope(root: &Utf8Path, config: &RuntimeConfig) -> anyhow::Result<Vec<StatusRow>> {
    let apps_dir = root.join("apps");
    if !apps_dir.exists() {
        return Ok(Vec::new());
    }

    let mut app_names = fs::read_dir(&apps_dir)
        .with_context(|| format!("failed to read installed apps from {}", apps_dir))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .and_then(|_| entry.file_name().into_string().ok())
        })
        .filter(|name| name != "scoop")
        .collect::<Vec<_>>();
    app_names.sort_unstable();

    let mut rows = Vec::new();
    for app in app_names {
        if let Some(row) = status_for_app(root, config, &app)? {
            rows.push(row);
        }
    }
    Ok(rows)
}

fn status_for_app(
    root: &Utf8Path,
    config: &RuntimeConfig,
    app: &str,
) -> anyhow::Result<Option<StatusRow>> {
    let app_dir = root.join("apps").join(app);
    let current_dir = app_dir.join("current");
    let installed_versions = installed_versions(root, app)?;
    let installed = !installed_versions.is_empty();
    let has_current = current_dir.exists();
    let failed = app_dir.exists() && !(has_current && installed);
    let Some(installed_version) =
        current_version(root, app)?.or_else(|| installed_versions.last().cloned())
    else {
        return Ok(failed.then_some(StatusRow {
            name: app.to_owned(),
            installed_version: String::new(),
            latest_version: String::new(),
            missing_dependencies: Vec::new(),
            info: vec![String::from("Install failed")],
        }));
    };

    let install_info =
        read_install_metadata(&app_dir.join(&installed_version).join("install.json"))
            .unwrap_or_default();
    let bucket = install_info.bucket.as_deref();
    let manifest_path = bucket_manifest_path(config, bucket, app);
    let manifest = manifest_path.as_ref().and_then(|path| load_json(path).ok());
    let removed = bucket.is_some() && manifest.is_none();
    let deprecated = bucket
        .map(|bucket| {
            config
                .paths()
                .buckets()
                .join(bucket)
                .join("deprecated")
                .join(format!("{app}.json"))
                .exists()
        })
        .unwrap_or(false);
    let latest_version = manifest
        .as_ref()
        .and_then(|manifest| manifest.get("version").and_then(Value::as_str))
        .unwrap_or(&installed_version)
        .to_owned();
    let outdated = manifest
        .as_ref()
        .map(|_| compare_versions(&installed_version, &latest_version).is_lt())
        .unwrap_or(false);
    let missing_dependencies = manifest
        .as_ref()
        .map(|manifest| {
            dependencies(manifest)
                .into_iter()
                .filter(|dependency| !is_installed_anywhere(config, dependency))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut info = Vec::new();
    if failed {
        info.push(String::from("Install failed"));
    }
    if install_info.hold.unwrap_or(false) {
        info.push(String::from("Held package"));
    }
    if deprecated {
        info.push(String::from("Deprecated"));
    }
    if removed {
        info.push(String::from("Manifest removed"));
    }

    if !outdated && info.is_empty() && missing_dependencies.is_empty() {
        return Ok(None);
    }

    Ok(Some(StatusRow {
        name: app.to_owned(),
        installed_version,
        latest_version: if outdated {
            latest_version
        } else {
            String::new()
        },
        missing_dependencies,
        info,
    }))
}

fn bucket_manifest_path(
    config: &RuntimeConfig,
    bucket: Option<&str>,
    app: &str,
) -> Option<camino::Utf8PathBuf> {
    let bucket = bucket?;
    let root = bucket_root(config, Some(bucket));
    let primary = root.join("bucket").join(format!("{app}.json"));
    let deprecated = root.join("deprecated").join(format!("{app}.json"));
    if primary.exists() {
        Some(primary)
    } else if deprecated.exists() {
        Some(deprecated)
    } else {
        None
    }
}

fn installed_versions(root: &Utf8Path, app: &str) -> anyhow::Result<Vec<String>> {
    let app_dir = root.join("apps").join(app);
    if !app_dir.exists() {
        return Ok(Vec::new());
    }

    let mut versions = fs::read_dir(&app_dir)
        .with_context(|| format!("failed to read installed versions from {}", app_dir))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .and_then(|_| entry.file_name().into_string().ok())
        })
        .filter(|name| name != "current" && !name.starts_with('_'))
        .filter_map(|name| {
            let install_json = app_dir.join(&name).join("install.json");
            install_json
                .is_file()
                .then(|| modified_time(&install_json).ok().map(|time| (time, name)))
                .flatten()
        })
        .collect::<Vec<_>>();
    versions.sort_by_key(|(updated_at, _)| *updated_at);

    Ok(versions.into_iter().map(|(_, version)| version).collect())
}

fn current_version(root: &Utf8Path, app: &str) -> anyhow::Result<Option<String>> {
    let current_manifest = root
        .join("apps")
        .join(app)
        .join("current")
        .join("manifest.json");
    if current_manifest.is_file() {
        return Ok(load_json(&current_manifest)
            .ok()
            .as_ref()
            .and_then(|manifest| {
                manifest
                    .get("version")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }));
    }
    Ok(None)
}

fn read_install_metadata(path: &Utf8Path) -> anyhow::Result<InstallMetadata> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read install metadata {}", path))?;
    serde_json::from_str(&source)
        .with_context(|| format!("failed to parse install metadata {}", path))
}

fn load_json(path: &Utf8Path) -> anyhow::Result<Value> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read JSON file {}", path))?;
    serde_json::from_str(&source).with_context(|| format!("failed to parse JSON file {}", path))
}

fn modified_time(path: &Utf8Path) -> anyhow::Result<SystemTime> {
    fs::metadata(path)
        .with_context(|| format!("failed to read metadata from {}", path))?
        .modified()
        .with_context(|| format!("failed to read modified time from {}", path))
}

fn dependencies(manifest: &Value) -> Vec<String> {
    match manifest.get("depends") {
        Some(Value::String(value)) => vec![dependency_name(value)],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(dependency_name)
            .collect(),
        _ => Vec::new(),
    }
}

fn dependency_name(value: &str) -> String {
    value
        .split(['/', '\\'])
        .next_back()
        .unwrap_or(value)
        .to_owned()
}

fn is_installed_anywhere(config: &RuntimeConfig, app: &str) -> bool {
    config.paths().app_dir(app).exists() || config.global_paths().app_dir(app).exists()
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::collect_status;

    #[test]
    fn collects_removed_and_missing_dependency_rows() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            r#"{"version":"1.2.3","depends":["other"]}"#,
        );
        fixture.write(
            "local",
            "apps\\demo\\current\\manifest.json",
            r#"{"version":"1.2.2"}"#,
        );
        fixture.write(
            "local",
            "apps\\demo\\1.2.2\\install.json",
            r#"{"bucket":"main","hold":true}"#,
        );
        fixture.write(
            "local",
            "apps\\removedapp\\current\\manifest.json",
            r#"{"version":"1.0.0"}"#,
        );
        fixture.write(
            "local",
            "apps\\removedapp\\1.0.0\\install.json",
            r#"{"bucket":"main"}"#,
        );

        let report = collect_status(&fixture.config(), true).expect("status should succeed");
        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.rows[0].name, "demo");
        assert_eq!(report.rows[0].latest_version, "1.2.3");
        assert_eq!(
            report.rows[0].missing_dependencies,
            vec![String::from("other")]
        );
        assert_eq!(report.rows[0].info, vec![String::from("Held package")]);
        assert_eq!(report.rows[1].name, "removedapp");
        assert_eq!(report.rows[1].info, vec![String::from("Manifest removed")]);
    }

    #[test]
    fn ignores_invalid_current_manifest_and_uses_installed_version_fallback() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            r#"{"version":"1.2.3"}"#,
        );
        fixture.write(
            "local",
            "apps\\demo\\current\\manifest.json",
            "{ invalid json",
        );
        fixture.write(
            "local",
            "apps\\demo\\1.0.0\\install.json",
            r#"{"bucket":"main"}"#,
        );

        let report = collect_status(&fixture.config(), true).expect("status should succeed");
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].name, "demo");
        assert_eq!(report.rows[0].installed_version, "1.0.0");
        assert_eq!(report.rows[0].latest_version, "1.2.3");
    }

    #[test]
    fn detects_remote_bucket_updates_via_git_fetch() {
        let fixture = Fixture::new();
        fixture.init_git_repo("local", "apps\\scoop\\current");
        fixture.init_git_repo("local", "buckets\\main");

        let report = collect_status(&fixture.config(), false).expect("status should succeed");
        assert!(!report.scoop_out_of_date);
        assert!(!report.bucket_out_of_date);

        fixture.push_git_update(
            "local",
            "buckets\\main",
            "bucket\\demo.json",
            r#"{"version":"2.0.0"}"#,
        );

        let report = collect_status(&fixture.config(), false).expect("status should succeed");
        assert!(!report.scoop_out_of_date);
        assert!(report.bucket_out_of_date);
    }

    #[test]
    fn reports_network_failures_from_remote_git_fetch() {
        let fixture = Fixture::new();
        let scoop_current = fixture
            .local_root
            .join("apps")
            .join("scoop")
            .join("current");
        fs::create_dir_all(&scoop_current).expect("scoop current dir should exist");
        run_git(&fixture.local_root, &["init", scoop_current.as_str()]);
        run_git(
            &fixture.local_root,
            &["-C", scoop_current.as_str(), "config", "user.name", "Codex"],
        );
        run_git(
            &fixture.local_root,
            &[
                "-C",
                scoop_current.as_str(),
                "config",
                "user.email",
                "codex@example.invalid",
            ],
        );
        fs::write(scoop_current.join("README.md"), "seed")
            .expect("scoop repo seed file should be written");
        run_git(
            &fixture.local_root,
            &["-C", scoop_current.as_str(), "add", "."],
        );
        run_git(
            &fixture.local_root,
            &[
                "-C",
                scoop_current.as_str(),
                "commit",
                "-m",
                "seed",
                "--no-gpg-sign",
            ],
        );

        let report = collect_status(&fixture.config(), false).expect("status should succeed");
        assert!(report.network_failure);
        assert!(!report.scoop_out_of_date);
        assert!(!report.bucket_out_of_date);
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
            fs::create_dir_all(&local_root).expect("local root should exist");
            fs::create_dir_all(&global_root).expect("global root should exist");

            Self {
                _temp: temp,
                local_root,
                global_root,
            }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::new(self.local_root.clone(), self.global_root.clone())
        }

        fn write(&self, scope: &str, relative_path: &str, content: &str) {
            let root = match scope {
                "local" => &self.local_root,
                "global" => &self.global_root,
                _ => panic!("unknown scope"),
            };
            let path = root.join(relative_path);
            fs::create_dir_all(path.parent().expect("fixture file should have a parent"))
                .expect("fixture parent should exist");
            fs::write(path, content).expect("fixture file should be written");
        }

        fn init_git_repo(&self, scope: &str, relative_path: &str) {
            let root = match scope {
                "local" => &self.local_root,
                "global" => &self.global_root,
                _ => panic!("unknown scope"),
            };
            let worktree = root.join(relative_path);
            fs::create_dir_all(&worktree).expect("git worktree should exist");

            run_git(root, &["init", worktree.as_str()]);
            run_git(
                root,
                &["-C", worktree.as_str(), "config", "user.name", "Codex"],
            );
            run_git(
                root,
                &["-C", worktree.as_str(), "config", "commit.gpgsign", "false"],
            );
            run_git(
                root,
                &[
                    "-C",
                    worktree.as_str(),
                    "config",
                    "user.email",
                    "codex@example.invalid",
                ],
            );
            self.write(scope, &format!("{relative_path}\\README.md"), "seed");
            run_git(root, &["-C", worktree.as_str(), "add", "."]);
            run_git(root, &["-C", worktree.as_str(), "commit", "-m", "seed"]);
            let branch = git_stdout(root, &["-C", worktree.as_str(), "branch", "--show-current"]);
            let branch = branch.trim();
            let head = git_stdout(root, &["-C", worktree.as_str(), "rev-parse", "HEAD"]);
            run_git(
                root,
                &[
                    "-C",
                    worktree.as_str(),
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    head.trim(),
                ],
            );
        }

        fn push_git_update(
            &self,
            scope: &str,
            relative_path: &str,
            changed_path: &str,
            content: &str,
        ) {
            let root = match scope {
                "local" => &self.local_root,
                "global" => &self.global_root,
                _ => panic!("unknown scope"),
            };
            let worktree = root.join(relative_path);
            let branch = git_stdout(root, &["-C", worktree.as_str(), "branch", "--show-current"]);
            let branch = branch.trim().to_owned();
            let base_head = git_stdout(root, &["-C", worktree.as_str(), "rev-parse", "HEAD"]);
            let changed = worktree.join(changed_path);
            fs::create_dir_all(changed.parent().expect("changed file should have a parent"))
                .expect("changed file parent should exist");
            fs::write(&changed, content).expect("changed file should be written");
            run_git(root, &["-C", worktree.as_str(), "add", "."]);
            run_git(root, &["-C", worktree.as_str(), "commit", "-m", "update"]);
            let updated_head = git_stdout(root, &["-C", worktree.as_str(), "rev-parse", "HEAD"]);
            run_git(
                root,
                &["-C", worktree.as_str(), "reset", "--hard", base_head.trim()],
            );
            run_git(
                root,
                &[
                    "-C",
                    worktree.as_str(),
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    updated_head.trim(),
                ],
            );

            assert!(
                worktree.join(".git").exists(),
                "fixture repo should still exist"
            );
        }
    }

    fn run_git(cwd: &Utf8PathBuf, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {:?}: {error}", args));
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(cwd: &Utf8PathBuf, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {:?}: {error}", args));
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}
