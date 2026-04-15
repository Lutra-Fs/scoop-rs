use std::time::SystemTime;

use anyhow::Context;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    RuntimeConfig,
    app::{bucket::list_buckets, config::current_config, list::list_installed},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportBucket {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Source")]
    pub source: String,
    #[serde(rename = "Updated")]
    pub updated: String,
    #[serde(rename = "Manifests")]
    pub manifests: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportApp {
    #[serde(rename = "Info")]
    pub info: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Updated")]
    pub updated: String,
    #[serde(rename = "Source")]
    pub source: String,
    #[serde(rename = "Version")]
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportPayload {
    pub apps: Vec<ExportApp>,
    pub buckets: Vec<ExportBucket>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Map<String, Value>>,
}

pub fn export_state(config: &RuntimeConfig, include_config: bool) -> anyhow::Result<ExportPayload> {
    let apps = list_installed(config, None)?
        .apps
        .into_iter()
        .map(|app| {
            Ok(ExportApp {
                info: app.info.join(", "),
                name: app.name,
                updated: format_export_updated(app.updated)?,
                source: app.source.unwrap_or_default(),
                version: app.version,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let buckets = list_buckets(config)?
        .into_iter()
        .map(|bucket| {
            Ok(ExportBucket {
                name: bucket.name,
                source: bucket.source,
                updated: format_export_bucket_updated(bucket.updated)?,
                manifests: bucket.manifests,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let config = include_config.then(|| sanitize_export_config(current_config(config)));

    Ok(ExportPayload {
        apps,
        buckets,
        config,
    })
}

pub fn render_export_json(payload: &ExportPayload) -> anyhow::Result<String> {
    serde_json::to_string_pretty(payload)
        .map(|json| json.replace('\n', "\r\n"))
        .context("failed to serialize export payload")
}

fn sanitize_export_config(mut config: Map<String, Value>) -> Map<String, Value> {
    for key in [
        "last_update",
        "root_path",
        "global_path",
        "cache_path",
        "alias",
    ] {
        config.remove(key);
    }
    config
}

fn format_export_updated(updated: SystemTime) -> anyhow::Result<String> {
    let timestamp = jiff::Timestamp::try_from(updated).context("failed to convert system time")?;
    let zoned = timestamp.to_zoned(jiff::tz::TimeZone::system());
    let offset = zoned.strftime("%:z").to_string();
    let fraction = format!("{:09}", timestamp.subsec_nanosecond() / 100);
    Ok(format!(
        "{}.{}{}",
        zoned.strftime("%Y-%m-%dT%H:%M:%S"),
        &fraction[..7],
        offset
    ))
}

fn format_export_bucket_updated(updated: SystemTime) -> anyhow::Result<String> {
    let timestamp = jiff::Timestamp::try_from(updated).context("failed to convert system time")?;
    Ok(timestamp
        .to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%dT%H:%M:%S%:z")
        .to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{export_state, render_export_json};

    #[test]
    fn exports_apps_buckets_and_filtered_config() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "apps\\scoop\\current\\buckets.json",
            r#"{"main":"https://github.com/ScoopInstaller/Main"}"#,
        );
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            r#"{"version":"1.0.0"}"#,
        );
        fixture.write(
            "local",
            "apps\\demo\\1.0.0\\install.json",
            r#"{"bucket":"main"}"#,
        );
        fixture.write(
            "local",
            "apps\\demo\\current\\manifest.json",
            r#"{"version":"1.0.0"}"#,
        );
        fixture.write(
            "local",
            "config.json",
            r#"{"use_sqlite_cache":true,"last_update":"2026-01-01T00:00:00Z","alias":{"ls":"list"}}"#,
        );

        let exported = export_state(&fixture.config(), true).expect("export should succeed");

        assert_eq!(exported.apps.len(), 1);
        assert_eq!(exported.apps[0].name, "demo");
        assert_eq!(exported.apps[0].source, "main");
        assert_eq!(exported.buckets.len(), 1);
        assert_eq!(exported.buckets[0].name, "main");
        assert_eq!(exported.buckets[0].manifests, 1);
        assert_eq!(
            exported.config,
            Some(serde_json::from_value(json!({"use_sqlite_cache":true})).expect("map"))
        );
    }

    #[test]
    fn renders_pretty_json_with_crlf() {
        let payload = super::ExportPayload {
            apps: vec![super::ExportApp {
                info: String::new(),
                name: String::from("demo"),
                updated: String::from("2026-04-15T12:00:00.0000000+10:00"),
                source: String::from("main"),
                version: String::from("1.0.0"),
            }],
            buckets: vec![super::ExportBucket {
                name: String::from("main"),
                source: String::from("https://example.invalid/main"),
                updated: String::from("2026-04-15T12:00:00+10:00"),
                manifests: 1,
            }],
            config: None,
        };

        let rendered = render_export_json(&payload).expect("payload should render");

        assert!(rendered.contains("\r\n"));
        assert_eq!(
            serde_json::from_str::<Value>(&rendered).expect("json"),
            json!({
                "apps": [{
                    "Info": "",
                    "Name": "demo",
                    "Updated": "2026-04-15T12:00:00.0000000+10:00",
                    "Source": "main",
                    "Version": "1.0.0"
                }],
                "buckets": [{
                    "Name": "main",
                    "Source": "https://example.invalid/main",
                    "Updated": "2026-04-15T12:00:00+10:00",
                    "Manifests": 1
                }]
            })
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
    }
}
