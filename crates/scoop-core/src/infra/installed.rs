use std::{fs, time::SystemTime};

use anyhow::Context;
use camino::Utf8Path;
use jiff::Timestamp;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use crate::{RuntimeConfig, domain::paths::ScoopPaths};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallScope {
    Local,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
    pub updated: SystemTime,
    pub info: Vec<String>,
    pub scope: InstallScope,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct InstallMetadata {
    #[serde(default)]
    architecture: Option<String>,
    #[serde(default)]
    bucket: Option<String>,
    #[serde(default)]
    hold: Option<bool>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct VersionMetadata {
    version: String,
}

pub fn discover_installed_apps(config: &RuntimeConfig) -> anyhow::Result<Vec<InstalledApp>> {
    let default_architecture = default_architecture();
    let mut apps = Vec::new();

    apps.extend(discover_scope(
        config.paths(),
        InstallScope::Local,
        default_architecture,
    )?);
    apps.extend(discover_scope(
        config.global_paths(),
        InstallScope::Global,
        default_architecture,
    )?);

    Ok(apps)
}

pub fn compile_query(query: &str) -> anyhow::Result<Regex> {
    RegexBuilder::new(query)
        .case_insensitive(true)
        .build()
        .with_context(|| format!("invalid list query regex: {query}"))
}

pub fn filter_installed_apps<'a>(
    apps: &'a [InstalledApp],
    query: Option<&Regex>,
) -> Vec<&'a InstalledApp> {
    apps.iter()
        .filter(|app| query.is_none_or(|pattern| pattern.is_match(&app.name)))
        .collect()
}

pub fn format_updated_time(updated: SystemTime) -> anyhow::Result<String> {
    let timestamp = Timestamp::try_from(updated).context("failed to convert system time")?;
    let zoned = timestamp.to_zoned(jiff::tz::TimeZone::system());
    Ok(zoned.strftime("%Y-%m-%d %H:%M:%S").to_string())
}

pub fn normalize_for_text_comparison(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn discover_scope(
    paths: &ScoopPaths,
    scope: InstallScope,
    default_architecture: &str,
) -> anyhow::Result<Vec<InstalledApp>> {
    let apps_dir = paths.apps();
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

    let mut apps = Vec::with_capacity(app_names.len());
    for name in app_names {
        if let Some(app) = read_app(paths, scope, &name, default_architecture)? {
            apps.push(app);
        }
    }
    Ok(apps)
}

fn read_app(
    paths: &ScoopPaths,
    scope: InstallScope,
    name: &str,
    default_architecture: &str,
) -> anyhow::Result<Option<InstalledApp>> {
    let app_dir = paths.app_dir(name);
    if !app_dir.exists() {
        return Ok(None);
    }

    let current_dir = paths.current_dir(name);
    let version = select_current_version(paths, name)?.unwrap_or_default();
    let failed = app_dir.exists() && (!current_dir.exists() || version.is_empty());

    let version_dir = (!version.is_empty()).then(|| paths.version_dir(name, &version));
    let install_json_path = version_dir
        .as_ref()
        .map(|version_dir| version_dir.join("install.json"));
    let install_metadata = if install_json_path
        .as_ref()
        .is_some_and(|install_json_path| install_json_path.exists())
    {
        install_json_path
            .as_ref()
            .and_then(|install_json_path| read_install_metadata(install_json_path).ok())
    } else {
        None
    };
    let updated = if let Some(install_json_path) = install_json_path
        .as_ref()
        .filter(|install_json_path| install_json_path.exists())
    {
        modified_time(install_json_path)?
    } else {
        modified_time(&app_dir)?
    };

    let source = install_metadata
        .as_ref()
        .and_then(|metadata| metadata.bucket.clone().or_else(|| metadata.url.clone()))
        .map(|source| maybe_auto_generated_source(paths, name, source));

    let mut info = Vec::new();
    if install_metadata
        .as_ref()
        .and_then(|metadata| metadata.bucket.as_deref())
        .is_some_and(|bucket| {
            bucket_root_from_paths(paths, bucket)
                .join("deprecated")
                .join(format!("{name}.json"))
                .exists()
        })
    {
        info.push(String::from("Deprecated package"));
    }
    if matches!(scope, InstallScope::Global) {
        info.push(String::from("Global install"));
    }
    if failed {
        info.push(String::from("Install failed"));
    }
    if install_metadata
        .as_ref()
        .and_then(|metadata| metadata.hold)
        .unwrap_or(false)
    {
        info.push(String::from("Held package"));
    }
    if let Some(architecture) = install_metadata
        .as_ref()
        .and_then(|metadata| metadata.architecture.as_deref())
        .filter(|architecture| *architecture != default_architecture)
    {
        info.push(architecture.to_owned());
    }

    Ok(Some(InstalledApp {
        name: name.to_owned(),
        version,
        source,
        updated,
        info,
        scope,
    }))
}

fn bucket_root_from_paths(paths: &ScoopPaths, bucket: &str) -> camino::Utf8PathBuf {
    paths.buckets().join(bucket)
}

fn select_current_version(paths: &ScoopPaths, name: &str) -> anyhow::Result<Option<String>> {
    let current_dir = paths.current_dir(name);
    let current_manifest = current_dir.join("manifest.json");
    if current_manifest.exists()
        && let Ok(metadata) = read_version_metadata(&current_manifest)
    {
        let version = metadata.version;
        if version == "nightly"
            && let Ok(target) = fs::read_link(&current_dir)
            && let Some(file_name) = target.file_name().and_then(|value| value.to_str())
        {
            return Ok(Some(file_name.to_owned()));
        }
        return Ok(Some(version));
    }

    let app_dir = paths.app_dir(name);
    if !app_dir.exists() {
        return Ok(None);
    }

    let mut installed_versions = fs::read_dir(&app_dir)
        .with_context(|| format!("failed to read versions from {}", app_dir))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .and_then(|_| entry.file_name().into_string().ok())
        })
        .filter(|entry| entry != "current" && !entry.starts_with('_'))
        .filter_map(|version| {
            let install_json = app_dir.join(&version).join("install.json");
            install_json
                .exists()
                .then(|| {
                    modified_time(&install_json)
                        .ok()
                        .map(|time| (time, version))
                })
                .flatten()
        })
        .collect::<Vec<_>>();

    installed_versions.sort_by_key(|(updated, _)| *updated);
    Ok(installed_versions
        .into_iter()
        .last()
        .map(|(_, version)| version))
}

fn read_install_metadata(path: &Utf8Path) -> anyhow::Result<InstallMetadata> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read install metadata {}", path))?;
    serde_json::from_str(&source)
        .with_context(|| format!("failed to parse install metadata {}", path))
}

fn read_version_metadata(path: &Utf8Path) -> anyhow::Result<VersionMetadata> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read version metadata {}", path))?;
    serde_json::from_str(&source)
        .with_context(|| format!("failed to parse version metadata {}", path))
}

fn maybe_auto_generated_source(paths: &ScoopPaths, name: &str, source: String) -> String {
    let user_manifest = paths.workspace().join(format!("{name}.json"));
    if source == user_manifest.as_str() {
        String::from("<auto-generated>")
    } else {
        source
    }
}

fn modified_time(path: &Utf8Path) -> anyhow::Result<SystemTime> {
    fs::metadata(path)
        .with_context(|| format!("failed to read metadata from {}", path))?
        .modified()
        .with_context(|| format!("failed to read modified time from {}", path))
}

fn default_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "64bit",
        "aarch64" => "arm64",
        _ => "32bit",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::{Utf8Path, Utf8PathBuf};
    use tempfile::TempDir;

    use crate::infra::config::RuntimeConfig;

    use super::{InstallScope, compile_query, discover_installed_apps, filter_installed_apps};

    #[test]
    fn discovers_local_and_global_apps_from_fixture_roots() {
        let fixture = Fixture::new();
        fixture.local_app(
            "git",
            "2.53.0.2",
            r#"{"version":"2.53.0.2"}"#,
            r#"{"bucket":"main","architecture":"64bit"}"#,
        );
        fixture.global_app(
            "nodejs",
            "24.2.0",
            r#"{"version":"24.2.0"}"#,
            r#"{"bucket":"main","architecture":"32bit","hold":true}"#,
        );

        let config = fixture.config();
        let apps = discover_installed_apps(&config).expect("fixture apps should be discovered");

        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name, "git");
        assert_eq!(apps[0].version, "2.53.0.2");
        assert_eq!(apps[0].source.as_deref(), Some("main"));
        assert!(apps[0].info.is_empty());
        assert_eq!(apps[0].scope, InstallScope::Local);

        assert_eq!(apps[1].name, "nodejs");
        assert_eq!(apps[1].version, "24.2.0");
        assert_eq!(apps[1].source.as_deref(), Some("main"));
        assert_eq!(
            apps[1].info,
            vec!["Global install", "Held package", "32bit"]
        );
        assert_eq!(apps[1].scope, InstallScope::Global);
    }

    #[test]
    fn filters_queries_case_insensitively_like_powershell_match() {
        let fixture = Fixture::new();
        fixture.local_app(
            "Git",
            "2.53.0.2",
            r#"{"version":"2.53.0.2"}"#,
            r#"{"bucket":"main","architecture":"64bit"}"#,
        );
        fixture.local_app(
            "gh",
            "2.89.0",
            r#"{"version":"2.89.0"}"#,
            r#"{"bucket":"main","architecture":"64bit"}"#,
        );

        let config = fixture.config();
        let apps = discover_installed_apps(&config).expect("fixture apps should be discovered");
        let query = compile_query("^git$").expect("query should compile");
        let matches = filter_installed_apps(&apps, Some(&query));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "Git");
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
            fs::create_dir_all(global_root.join("apps")).expect("global apps dir should exist");

            Self {
                _temp: temp,
                local_root,
                global_root,
            }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::new(self.local_root.clone(), self.global_root.clone())
        }

        fn local_app(&self, name: &str, version: &str, manifest_json: &str, install_json: &str) {
            self.app(&self.local_root, name, version, manifest_json, install_json);
        }

        fn global_app(&self, name: &str, version: &str, manifest_json: &str, install_json: &str) {
            self.app(
                &self.global_root,
                name,
                version,
                manifest_json,
                install_json,
            );
        }

        fn app(
            &self,
            root: &Utf8Path,
            name: &str,
            version: &str,
            manifest_json: &str,
            install_json: &str,
        ) {
            let version_dir = root.join("apps").join(name).join(version);
            let current_dir = root.join("apps").join(name).join("current");
            fs::create_dir_all(&version_dir).expect("version dir should exist");
            fs::create_dir_all(&current_dir).expect("current dir should exist");
            fs::write(version_dir.join("install.json"), install_json)
                .expect("install metadata should exist");
            fs::write(current_dir.join("manifest.json"), manifest_json)
                .expect("current manifest should exist");
        }
    }
}
