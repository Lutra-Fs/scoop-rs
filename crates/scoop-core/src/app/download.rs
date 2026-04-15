use camino::Utf8PathBuf;
use reqwest::blocking::Client;

use crate::{
    RuntimeConfig,
    app::install::{
        arch_specific_strings, choose_architecture, default_architecture, download_filename,
        effective_install_version, fetch_to_path, is_nightly_manifest, manifest_version,
        parse_app_reference, remove_existing_path_if_exists,
        resolve_manifest_reference_for_install, validate_hash,
    },
    infra::http::build_blocking_http_client,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOptions {
    pub use_cache: bool,
    pub check_hash: bool,
    pub architecture: Option<String>,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            use_cache: true,
            check_hash: true,
            architecture: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedFile {
    pub file_name: String,
    pub path: Utf8PathBuf,
    pub loaded_from_cache: bool,
    pub verified_hash: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadOutcome {
    Downloaded {
        version: String,
        files: Vec<DownloadedFile>,
        skipped_hash_verification: bool,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadReport {
    pub app: String,
    pub bucket: Option<String>,
    pub requested_version: Option<String>,
    pub architecture: String,
    pub outcome: DownloadOutcome,
}

struct CachePayloadPlan<'a> {
    app: &'a str,
    version: &'a str,
    urls: &'a [String],
    hashes: &'a [String],
}

pub fn download_apps(
    config: &RuntimeConfig,
    app_references: &[String],
    options: &DownloadOptions,
) -> anyhow::Result<Vec<DownloadReport>> {
    let client = build_blocking_http_client()?;
    let mut reports = Vec::new();
    for app_reference in app_references {
        reports.push(download_single_app(
            config,
            &client,
            app_reference,
            options,
        )?);
    }
    Ok(reports)
}

fn download_single_app(
    config: &RuntimeConfig,
    client: &Client,
    app_reference: &str,
    options: &DownloadOptions,
) -> anyhow::Result<DownloadReport> {
    let parsed = parse_app_reference(app_reference)?;
    let architecture = options
        .architecture
        .clone()
        .unwrap_or_else(|| default_architecture().to_owned());
    let mut report = DownloadReport {
        app: parsed.app.clone(),
        bucket: parsed.bucket.clone(),
        requested_version: parsed.version.clone(),
        architecture: architecture.clone(),
        outcome: DownloadOutcome::Failed {
            message: String::new(),
        },
    };

    let resolved = match resolve_manifest_reference_for_install(config, app_reference) {
        Ok(Some(manifest)) => manifest,
        Ok(None) => {
            report.outcome = DownloadOutcome::Failed {
                message: missing_manifest_message(
                    &parsed.app,
                    parsed.bucket.as_deref(),
                    parsed.url_or_path.as_deref(),
                ),
            };
            return Ok(report);
        }
        Err(error) => {
            report.outcome = DownloadOutcome::Failed {
                message: error.to_string(),
            };
            return Ok(report);
        }
    };

    report.bucket = resolved.bucket.clone().or(report.bucket);

    let Some(manifest_version) = manifest_version(&resolved.manifest) else {
        report.outcome = DownloadOutcome::Failed {
            message: String::from("Manifest doesn't specify a version."),
        };
        return Ok(report);
    };
    if let Some(character) = unsupported_version_character(manifest_version) {
        report.outcome = DownloadOutcome::Failed {
            message: format!("Manifest version has unsupported character '{character}'."),
        };
        return Ok(report);
    }

    let version = effective_install_version(config, &resolved.manifest, manifest_version)?;
    let check_hash = options.check_hash && !is_nightly_manifest(&resolved.manifest);
    let Some(architecture) = choose_architecture(&resolved.manifest, Some(&architecture)) else {
        report.outcome = DownloadOutcome::Failed {
            message: format!("'{}' doesn't support current architecture!", report.app),
        };
        return Ok(report);
    };
    report.architecture = architecture.clone();

    let urls = arch_specific_strings(&resolved.manifest, &architecture, "url");
    if urls.is_empty() {
        report.outcome = DownloadOutcome::Failed {
            message: String::from("manifest doesn't contain a downloadable URL"),
        };
        return Ok(report);
    }
    let hashes = arch_specific_strings(&resolved.manifest, &architecture, "hash");
    if check_hash && !hashes.is_empty() && hashes.len() != urls.len() {
        report.outcome = DownloadOutcome::Failed {
            message: String::from("manifest hash count doesn't match URL count"),
        };
        return Ok(report);
    }

    match cache_payloads(
        client,
        config,
        CachePayloadPlan {
            app: &report.app,
            version: &version,
            urls: &urls,
            hashes: &hashes,
        },
        options,
        check_hash,
    ) {
        Ok((files, skipped_hash_verification)) => {
            report.outcome = DownloadOutcome::Downloaded {
                version,
                files,
                skipped_hash_verification,
            };
        }
        Err(error) => {
            report.outcome = DownloadOutcome::Failed {
                message: error.to_string(),
            };
        }
    }

    Ok(report)
}

fn cache_payloads(
    client: &Client,
    config: &RuntimeConfig,
    plan: CachePayloadPlan<'_>,
    options: &DownloadOptions,
    check_hash: bool,
) -> anyhow::Result<(Vec<DownloadedFile>, bool)> {
    let mut files = Vec::new();
    for (index, url) in plan.urls.iter().enumerate() {
        let file_name = download_filename(url)?;
        let path = config
            .cache_dir()
            .join(format!("{}#{}#{file_name}", plan.app, plan.version));
        let loaded_from_cache = options.use_cache && path.is_file();
        if !loaded_from_cache {
            fetch_to_path(client, url, &path)?;
        }

        let mut verified_hash = false;
        if check_hash && let Some(expected) = plan.hashes.get(index) {
            if let Err(error) = validate_hash(&path, expected, plan.app, url) {
                let _ = remove_existing_path_if_exists(&path);
                return Err(error);
            }
            verified_hash = true;
        }

        files.push(DownloadedFile {
            file_name,
            path,
            loaded_from_cache,
            verified_hash,
        });
    }

    Ok((files, !check_hash))
}

fn missing_manifest_message(app: &str, bucket: Option<&str>, url_or_path: Option<&str>) -> String {
    if let Some(bucket) = bucket {
        format!("Couldn't find manifest for '{app}' from '{bucket}' bucket.")
    } else if let Some(url_or_path) = url_or_path {
        format!("Couldn't find manifest for '{app}' at '{url_or_path}'.")
    } else {
        format!("Couldn't find manifest for '{app}'.")
    }
}

fn unsupported_version_character(version: &str) -> Option<char> {
    version.chars().find(|character| {
        !(character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+' | '_'))
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{DownloadOptions, DownloadOutcome, download_apps};

    #[test]
    fn download_caches_files_and_validates_hashes() {
        let fixture = DownloadFixture::new();
        let archive = fixture.write_file("payload/demo.zip", b"demo");
        let hash = crate::infra::hash::sha256_file(&archive).expect("hash should compute");
        fixture.bucket_manifest(
            "main",
            "demo",
            &format!(
                r#"{{"version":"1.2.3","url":"{}","hash":"{}"}}"#,
                archive.as_str().replace('\\', "\\\\"),
                hash
            ),
        );

        let reports = download_apps(
            &fixture.config(),
            &[String::from("demo")],
            &DownloadOptions::default(),
        )
        .expect("download should succeed");

        assert_eq!(reports.len(), 1);
        match &reports[0].outcome {
            DownloadOutcome::Downloaded { version, files, .. } => {
                assert_eq!(version, "1.2.3");
                assert_eq!(files.len(), 1);
                assert!(files[0].verified_hash);
                assert!(files[0].path.is_file());
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn download_reports_missing_manifest_without_failing_batch() {
        let fixture = DownloadFixture::new();

        let reports = download_apps(
            &fixture.config(),
            &[String::from("missing")],
            &DownloadOptions::default(),
        )
        .expect("download should keep missing manifest in report");

        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].outcome,
            DownloadOutcome::Failed {
                message: String::from("Couldn't find manifest for 'missing'."),
            }
        );
    }

    struct DownloadFixture {
        _temp: TempDir,
        root: Utf8PathBuf,
    }

    impl DownloadFixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            fs::create_dir_all(root.join("local/buckets")).expect("local buckets should exist");
            fs::create_dir_all(root.join("local/cache")).expect("cache dir should exist");
            fs::create_dir_all(root.join("global")).expect("global root should exist");
            Self { _temp: temp, root }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::new(self.root.join("local"), self.root.join("global"))
        }

        fn bucket_manifest(&self, bucket: &str, app: &str, manifest: &str) {
            let path = self
                .root
                .join(format!("local/buckets/{bucket}/bucket/{app}.json"));
            fs::create_dir_all(path.parent().expect("manifest should have a parent"))
                .expect("manifest parent should exist");
            fs::write(path, manifest).expect("fixture manifest should be written");
        }

        fn write_file(&self, relative: &str, bytes: &[u8]) -> Utf8PathBuf {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().expect("file should have a parent"))
                .expect("file parent should exist");
            fs::write(&path, bytes).expect("fixture file should be written");
            path
        }
    }
}
