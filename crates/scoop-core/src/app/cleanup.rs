use std::fs;

use anyhow::{Context, bail};
use camino::Utf8PathBuf;
use serde_json::Value;

use crate::{
    RuntimeConfig,
    app::{
        install::{current_version, installed_versions, is_admin},
        uninstall::unlink_persist_data,
    },
    infra::cache::parse_cache_entry_name,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupOptions {
    pub global: bool,
    pub cache: bool,
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupOutcome {
    Cleaned {
        app: String,
        removed_versions: Vec<String>,
        removed_cache_files: Vec<String>,
    },
    AlreadyClean {
        app: String,
    },
    NotInstalled {
        app: String,
    },
}

pub fn cleanup_apps(
    config: &RuntimeConfig,
    apps: &[String],
    options: &CleanupOptions,
) -> anyhow::Result<Vec<CleanupOutcome>> {
    ensure_admin_for_global(options)?;

    let mut targets = if options.all {
        installed_app_names(config, options.global)?
    } else {
        apps.to_vec()
    };
    targets.sort();
    targets.dedup();

    let mut outcomes = Vec::new();
    for app in targets {
        outcomes.push(cleanup_single_app(config, &app, options)?);
    }

    if options.cache {
        remove_download_markers(config)?;
    }
    Ok(outcomes)
}

fn ensure_admin_for_global(options: &CleanupOptions) -> anyhow::Result<()> {
    if !options.global || is_admin()? {
        return Ok(());
    }
    bail!("you need admin rights to cleanup global apps");
}

fn installed_app_names(config: &RuntimeConfig, global: bool) -> anyhow::Result<Vec<String>> {
    let root = if global {
        config.global_paths().root()
    } else {
        config.paths().root()
    };
    let apps_dir = root.join("apps");
    if !apps_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = fs::read_dir(&apps_dir)
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
    names.sort();
    Ok(names)
}

fn cleanup_single_app(
    config: &RuntimeConfig,
    app: &str,
    options: &CleanupOptions,
) -> anyhow::Result<CleanupOutcome> {
    let paths = if options.global {
        config.global_paths()
    } else {
        config.paths()
    };
    let Some(current) = current_version(paths.root(), app)? else {
        return Ok(CleanupOutcome::NotInstalled {
            app: app.to_owned(),
        });
    };

    let app_dir = paths.app_dir(app);
    let manifest = load_manifest(&paths.version_dir(app, &current))?;
    let mut versions = installed_versions(paths.root(), app)?
        .into_iter()
        .filter(|version| version != &current)
        .collect::<Vec<_>>();
    let removed_cache_files = if options.cache {
        remove_outdated_cache_files(config, app, &current)?
    } else {
        Vec::new()
    };

    if versions.is_empty() && removed_cache_files.is_empty() {
        return Ok(CleanupOutcome::AlreadyClean {
            app: app.to_owned(),
        });
    }

    versions.sort();
    for version in &versions {
        let version_dir = paths.version_dir(app, version);
        if let Some(manifest) = manifest.as_ref() {
            let _ = unlink_persist_data(manifest, &version_dir);
        }
        let _ = fs::remove_dir_all(&version_dir);
    }

    if app_dir.is_dir() && is_effectively_empty(&app_dir)? {
        let _ = fs::remove_dir_all(&app_dir);
    }

    Ok(CleanupOutcome::Cleaned {
        app: app.to_owned(),
        removed_versions: versions,
        removed_cache_files,
    })
}

fn load_manifest(version_dir: &camino::Utf8Path) -> anyhow::Result<Option<Value>> {
    let path = version_dir.join("manifest.json");
    if !path.is_file() {
        return Ok(None);
    }
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read manifest {}", path))?;
    let manifest = serde_json::from_str(&source)
        .with_context(|| format!("failed to parse manifest {}", path))?;
    Ok(Some(manifest))
}

fn remove_outdated_cache_files(
    config: &RuntimeConfig,
    app: &str,
    current_version: &str,
) -> anyhow::Result<Vec<String>> {
    let cache_dir = config.cache_dir();
    if !cache_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut removed = Vec::new();
    for entry in fs::read_dir(cache_dir).with_context(|| format!("failed to read {}", cache_dir))? {
        let entry = entry?;
        let file_type = entry.file_type().with_context(|| {
            format!(
                "failed to read file type for cache entry {}",
                entry.path().display()
            )
        })?;
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some((candidate_app, version, _)) = parse_cache_entry_name(&file_name) else {
            continue;
        };
        if !candidate_app.eq_ignore_ascii_case(app) || version == current_version {
            continue;
        }
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|_| anyhow::anyhow!("cache path must be valid UTF-8"))?;
        fs::remove_file(&path).with_context(|| format!("failed to remove cached file {}", path))?;
        removed.push(file_name);
    }
    removed.sort();
    Ok(removed)
}

fn remove_download_markers(config: &RuntimeConfig) -> anyhow::Result<()> {
    let cache_dir = config.cache_dir();
    if !cache_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(cache_dir).with_context(|| format!("failed to read {}", cache_dir))? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|_| anyhow::anyhow!("cache path must be valid UTF-8"))?;
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("download"))
        {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn is_effectively_empty(app_dir: &camino::Utf8Path) -> anyhow::Result<bool> {
    let mut entries = fs::read_dir(app_dir)
        .with_context(|| format!("failed to read app directory {}", app_dir))?;
    Ok(entries.next().is_none())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{CleanupOptions, CleanupOutcome, cleanup_apps};

    #[test]
    fn cleanup_removes_old_versions_and_old_cache() {
        let fixture = CleanupFixture::new();
        fixture.app("demo", "1.0.0");
        fixture.app("demo", "2.0.0");
        fixture.current_manifest("demo", "2.0.0");
        fixture.cache_file("demo#1.0.0#demo.zip");
        fixture.cache_file("demo#2.0.0#demo.zip");

        let outcomes = cleanup_apps(
            &fixture.config,
            &[String::from("demo")],
            &CleanupOptions {
                cache: true,
                ..CleanupOptions::default()
            },
        )
        .expect("cleanup should succeed");

        assert_eq!(
            outcomes,
            vec![CleanupOutcome::Cleaned {
                app: String::from("demo"),
                removed_versions: vec![String::from("1.0.0")],
                removed_cache_files: vec![String::from("demo#1.0.0#demo.zip")],
            }]
        );
        assert!(!fixture.local_root.join("apps/demo/1.0.0").is_dir());
        assert!(fixture.local_root.join("apps/demo/2.0.0").is_dir());
        assert!(
            !fixture
                .local_root
                .join("cache/demo#1.0.0#demo.zip")
                .is_file()
        );
        assert!(
            fixture
                .local_root
                .join("cache/demo#2.0.0#demo.zip")
                .is_file()
        );
    }

    struct CleanupFixture {
        _temp: TempDir,
        local_root: Utf8PathBuf,
        config: RuntimeConfig,
    }

    impl CleanupFixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            let local_root = root.join("local");
            let global_root = root.join("global");
            fs::create_dir_all(local_root.join("apps")).expect("apps dir should exist");
            fs::create_dir_all(local_root.join("cache")).expect("cache dir should exist");
            fs::create_dir_all(&global_root).expect("global root should exist");
            Self {
                _temp: temp,
                config: RuntimeConfig::new(local_root.clone(), global_root),
                local_root,
            }
        }

        fn app(&self, app: &str, version: &str) {
            let version_dir = self.local_root.join("apps").join(app).join(version);
            fs::create_dir_all(&version_dir).expect("version dir should exist");
            fs::write(version_dir.join("install.json"), r#"{"bucket":"main"}"#)
                .expect("install info should exist");
            fs::write(
                version_dir.join("manifest.json"),
                format!(r#"{{"version":"{version}"}}"#),
            )
            .expect("manifest should exist");
        }

        fn current_manifest(&self, app: &str, version: &str) {
            let current_dir = self.local_root.join("apps").join(app).join("current");
            fs::create_dir_all(&current_dir).expect("current dir should exist");
            fs::write(
                current_dir.join("manifest.json"),
                format!(r#"{{"version":"{version}"}}"#),
            )
            .expect("current manifest should exist");
        }

        fn cache_file(&self, name: &str) {
            fs::write(self.local_root.join("cache").join(name), b"cache")
                .expect("cache file should exist");
        }
    }
}
