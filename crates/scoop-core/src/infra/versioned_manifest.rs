use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    process::Command,
};

use anyhow::{Context, bail};
use camino::{Utf8Path, Utf8PathBuf};
use regex::Regex;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{
    RuntimeConfig,
    infra::{cache::canonical_cache_path, hash::sha256_file, http::build_blocking_http_client},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionResolution {
    ExactCurrent,
    GitHistory,
    Autoupdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVersionedManifest {
    pub manifest: Value,
    pub source: VersionResolution,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct StoredVersionIndex {
    repo_head: String,
    revisions: BTreeMap<String, String>,
}

pub fn resolve_versioned_manifest(
    config: &RuntimeConfig,
    app: &str,
    manifest_path: &Utf8Path,
    current_manifest: &Value,
    requested_version: &str,
) -> anyhow::Result<Option<ResolvedVersionedManifest>> {
    if manifest_version(current_manifest) == Some(requested_version) {
        return Ok(Some(ResolvedVersionedManifest {
            manifest: current_manifest.clone(),
            source: VersionResolution::ExactCurrent,
        }));
    }

    if let Some(manifest) = resolve_from_git_history(config, manifest_path, requested_version)? {
        return Ok(Some(ResolvedVersionedManifest {
            manifest,
            source: VersionResolution::GitHistory,
        }));
    }

    if current_manifest.get("autoupdate").is_some()
        && let Some(manifest) =
            synthesize_from_autoupdate(config, app, current_manifest, requested_version)?
    {
        return Ok(Some(ResolvedVersionedManifest {
            manifest,
            source: VersionResolution::Autoupdate,
        }));
    }

    Ok(None)
}

fn resolve_from_git_history(
    config: &RuntimeConfig,
    manifest_path: &Utf8Path,
    requested_version: &str,
) -> anyhow::Result<Option<Value>> {
    let Some((repo_root, relative_git)) = git_repo_and_relative_path(manifest_path)? else {
        return Ok(None);
    };
    let repo_head = match git_stdout(&repo_root, &["rev-parse", "HEAD"]) {
        Ok(head) => head.trim().to_owned(),
        Err(_) => return Ok(None),
    };
    if repo_head.is_empty() {
        return Ok(None);
    }

    let index = load_or_build_index(config, &repo_root, &relative_git, &repo_head)?;
    let Some(revision) = index.revisions.get(requested_version) else {
        return Ok(None);
    };

    load_manifest_at_revision(&repo_root, &relative_git, revision)
}

fn load_or_build_index(
    config: &RuntimeConfig,
    repo_root: &Utf8Path,
    relative_git: &str,
    repo_head: &str,
) -> anyhow::Result<StoredVersionIndex> {
    let path = index_path(config, repo_root, relative_git);
    if let Ok(source) = fs::read_to_string(&path)
        && let Ok(index) = serde_json::from_str::<StoredVersionIndex>(&source)
        && index.repo_head == repo_head
    {
        return Ok(index);
    }

    let mut revisions = BTreeMap::new();
    let history = git_stdout(
        repo_root,
        &["log", "--follow", "--format=%H", "--", relative_git],
    )?;
    for revision in history
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Some(manifest) = load_manifest_at_revision(repo_root, relative_git, revision)? else {
            continue;
        };
        let Some(version) = manifest_version(&manifest) else {
            continue;
        };
        revisions
            .entry(version.to_owned())
            .or_insert_with(|| revision.to_owned());
    }

    let index = StoredVersionIndex {
        repo_head: repo_head.to_owned(),
        revisions,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create version index directory {}", parent))?;
    }
    fs::write(
        &path,
        serde_json::to_string_pretty(&index).context("failed to serialize version index")?,
    )
    .with_context(|| format!("failed to write version index {}", path))?;
    Ok(index)
}

fn index_path(config: &RuntimeConfig, repo_root: &Utf8Path, relative_git: &str) -> Utf8PathBuf {
    let mut digest = Sha256::new();
    digest.update(repo_root.as_str().as_bytes());
    digest.update(b"\n");
    digest.update(relative_git.as_bytes());
    let key = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    config
        .cache_dir()
        .join("version-index")
        .join(format!("{key}.json"))
}

fn git_repo_and_relative_path(path: &Utf8Path) -> anyhow::Result<Option<(Utf8PathBuf, String)>> {
    let Some(repo_root) = path
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .map(Utf8Path::to_path_buf)
    else {
        return Ok(None);
    };
    let relative = path
        .strip_prefix(&repo_root)
        .map_err(|_| anyhow::anyhow!("manifest should be inside its bucket repository"))?;
    Ok(Some((repo_root, relative.as_str().replace('\\', "/"))))
}

fn load_manifest_at_revision(
    repo_root: &Utf8Path,
    relative_git: &str,
    revision: &str,
) -> anyhow::Result<Option<Value>> {
    let spec = format!("{revision}:{relative_git}");
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["show", &spec])
        .output()
        .with_context(|| {
            format!(
                "failed to load historical manifest {} at {}",
                relative_git, revision
            )
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    let manifest = serde_json::from_slice(&output.stdout).ok();
    Ok(manifest)
}

fn synthesize_from_autoupdate(
    config: &RuntimeConfig,
    app: &str,
    current_manifest: &Value,
    requested_version: &str,
) -> anyhow::Result<Option<Value>> {
    let Some(autoupdate) = current_manifest
        .get("autoupdate")
        .and_then(Value::as_object)
    else {
        return Ok(None);
    };

    let substitutions = version_substitutions(requested_version);
    let mut manifest = current_manifest.clone();
    let properties = updated_properties(autoupdate);

    for property in properties {
        if property == "hash" {
            update_hash_property(config, app, requested_version, &mut manifest, autoupdate)?;
            continue;
        }
        update_manifest_property(&mut manifest, autoupdate, &property, &substitutions);
    }

    manifest["version"] = Value::String(requested_version.to_owned());
    Ok(Some(manifest))
}

fn updated_properties(autoupdate: &Map<String, Value>) -> BTreeSet<String> {
    let mut properties = autoupdate
        .keys()
        .filter(|key| key.as_str() != "architecture")
        .cloned()
        .collect::<BTreeSet<_>>();
    if let Some(architecture) = autoupdate.get("architecture").and_then(Value::as_object) {
        for scoped in architecture.values().filter_map(Value::as_object) {
            properties.extend(scoped.keys().cloned());
        }
    }
    if properties.contains("url") {
        properties.insert(String::from("hash"));
    }
    properties
}

fn update_manifest_property(
    manifest: &mut Value,
    autoupdate: &Map<String, Value>,
    property: &str,
    substitutions: &[(String, String)],
) {
    if let Some(current) = manifest.get(property).cloned()
        && current != Value::Null
        && let Some(update_value) = autoupdate.get(property)
    {
        manifest[property] = substitute_value(update_value, substitutions);
    }

    let Some(architecture) = manifest
        .get_mut("architecture")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    for (arch, scoped_manifest) in architecture {
        let Some(scoped_manifest) = scoped_manifest.as_object_mut() else {
            continue;
        };
        if !scoped_manifest.contains_key(property) {
            continue;
        }
        let Some(update_value) = arch_specific_autoupdate_value(autoupdate, arch, property) else {
            continue;
        };
        scoped_manifest.insert(
            property.to_owned(),
            substitute_value(update_value, substitutions),
        );
    }
}

fn update_hash_property(
    config: &RuntimeConfig,
    app: &str,
    requested_version: &str,
    manifest: &mut Value,
    autoupdate: &Map<String, Value>,
) -> anyhow::Result<()> {
    let client = build_blocking_http_client()?;
    if manifest.get("hash").is_some() {
        let urls = value_to_strings(root_autoupdate_url(autoupdate))
            .map(|value| substitute_strings(value, &version_substitutions(requested_version)))
            .unwrap_or_default();
        if !urls.is_empty() {
            manifest["hash"] = serialize_hashes(compute_hashes(
                config,
                &client,
                app,
                requested_version,
                &urls,
                autoupdate.get("hash"),
                &version_substitutions(requested_version),
            )?);
        }
    }

    let Some(architecture) = manifest
        .get_mut("architecture")
        .and_then(Value::as_object_mut)
    else {
        return Ok(());
    };
    for (arch, scoped_manifest) in architecture {
        let Some(scoped_manifest) = scoped_manifest.as_object_mut() else {
            continue;
        };
        if !scoped_manifest.contains_key("hash") {
            continue;
        }
        let urls = value_to_strings(arch_specific_autoupdate_value(autoupdate, arch, "url"))
            .map(|value| substitute_strings(value, &version_substitutions(requested_version)))
            .unwrap_or_default();
        if urls.is_empty() {
            continue;
        }
        let hashes = compute_hashes(
            config,
            &client,
            app,
            requested_version,
            &urls,
            arch_specific_autoupdate_value(autoupdate, arch, "hash"),
            &version_substitutions(requested_version),
        )?;
        scoped_manifest.insert(String::from("hash"), serialize_hashes(hashes));
    }
    Ok(())
}

fn root_autoupdate_url(autoupdate: &Map<String, Value>) -> Option<&Value> {
    autoupdate.get("url")
}

fn compute_hashes(
    config: &RuntimeConfig,
    client: &Client,
    app: &str,
    requested_version: &str,
    urls: &[String],
    configured_hashes: Option<&Value>,
    substitutions: &[(String, String)],
) -> anyhow::Result<Vec<String>> {
    if let Some(hashes) = direct_hash_values(configured_hashes, substitutions, urls.len()) {
        return Ok(hashes);
    }

    urls.iter()
        .map(|url| {
            let path = canonical_cache_path(config, app, requested_version, url)?;
            if !path.is_file() {
                fetch_to_path(client, url, &path)?;
            }
            sha256_file(&path)
        })
        .collect()
}

fn direct_hash_values(
    configured_hashes: Option<&Value>,
    substitutions: &[(String, String)],
    url_count: usize,
) -> Option<Vec<String>> {
    let value = configured_hashes?;
    let mut hashes = match value {
        Value::String(hash) => vec![substitute_string(hash, substitutions)],
        Value::Array(values) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|value| substitute_string(value, substitutions))
            })
            .collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    if hashes.is_empty() {
        return None;
    }
    while hashes.len() < url_count {
        let repeated = hashes.last().cloned()?;
        hashes.push(repeated);
    }
    hashes.truncate(url_count);
    Some(hashes)
}

fn substitute_value(value: &Value, substitutions: &[(String, String)]) -> Value {
    match value {
        Value::String(value) => Value::String(substitute_string(value, substitutions)),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| substitute_value(value, substitutions))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), substitute_value(value, substitutions)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn substitute_string(value: &str, substitutions: &[(String, String)]) -> String {
    let mut substituted = value.to_owned();
    let mut ordered = substitutions.to_vec();
    ordered.sort_by(|left, right| right.0.len().cmp(&left.0.len()));
    for (token, replacement) in ordered {
        substituted = substituted.replace(&token, &replacement);
    }
    substituted
}

fn substitute_strings(values: Vec<String>, substitutions: &[(String, String)]) -> Vec<String> {
    values
        .into_iter()
        .map(|value| substitute_string(&value, substitutions))
        .collect()
}

fn value_to_strings(value: Option<&Value>) -> Option<Vec<String>> {
    match value? {
        Value::String(value) => Some(vec![value.clone()]),
        Value::Array(values) => values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect(),
        _ => None,
    }
}

fn arch_specific_autoupdate_value<'a>(
    autoupdate: &'a Map<String, Value>,
    arch: &str,
    property: &str,
) -> Option<&'a Value> {
    autoupdate
        .get("architecture")
        .and_then(Value::as_object)
        .and_then(|architecture| architecture.get(arch))
        .and_then(Value::as_object)
        .and_then(|scoped| scoped.get(property))
        .or_else(|| autoupdate.get(property))
}

fn version_substitutions(version: &str) -> Vec<(String, String)> {
    let first_part = version.split('-').next().unwrap_or(version);
    let last_part = version.rsplit('-').next().unwrap_or(version);
    let mut substitutions = vec![
        (String::from("$version"), version.to_owned()),
        (
            String::from("$dotVersion"),
            replace_version_separators(version, "."),
        ),
        (
            String::from("$underscoreVersion"),
            replace_version_separators(version, "_"),
        ),
        (
            String::from("$dashVersion"),
            replace_version_separators(version, "-"),
        ),
        (
            String::from("$cleanVersion"),
            version
                .chars()
                .filter(|character| !matches!(character, '.' | '_' | '-'))
                .collect(),
        ),
        (
            String::from("$majorVersion"),
            first_part.split('.').next().unwrap_or_default().to_owned(),
        ),
        (
            String::from("$minorVersion"),
            first_part.split('.').nth(1).unwrap_or_default().to_owned(),
        ),
        (
            String::from("$patchVersion"),
            first_part.split('.').nth(2).unwrap_or_default().to_owned(),
        ),
        (
            String::from("$buildVersion"),
            first_part.split('.').nth(3).unwrap_or_default().to_owned(),
        ),
        (String::from("$preReleaseVersion"), last_part.to_owned()),
    ];
    if let Ok(pattern) = Regex::new(r"^(?P<head>\d+\.\d+(?:\.\d+)?)(?P<tail>.*)$")
        && let Some(captures) = pattern.captures(version)
    {
        substitutions.push((
            String::from("$matchHead"),
            captures
                .name("head")
                .map(|value| value.as_str().to_owned())
                .unwrap_or_default(),
        ));
        substitutions.push((
            String::from("$matchTail"),
            captures
                .name("tail")
                .map(|value| value.as_str().to_owned())
                .unwrap_or_default(),
        ));
    }
    substitutions
}

fn replace_version_separators(version: &str, replacement: &str) -> String {
    version
        .chars()
        .map(|character| match character {
            '.' | '_' | '-' => replacement,
            _ => "",
        })
        .zip(version.chars())
        .map(|(mapped, original)| {
            if mapped.is_empty() {
                original.to_string()
            } else {
                mapped.to_owned()
            }
        })
        .collect()
}

fn serialize_hashes(hashes: Vec<String>) -> Value {
    match hashes.as_slice() {
        [single] => Value::String(single.clone()),
        _ => Value::Array(hashes.into_iter().map(Value::String).collect()),
    }
}

fn manifest_version(manifest: &Value) -> Option<&str> {
    manifest.get("version").and_then(Value::as_str)
}

fn git_stdout(repopath: &Utf8Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .current_dir(repopath)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git in {}", repopath))?;
    if !output.status.success() {
        bail!(
            "git {:?} failed in {}\nstdout: {}\nstderr: {}",
            args,
            repopath,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn fetch_to_path(client: &Client, url: &str, path: &Utf8Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent))?;
    }

    if Utf8Path::new(url).is_file() {
        fs::copy(url, path).with_context(|| format!("failed to copy {} to {}", url, path))?;
        return Ok(());
    }

    let mut response = client
        .get(url)
        .send()
        .with_context(|| format!("failed to download {url}"))?
        .error_for_status()
        .with_context(|| format!("failed to download {url}"))?;
    let mut file =
        fs::File::create(path).with_context(|| format!("failed to create download {}", path))?;
    io::copy(&mut response, &mut file)
        .with_context(|| format!("failed to write download {}", path))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{VersionResolution, resolve_versioned_manifest};

    #[test]
    fn resolves_versioned_manifest_from_git_history() {
        let fixture = Fixture::new();
        fixture.commit_manifest(
            "bucket/demo.json",
            r#"{"version":"1.0.0","url":"https://example.invalid/demo-1.zip","hash":"sha256:1"}"#,
            "seed",
        );
        fixture.commit_manifest(
            "bucket/demo.json",
            r#"{"version":"2.0.0","url":"https://example.invalid/demo-2.zip","hash":"sha256:2"}"#,
            "upgrade",
        );
        let path = fixture.repo_root.join("bucket/demo.json");
        let current = fs::read_to_string(&path).expect("manifest should exist");
        let current = serde_json::from_str(&current).expect("manifest should parse");

        let resolved =
            resolve_versioned_manifest(&fixture.config(), "demo", &path, &current, "1.0.0")
                .expect("lookup should succeed")
                .expect("manifest should resolve");

        assert_eq!(resolved.source, VersionResolution::GitHistory);
        assert_eq!(resolved.manifest["version"], "1.0.0");
    }

    #[test]
    fn synthesizes_versioned_manifest_from_autoupdate() {
        let fixture = Fixture::new();
        let archive = fixture.write_file("payload/demo-9.8.7.zip", b"demo");
        let path = fixture.repo_root.join("bucket/demo.json");
        fixture.commit_manifest(
            "bucket/demo.json",
            &format!(
                r#"{{
                    "version":"1.0.0",
                    "url":"https://example.invalid/demo-1.0.0.zip",
                    "hash":"sha256:old",
                    "bin":"demo.exe",
                    "autoupdate": {{
                        "url": "{}",
                        "bin": "demo.exe"
                    }}
                }}"#,
                archive.as_str().replace('\\', "\\\\")
            ),
            "seed",
        );
        let current = fs::read_to_string(&path).expect("manifest should exist");
        let current = serde_json::from_str(&current).expect("manifest should parse");

        let resolved =
            resolve_versioned_manifest(&fixture.config(), "demo", &path, &current, "9.8.7")
                .expect("lookup should succeed")
                .expect("manifest should resolve");

        assert_eq!(resolved.source, VersionResolution::Autoupdate);
        assert_eq!(resolved.manifest["version"], "9.8.7");
        assert_eq!(resolved.manifest["url"], archive.as_str());
        assert!(resolved.manifest["hash"].as_str().is_some());
    }

    struct Fixture {
        _temp: TempDir,
        root: Utf8PathBuf,
        repo_root: Utf8PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            let repo_root = root.join("local/buckets/main");
            fs::create_dir_all(&repo_root).expect("repo root should exist");
            run_git(&repo_root, &["init"]);
            run_git(&repo_root, &["config", "commit.gpgsign", "false"]);
            run_git(&repo_root, &["config", "user.name", "Codex"]);
            run_git(
                &repo_root,
                &["config", "user.email", "codex@example.invalid"],
            );
            Self {
                _temp: temp,
                root,
                repo_root,
            }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::new(self.root.join("local"), self.root.join("global"))
        }

        fn commit_manifest(&self, relative: &str, content: &str, message: &str) {
            let path = self.repo_root.join(relative);
            fs::create_dir_all(path.parent().expect("manifest should have a parent"))
                .expect("manifest parent should exist");
            fs::write(&path, content).expect("manifest should be written");
            run_git(&self.repo_root, &["add", relative]);
            run_git(&self.repo_root, &["commit", "-m", message]);
        }

        fn write_file(&self, relative: &str, bytes: &[u8]) -> Utf8PathBuf {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().expect("file should have parent"))
                .expect("file parent should exist");
            fs::write(&path, bytes).expect("file should be written");
            path
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
}
