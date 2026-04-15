use crate::{
    RuntimeConfig,
    app::install::{
        default_architecture, manifest_dependencies, resolve_manifest_reference_for_install,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyRow {
    pub source: String,
    pub name: String,
}

pub fn list_dependencies(
    config: &RuntimeConfig,
    app_reference: &str,
    architecture: Option<&str>,
) -> anyhow::Result<Vec<DependencyRow>> {
    let architecture = architecture.unwrap_or(default_architecture());
    let mut rows = Vec::new();
    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();
    append_dependencies(
        config,
        app_reference,
        architecture,
        &mut rows,
        &mut resolved,
        &mut unresolved,
    )?;
    Ok(rows)
}

fn append_dependencies(
    config: &RuntimeConfig,
    app_reference: &str,
    architecture: &str,
    rows: &mut Vec<DependencyRow>,
    resolved: &mut Vec<String>,
    unresolved: &mut Vec<String>,
) -> anyhow::Result<()> {
    let manifest = resolve_manifest_reference_for_install(config, app_reference)?
        .ok_or_else(|| anyhow::anyhow!("Couldn't find manifest for '{app_reference}'."))?;
    let canonical = canonical_reference(app_reference, &manifest);

    if resolved.contains(&canonical) {
        return Ok(());
    }
    if unresolved.contains(&canonical) {
        anyhow::bail!("Circular dependency detected: '{canonical}'.");
    }

    unresolved.push(canonical.clone());
    for dependency in manifest_dependencies(config, &manifest.manifest, architecture) {
        append_dependencies(
            config,
            &dependency,
            architecture,
            rows,
            resolved,
            unresolved,
        )?;
    }
    unresolved.retain(|candidate| candidate != &canonical);
    resolved.push(canonical);
    rows.push(DependencyRow {
        source: manifest
            .bucket
            .unwrap_or_else(|| source_reference(app_reference)),
        name: manifest.app,
    });
    Ok(())
}

fn canonical_reference(
    app_reference: &str,
    manifest: &crate::compat::catalog::ResolvedManifest,
) -> String {
    manifest
        .bucket
        .as_deref()
        .map(|bucket| format!("{bucket}/{}", manifest.app))
        .unwrap_or_else(|| {
            let source = source_reference(app_reference);
            if source.is_empty() {
                manifest.app.clone()
            } else {
                source
            }
        })
}

fn source_reference(app_reference: &str) -> String {
    if app_reference.starts_with("http://")
        || app_reference.starts_with("https://")
        || app_reference.contains(':')
        || app_reference.contains('\\')
    {
        app_reference.to_owned()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Context;
    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use super::{DependencyRow, list_dependencies};
    use crate::RuntimeConfig;

    #[test]
    fn lists_dependencies_in_install_order() {
        let fixture = DependsFixture::new();
        fixture.bucket_manifest("main", "dep", r#"{"version":"1.0.0"}"#);
        fixture.bucket_manifest("main", "demo", r#"{"version":"2.0.0","depends":"dep"}"#);

        let rows = list_dependencies(&fixture.config(), "demo", None)
            .expect("dependency list should resolve");

        assert_eq!(
            rows,
            vec![
                DependencyRow {
                    source: String::from("main"),
                    name: String::from("dep"),
                },
                DependencyRow {
                    source: String::from("main"),
                    name: String::from("demo"),
                },
            ]
        );
    }

    #[test]
    fn missing_manifest_returns_actionable_error() {
        let fixture = DependsFixture::new();

        let error =
            list_dependencies(&fixture.config(), "missing", None).expect_err("missing must fail");

        assert_eq!(error.to_string(), "Couldn't find manifest for 'missing'.");
    }

    struct DependsFixture {
        _temp: TempDir,
        root: Utf8PathBuf,
    }

    impl DependsFixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            fs::create_dir_all(root.join("local/buckets")).expect("local buckets should exist");
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
            fs::write(&path, manifest)
                .with_context(|| format!("failed to write fixture manifest {}", path))
                .expect("fixture manifest should be written");
        }
    }
}
