use std::fs;

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use serde_json::{Serializer, Value, ser::PrettyFormatter};

use crate::RuntimeConfig;
use crate::infra::http::build_blocking_http_client;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedManifest {
    pub app: String,
    pub bucket: Option<String>,
    pub path: Utf8PathBuf,
    pub manifest: Value,
}

pub fn resolve_manifest(
    config: &RuntimeConfig,
    app_reference: &str,
) -> anyhow::Result<Option<ResolvedManifest>> {
    let app_reference = app_reference.trim_start_matches('/');

    if let Some(manifest) = resolve_manifest_from_source(app_reference)? {
        return Ok(Some(manifest));
    }

    if let Some((bucket, app)) = split_bucket_reference(app_reference) {
        return resolve_manifest_in_bucket(config, bucket, app);
    }

    if let Some(manifest) = resolve_installed_manifest(config, app_reference)? {
        return Ok(Some(manifest));
    }

    resolve_manifest_across_buckets(config, app_reference)
}

pub fn render_manifest_json(manifest: &Value) -> anyhow::Result<String> {
    let mut output = Vec::new();
    let formatter = PrettyFormatter::with_indent(b"    ");
    let mut serializer = Serializer::with_formatter(&mut output, formatter);
    manifest
        .serialize(&mut serializer)
        .context("failed to serialize manifest")?;

    let rendered = String::from_utf8(output).context("manifest JSON should be UTF-8")?;
    Ok(rendered.replace('\n', "\r\n"))
}

fn split_bucket_reference(app_reference: &str) -> Option<(&str, &str)> {
    let (bucket, app) = app_reference.split_once('/')?;
    (!bucket.is_empty() && !app.is_empty()).then_some((bucket, app))
}

fn resolve_installed_manifest(
    config: &RuntimeConfig,
    app: &str,
) -> anyhow::Result<Option<ResolvedManifest>> {
    for root in [config.paths().root(), config.global_paths().root()] {
        let current_manifest = root
            .join("apps")
            .join(app)
            .join("current")
            .join("manifest.json");
        let current_install_info = root
            .join("apps")
            .join(app)
            .join("current")
            .join("install.json");
        if let Some(bucket) = load_bucket_from_install_info(&current_install_info)?
            && let Some(manifest) = resolve_manifest_in_bucket(config, &bucket, app)?
        {
            return Ok(Some(manifest));
        }
        if current_manifest.is_file() {
            let manifest = load_manifest_json(&current_manifest)?;
            return Ok(Some(ResolvedManifest {
                app: app.to_owned(),
                bucket: None,
                path: current_manifest,
                manifest,
            }));
        }
    }

    Ok(None)
}

fn resolve_manifest_from_source(app_reference: &str) -> anyhow::Result<Option<ResolvedManifest>> {
    if !looks_like_manifest_source(app_reference) {
        return Ok(None);
    }
    if app_reference.starts_with("http://") || app_reference.starts_with("https://") {
        let client = build_blocking_http_client()?;
        let manifest = client
            .get(app_reference)
            .send()
            .with_context(|| format!("failed to download manifest {app_reference}"))?
            .error_for_status()
            .with_context(|| format!("failed to download manifest {app_reference}"))?
            .json::<Value>()
            .with_context(|| format!("failed to parse manifest {app_reference}"))?;
        Ok(Some(ResolvedManifest {
            app: Utf8Path::new(app_reference)
                .file_stem()
                .unwrap_or("manifest")
                .to_owned(),
            bucket: None,
            path: Utf8PathBuf::from(app_reference),
            manifest,
        }))
    } else {
        let path = Utf8Path::new(app_reference);
        let manifest = load_manifest_json(path).with_context(|| {
            format!("failed to parse manifest {app_reference} from file path {path}")
        })?;
        Ok(Some(ResolvedManifest {
            app: path
                .file_stem()
                .with_context(|| format!("invalid manifest path {app_reference}"))?
                .to_owned(),
            bucket: None,
            path: Utf8PathBuf::from(app_reference),
            manifest,
        }))
    }
}

fn looks_like_manifest_source(app_reference: &str) -> bool {
    app_reference.starts_with("http://")
        || app_reference.starts_with("https://")
        || app_reference.starts_with("\\\\")
        || app_reference.ends_with(".json")
            && (app_reference.contains('\\') || app_reference.contains('/'))
}

fn resolve_manifest_across_buckets(
    config: &RuntimeConfig,
    app: &str,
) -> anyhow::Result<Option<ResolvedManifest>> {
    let buckets_root = config.paths().buckets();
    if !buckets_root.exists() {
        return Ok(None);
    }

    let mut buckets = fs::read_dir(&buckets_root)
        .with_context(|| format!("failed to read buckets directory {}", buckets_root))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("failed to enumerate buckets directory {}", buckets_root))?;
    buckets.sort_by_key(|entry| entry.file_name());

    for bucket in buckets {
        let bucket_name = bucket.file_name().to_string_lossy().into_owned();
        if let Some(manifest) = resolve_manifest_in_bucket(config, &bucket_name, app)? {
            return Ok(Some(manifest));
        }
    }

    Ok(None)
}

fn resolve_manifest_in_bucket(
    config: &RuntimeConfig,
    bucket: &str,
    app: &str,
) -> anyhow::Result<Option<ResolvedManifest>> {
    for candidate in bucket_manifest_candidates(config, bucket, app) {
        if candidate.is_file() {
            let manifest = load_manifest_json(&candidate)?;
            return Ok(Some(ResolvedManifest {
                app: app.to_owned(),
                bucket: Some(bucket.to_owned()),
                path: candidate,
                manifest,
            }));
        }
    }

    Ok(None)
}

fn bucket_manifest_candidates(config: &RuntimeConfig, bucket: &str, app: &str) -> [Utf8PathBuf; 2] {
    let bucket_root = config.paths().buckets().join(bucket);
    [
        bucket_root.join("bucket").join(format!("{app}.json")),
        bucket_root.join("deprecated").join(format!("{app}.json")),
    ]
}

fn load_manifest_json(path: &Utf8Path) -> anyhow::Result<Value> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read manifest {}", path))?;
    serde_json::from_str(&source).with_context(|| format!("failed to parse manifest {}", path))
}

fn load_bucket_from_install_info(path: &Utf8Path) -> anyhow::Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }

    let install_info = load_manifest_json(path)?;
    Ok(install_info
        .get("bucket")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use serde_json::json;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{render_manifest_json, resolve_manifest};

    #[test]
    fn resolves_bucket_manifest_for_installed_app_when_bucket_is_known() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "apps\\git\\current\\manifest.json",
            r#"{"version":"1.0.0","description":"installed"}"#,
        );
        fixture.write(
            "local",
            "apps\\git\\current\\install.json",
            r#"{"bucket":"main"}"#,
        );
        fixture.write(
            "local",
            "buckets\\main\\bucket\\git.json",
            r#"{"version":"2.0.0","description":"bucket"}"#,
        );

        let manifest = resolve_manifest(&fixture.config(), "git")
            .expect("manifest lookup should succeed")
            .expect("manifest should exist");

        assert_eq!(manifest.app, "git");
        assert_eq!(manifest.bucket.as_deref(), Some("main"));
        assert_eq!(manifest.manifest["description"], "bucket");
    }

    #[test]
    fn falls_back_to_installed_manifest_without_bucket_metadata() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "apps\\git\\current\\manifest.json",
            r#"{"version":"1.0.0","description":"installed"}"#,
        );

        let manifest = resolve_manifest(&fixture.config(), "git")
            .expect("manifest lookup should succeed")
            .expect("manifest should exist");

        assert_eq!(manifest.bucket, None);
        assert_eq!(manifest.manifest["description"], "installed");
    }

    #[test]
    fn resolves_explicit_bucket_and_deprecated_entries() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "buckets\\versions\\deprecated\\nodejs.json",
            r#"{"version":"22.0.0"}"#,
        );

        let manifest = resolve_manifest(&fixture.config(), "versions/nodejs")
            .expect("manifest lookup should succeed")
            .expect("manifest should exist");

        assert_eq!(manifest.app, "nodejs");
        assert_eq!(manifest.bucket.as_deref(), Some("versions"));
        assert!(manifest.path.ends_with("deprecated/nodejs.json"));
    }

    #[test]
    fn pretty_prints_with_four_space_indentation_and_crlf() {
        let rendered = render_manifest_json(&json!({
            "version": "1.2.3",
            "bin": ["tool.exe"]
        }))
        .expect("manifest should render");

        assert_eq!(
            rendered,
            "{\r\n    \"version\": \"1.2.3\",\r\n    \"bin\": [\r\n        \"tool.exe\"\r\n    ]\r\n}"
        );
    }

    #[test]
    fn resolves_manifest_from_explicit_json_path() {
        let fixture = Fixture::new();
        let manifest_path = format!("{}\\manual\\manifests\\demo.json", fixture.local_root);
        fixture.write_file(
            &manifest_path,
            r#"{"version":"1.0.0","description":"direct"}"#,
        );

        let manifest = resolve_manifest(&fixture.config(), &manifest_path)
            .expect("manifest lookup should succeed")
            .expect("manifest should exist");

        assert_eq!(manifest.app, "demo");
        assert_eq!(manifest.bucket, None);
        assert_eq!(manifest.path, Utf8PathBuf::from(&manifest_path));
        assert_eq!(manifest.manifest["description"], "direct");
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

        fn write_file(&self, path: &str, content: &str) {
            let path = Utf8PathBuf::from(path);
            fs::create_dir_all(path.parent().expect("fixture path should have a parent"))
                .expect("fixture file parent should exist");
            fs::write(path, content).expect("fixture file should be written");
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
    }
}
