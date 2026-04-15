use std::fs;

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use rusqlite::{Connection, params};
use serde_json::Value;

use crate::{
    RuntimeConfig,
    infra::buckets::{bucket_manifests_dir, local_bucket_names},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedSearchRow {
    pub name: String,
    pub version: String,
    pub bucket: String,
    pub binary: Option<String>,
}

pub fn find_search_items(
    config: &RuntimeConfig,
    pattern: &str,
) -> anyhow::Result<Vec<CachedSearchRow>> {
    let database_path = cache_database_path(config);
    let database_existed = database_path.is_file();
    let connection = open_or_recreate_database(&database_path)?;

    if !database_existed || cache_is_empty(&connection)? {
        rebuild_search_cache_with_connection(config, &connection)?;
    }

    query_search_items(&connection, pattern)
}

pub fn rebuild_search_cache(config: &RuntimeConfig) -> anyhow::Result<()> {
    let database_path = cache_database_path(config);
    let connection = open_or_recreate_database(&database_path)?;
    rebuild_search_cache_with_connection(config, &connection)
}

fn cache_database_path(config: &RuntimeConfig) -> Utf8PathBuf {
    config.paths().root().join("scoop.db")
}

fn open_or_recreate_database(path: &Utf8Path) -> anyhow::Result<Connection> {
    match open_database(path) {
        Ok(connection) => Ok(connection),
        Err(original_error) if path.is_file() => {
            let _ = fs::remove_file(path);
            open_database(path)
                .with_context(|| {
                    format!(
                        "failed to recreate SQLite cache after opening {} failed",
                        path
                    )
                })
                .map_err(|recreated_error| original_error.context(recreated_error))
        }
        Err(error) => Err(error),
    }
}

fn open_database(path: &Utf8Path) -> anyhow::Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create cache directory {}", parent))?;
    }

    let connection = Connection::open(path.as_std_path())
        .with_context(|| format!("failed to open SQLite cache {}", path))?;
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS app (
                name TEXT NOT NULL COLLATE NOCASE,
                description TEXT NOT NULL,
                version TEXT NOT NULL,
                bucket VARCHAR NOT NULL,
                manifest JSON NOT NULL,
                binary TEXT,
                shortcut TEXT,
                dependency TEXT,
                suggest TEXT,
                PRIMARY KEY (name, version, bucket)
            )",
        )
        .with_context(|| format!("failed to initialize SQLite cache schema {}", path))?;
    Ok(connection)
}

fn cache_is_empty(connection: &Connection) -> anyhow::Result<bool> {
    let row_count = connection
        .query_row("SELECT COUNT(*) FROM app", [], |row| row.get::<_, i64>(0))
        .context("failed to count cached app rows")?;
    Ok(row_count == 0)
}

fn rebuild_cache(connection: &Connection, manifests: &[ManifestEntry]) -> anyhow::Result<()> {
    let transaction = connection
        .unchecked_transaction()
        .context("failed to start SQLite cache rebuild transaction")?;
    transaction
        .execute("DELETE FROM app", [])
        .context("failed to clear SQLite cache contents")?;

    {
        let mut statement = transaction
            .prepare(
                "INSERT OR REPLACE INTO app (
                    name, description, version, bucket, manifest, binary, shortcut, dependency, suggest
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .context("failed to prepare SQLite cache insert statement")?;

        for manifest in manifests {
            let Some(record) = build_record(manifest)? else {
                continue;
            };
            statement
                .execute(params![
                    record.name,
                    record.description,
                    record.version,
                    record.bucket,
                    record.manifest,
                    record.binary,
                    record.shortcut,
                    record.dependency,
                    record.suggest,
                ])
                .with_context(|| {
                    format!(
                        "failed to insert cached manifest {} from bucket {}",
                        manifest.name, manifest.bucket
                    )
                })?;
        }
    }

    transaction
        .commit()
        .context("failed to commit SQLite cache rebuild")
}

fn rebuild_search_cache_with_connection(
    config: &RuntimeConfig,
    connection: &Connection,
) -> anyhow::Result<()> {
    let manifests = local_manifest_entries(config)?;
    rebuild_cache(connection, &manifests)
}

fn query_search_items(
    connection: &Connection,
    pattern: &str,
) -> anyhow::Result<Vec<CachedSearchRow>> {
    let like_pattern = if pattern.is_empty() {
        String::from("%")
    } else {
        format!("%{pattern}%")
    };
    let mut statement = connection
        .prepare(
            "SELECT name, version, bucket, binary
             FROM (
                SELECT * FROM app
                WHERE name LIKE ?1 OR binary LIKE ?1 OR shortcut LIKE ?1
                ORDER BY version DESC
             )
             GROUP BY name, bucket",
        )
        .context("failed to prepare SQLite search query")?;

    let rows = statement
        .query_map([like_pattern], |row| {
            Ok(CachedSearchRow {
                name: row.get(0)?,
                version: row.get(1)?,
                bucket: row.get(2)?,
                binary: row.get(3)?,
            })
        })
        .context("failed to execute SQLite search query")?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row.context("failed to read SQLite search row")?);
    }
    Ok(results)
}

#[derive(Debug, Clone)]
struct ManifestEntry {
    bucket: String,
    name: String,
    path: Utf8PathBuf,
}

#[derive(Debug, Clone)]
struct CacheRecord {
    name: String,
    description: String,
    version: String,
    bucket: String,
    manifest: String,
    binary: Option<String>,
    shortcut: Option<String>,
    dependency: Option<String>,
    suggest: Option<String>,
}

fn local_manifest_entries(config: &RuntimeConfig) -> anyhow::Result<Vec<ManifestEntry>> {
    let mut manifests = Vec::new();
    for bucket in local_bucket_names(config)? {
        let manifests_dir = bucket_manifests_dir(config, Some(&bucket));
        if !manifests_dir.is_dir() {
            continue;
        }

        for path in walk_json_manifests(&manifests_dir)? {
            let Some(name) = path.file_stem().map(str::to_owned) else {
                continue;
            };
            manifests.push(ManifestEntry {
                bucket: bucket.clone(),
                name,
                path,
            });
        }
    }
    Ok(manifests)
}

fn walk_json_manifests(root: &Utf8Path) -> anyhow::Result<Vec<Utf8PathBuf>> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(directory) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    if let Ok(path) = Utf8PathBuf::from_path_buf(path) {
                        stack.push(path);
                    }
                }
                Ok(kind)
                    if kind.is_file()
                        && path
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("json")) =>
                {
                    if let Ok(path) = Utf8PathBuf::from_path_buf(path) {
                        files.push(path);
                    }
                }
                _ => {}
            }
        }
    }

    files.sort();
    Ok(files)
}

fn build_record(manifest: &ManifestEntry) -> anyhow::Result<Option<CacheRecord>> {
    let source = fs::read_to_string(&manifest.path)
        .with_context(|| format!("failed to read manifest {}", manifest.path))?;
    let value: Value = match serde_json::from_str(&source) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let Some(version) = value.get("version").and_then(Value::as_str) else {
        return Ok(None);
    };

    Ok(Some(CacheRecord {
        name: manifest.name.clone(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        version: version.to_owned(),
        bucket: manifest.bucket.clone(),
        manifest: source,
        binary: render_binary_cache_value(&value),
        shortcut: render_shortcut_cache_value(&value),
        dependency: render_dependency_cache_value(&value),
        suggest: render_suggest_cache_value(&value),
    }))
}

fn render_binary_cache_value(manifest: &Value) -> Option<String> {
    let value = arch_specific_value(manifest, "bin")?;
    let binaries = match value {
        Value::String(binary) => vec![binary_cache_name(binary)],
        Value::Array(entries) => entries
            .iter()
            .filter_map(|entry| match entry {
                Value::String(binary) => Some(binary_cache_name(binary)),
                Value::Array(parts) if !parts.is_empty() => {
                    let binary = parts.first().and_then(Value::as_str)?;
                    let alias = parts.get(1).and_then(Value::as_str);
                    Some(binary_cache_alias(binary, alias))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    (!binaries.is_empty()).then(|| binaries.join(" | "))
}

fn render_shortcut_cache_value(manifest: &Value) -> Option<String> {
    let value = arch_specific_value(manifest, "shortcuts")?;
    let shortcuts = match value {
        Value::Array(entries) => entries
            .iter()
            .filter_map(|entry| match entry {
                Value::Array(parts) if parts.len() >= 2 => parts
                    .get(1)
                    .and_then(Value::as_str)
                    .map(shortcut_cache_name),
                _ => None,
            })
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };

    (!shortcuts.is_empty()).then(|| shortcuts.join(" | "))
}

fn render_dependency_cache_value(manifest: &Value) -> Option<String> {
    let dependencies = string_or_array_values(manifest.get("depends"));
    (!dependencies.is_empty()).then(|| dependencies.join(" | "))
}

fn render_suggest_cache_value(manifest: &Value) -> Option<String> {
    let Value::Object(entries) = manifest.get("suggest")? else {
        return None;
    };
    let mut suggestions = Vec::new();
    for value in entries.values() {
        suggestions.extend(string_or_array_values(Some(value)));
    }
    (!suggestions.is_empty()).then(|| suggestions.join(" | "))
}

fn arch_specific_value<'a>(manifest: &'a Value, field: &str) -> Option<&'a Value> {
    manifest
        .get("architecture")
        .and_then(|value| value.get(default_architecture_key()))
        .and_then(|value| value.get(field))
        .or_else(|| manifest.get(field))
}

fn default_architecture_key() -> &'static str {
    let architecture = std::env::var("PROCESSOR_ARCHITECTURE")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if architecture.contains("arm64") {
        "arm64"
    } else if architecture == "x86" || architecture == "i386" {
        "32bit"
    } else {
        "64bit"
    }
}

fn binary_cache_name(binary: &str) -> String {
    normalize_cache_token(Utf8Path::new(binary).file_name().unwrap_or(binary))
}

fn binary_cache_alias(binary: &str, alias: Option<&str>) -> String {
    match alias {
        Some(alias) => normalize_cache_token(alias),
        None => binary_cache_name(binary),
    }
}

fn shortcut_cache_name(shortcut: &str) -> String {
    Utf8Path::new(shortcut)
        .file_name()
        .unwrap_or(shortcut)
        .to_owned()
}

fn normalize_cache_token(token: &str) -> String {
    let token = Utf8Path::new(token).file_name().unwrap_or(token);
    match Utf8Path::new(token)
        .extension()
        .map(|value| value.to_ascii_lowercase())
    {
        Some(extension)
            if matches!(
                extension.as_str(),
                "exe" | "bat" | "cmd" | "ps1" | "jar" | "py"
            ) =>
        {
            Utf8Path::new(token).file_stem().unwrap_or(token).to_owned()
        }
        _ => token.to_owned(),
    }
}

fn string_or_array_values(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(value)) => vec![value.to_owned()],
        Some(Value::Array(entries)) => entries
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{cache_database_path, find_search_items};

    #[test]
    fn builds_and_queries_upstream_compatible_cache() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            r#"{
                "version":"1.2.3",
                "description":"Demo app",
                "bin":["demo.exe",["bin\\helper.ps1","demo-helper"]],
                "shortcuts":[["demo.exe","Demo App"]],
                "depends":["git"],
                "suggest":{"Editor":["vscode","vim"]}
            }"#,
        );
        fixture.write(
            "local",
            "buckets\\extras\\bucket\\other.json",
            r#"{"version":"2.0.0","bin":"other.exe"}"#,
        );

        let all = find_search_items(&fixture.config(), "").expect("cache query should succeed");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "demo");
        assert_eq!(all[0].version, "1.2.3");
        assert_eq!(all[0].bucket, "main");
        assert_eq!(all[0].binary.as_deref(), Some("demo | demo-helper"));

        let shortcut =
            find_search_items(&fixture.config(), "Demo App").expect("shortcut query should work");
        assert_eq!(shortcut.len(), 1);
        assert_eq!(shortcut[0].name, "demo");
    }

    #[test]
    fn rebuilds_cache_when_database_is_deleted() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            r#"{"version":"1.0.0","bin":"demo.exe"}"#,
        );

        let initial = find_search_items(&fixture.config(), "demo").expect("initial query");
        assert_eq!(initial[0].version, "1.0.0");
        assert!(cache_database_path(&fixture.config()).is_file());

        fs::remove_file(cache_database_path(&fixture.config()).as_std_path())
            .expect("cache database should be removed");

        let rebuilt = find_search_items(&fixture.config(), "demo").expect("rebuilt query");
        assert_eq!(rebuilt[0].version, "1.0.0");
    }

    struct Fixture {
        _temp: TempDir,
        local_root: Utf8PathBuf,
        global_root: Utf8PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let base = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            let local_root = base.join("local");
            let global_root = base.join("global");
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
