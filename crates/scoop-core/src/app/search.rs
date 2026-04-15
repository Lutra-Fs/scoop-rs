use std::fs;

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use regex::{Regex, RegexBuilder};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    RuntimeConfig,
    infra::{
        buckets::{bucket_manifests_dir, known_bucket_repos, local_bucket_names},
        sqlite_cache::find_search_items,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub name: String,
    pub version: String,
    pub source: String,
    pub binaries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSearchResult {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchMode {
    Regex,
    SqliteCache,
}

pub fn compile_search_query(query: &str) -> anyhow::Result<Regex> {
    RegexBuilder::new(query)
        .case_insensitive(true)
        .build()
        .with_context(|| format!("Invalid regular expression: {query}"))
}

pub fn search_local_buckets(
    config: &RuntimeConfig,
    query: Option<&Regex>,
) -> anyhow::Result<Vec<SearchResult>> {
    let mut results = Vec::new();
    for bucket_name in local_bucket_names(config)? {
        let bucket_dir = bucket_manifests_dir(config, Some(&bucket_name));
        search_bucket(bucket_dir.as_std_path(), &bucket_name, query, &mut results)?;
    }
    Ok(results)
}

pub fn search_cached_buckets(
    config: &RuntimeConfig,
    query: Option<&str>,
) -> anyhow::Result<Vec<SearchResult>> {
    let mut results = find_search_items(config, query.unwrap_or_default())?
        .into_iter()
        .map(|row| SearchResult {
            name: row.name,
            version: row.version,
            source: row.bucket,
            binaries: row
                .binary
                .map(|value| {
                    value
                        .split(" | ")
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    results.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source.cmp(&right.source))
    });
    Ok(results)
}

pub fn search_remote_buckets(
    config: &RuntimeConfig,
    client: &Client,
    query: &Regex,
) -> anyhow::Result<Vec<RemoteSearchResult>> {
    let local_buckets = local_bucket_names(config)?
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let mut results = Vec::new();
    for (bucket, repo) in known_bucket_repos(config)? {
        if local_buckets.contains(&bucket) {
            continue;
        }
        let Some(api_url) = github_tree_api_url(&repo) else {
            continue;
        };
        let response = match client.get(api_url).send() {
            Ok(response) => response,
            Err(_) => continue,
        };
        if response.status().as_u16() == 403
            && response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == "0")
        {
            return Ok(Vec::new());
        }
        let tree = match response.error_for_status() {
            Ok(response) => response.json::<GitTreeResponse>(),
            Err(_) => continue,
        };
        let Ok(tree) = tree else {
            continue;
        };
        for name in matching_remote_manifest_names(&tree.tree, query) {
            results.push(RemoteSearchResult {
                name,
                source: bucket.clone(),
            });
        }
    }
    Ok(results)
}

pub fn search_remote_buckets_partial(
    config: &RuntimeConfig,
    client: &Client,
    query: &str,
) -> anyhow::Result<Vec<RemoteSearchResult>> {
    let local_buckets = local_bucket_names(config)?
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let mut results = Vec::new();
    for (bucket, repo) in known_bucket_repos(config)? {
        if local_buckets.contains(&bucket) {
            continue;
        }
        let Some(api_url) = github_tree_api_url(&repo) else {
            continue;
        };
        let response = match client.get(api_url).send() {
            Ok(response) => response,
            Err(_) => continue,
        };
        if response.status().as_u16() == 403
            && response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == "0")
        {
            return Ok(Vec::new());
        }
        let tree = match response.error_for_status() {
            Ok(response) => response.json::<GitTreeResponse>(),
            Err(_) => continue,
        };
        let Ok(tree) = tree else {
            continue;
        };
        for name in matching_remote_manifest_names_partial(&tree.tree, query) {
            results.push(RemoteSearchResult {
                name,
                source: bucket.clone(),
            });
        }
    }
    Ok(results)
}

fn search_bucket(
    bucket_root: &std::path::Path,
    bucket_name: &str,
    query: Option<&Regex>,
    results: &mut Vec<SearchResult>,
) -> anyhow::Result<()> {
    for manifest_path in walk_json_manifests(bucket_root)? {
        let Ok(path) = Utf8PathBuf::from_path_buf(manifest_path.clone()) else {
            continue;
        };
        let source = match fs::read_to_string(&manifest_path) {
            Ok(source) => source,
            Err(_) => continue,
        };
        let manifest: Value = match serde_json::from_str(&source) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        let Some(name) = path.file_stem().map(str::to_owned) else {
            continue;
        };
        let version = manifest
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();

        match query {
            None => results.push(SearchResult {
                name,
                version,
                source: bucket_name.to_owned(),
                binaries: Vec::new(),
            }),
            Some(pattern) if pattern.is_match(&name) => results.push(SearchResult {
                name,
                version,
                source: bucket_name.to_owned(),
                binaries: Vec::new(),
            }),
            Some(pattern) => {
                let binaries = matching_binaries(&manifest, pattern);
                if !binaries.is_empty() {
                    results.push(SearchResult {
                        name,
                        version,
                        source: bucket_name.to_owned(),
                        binaries,
                    });
                }
            }
        }
    }

    Ok(())
}

fn walk_json_manifests(root: &std::path::Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
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
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(kind)
                    if kind.is_file()
                        && path
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("json")) =>
                {
                    files.push(path);
                }
                _ => {}
            }
        }
    }

    files.sort();
    Ok(files)
}

fn matching_binaries(manifest: &Value, pattern: &Regex) -> Vec<String> {
    match manifest.get("bin") {
        Some(Value::String(binary)) => match_binary_entry(binary, None, pattern)
            .into_iter()
            .collect(),
        Some(Value::Array(entries)) => entries
            .iter()
            .filter_map(|entry| match entry {
                Value::String(binary) => match_binary_entry(binary, None, pattern),
                Value::Array(parts) if !parts.is_empty() => {
                    let binary = parts.first().and_then(Value::as_str)?;
                    let alias = parts.get(1).and_then(Value::as_str);
                    match_binary_entry(binary, alias, pattern)
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn match_binary_entry(binary: &str, alias: Option<&str>, pattern: &Regex) -> Option<String> {
    let binary_name = Utf8Path::new(binary).file_name().unwrap_or(binary);
    let binary_stem = Utf8Path::new(binary_name)
        .file_stem()
        .unwrap_or(binary_name);

    if pattern.is_match(binary_stem) {
        return Some(binary_name.to_owned());
    }
    if let Some(alias) = alias
        && pattern.is_match(alias)
    {
        return Some(alias.to_owned());
    }
    None
}

fn github_tree_api_url(repo: &str) -> Option<String> {
    let api_base = std::env::var("SCOOP_RS_GITHUB_API_BASE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| String::from("https://api.github.com/repos"));
    let normalized = repo.trim_end_matches('/');
    let normalized = normalized.strip_suffix(".git").unwrap_or(normalized);
    let normalized = normalized
        .strip_prefix("https://github.com/")
        .or_else(|| normalized.strip_prefix("http://github.com/"))
        .or_else(|| normalized.strip_prefix("git@github.com:"))?;
    let (user, repo) = normalized.split_once('/')?;
    Some(format!(
        "{api_base}/{user}/{repo}/git/trees/HEAD?recursive=1"
    ))
}

fn matching_remote_manifest_names(entries: &[GitTreeEntry], query: &Regex) -> Vec<String> {
    let mut matches = entries
        .iter()
        .filter_map(|entry| entry.path.strip_prefix("bucket/"))
        .filter_map(|path| path.strip_suffix(".json"))
        .map(str::to_owned)
        .filter(|name| query.is_match(name))
        .collect::<Vec<_>>();
    matches.sort();
    matches
}

fn matching_remote_manifest_names_partial(entries: &[GitTreeEntry], query: &str) -> Vec<String> {
    let query = query.to_ascii_lowercase();
    let mut matches = entries
        .iter()
        .filter_map(|entry| entry.path.strip_prefix("bucket/"))
        .filter_map(|path| path.strip_suffix(".json"))
        .filter(|name| query.is_empty() || name.to_ascii_lowercase().contains(&query))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    matches.sort();
    matches
}

#[derive(Debug, Clone, Deserialize)]
struct GitTreeResponse {
    #[serde(default)]
    tree: Vec<GitTreeEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitTreeEntry {
    path: String,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{
        GitTreeEntry, compile_search_query, matching_remote_manifest_names,
        matching_remote_manifest_names_partial, search_cached_buckets, search_local_buckets,
    };

    #[test]
    fn searches_names_and_binaries_in_local_buckets() {
        let fixture = Fixture::new();
        fixture.write(
            "buckets\\main\\bucket\\demo.json",
            r#"{"version":"1.2.3","bin":["demo.exe",["bin\\helper.exe","demo-helper"]]}"#,
        );
        fixture.write(
            "buckets\\extras\\bucket\\other.json",
            r#"{"version":"2.0.0","bin":"other.exe"}"#,
        );

        let all = search_local_buckets(&fixture.config(), None).expect("search should succeed");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "other");
        assert_eq!(all[1].name, "demo");

        let query = compile_search_query("helper").expect("query should compile");
        let helper =
            search_local_buckets(&fixture.config(), Some(&query)).expect("search should succeed");
        assert_eq!(helper.len(), 1);
        assert_eq!(helper[0].name, "demo");
        assert_eq!(helper[0].binaries, vec![String::from("helper.exe")]);
    }

    #[test]
    fn matches_remote_bucket_manifest_names_with_regex() {
        let query = compile_search_query("git").expect("query should compile");
        let matches = matching_remote_manifest_names(
            &[
                GitTreeEntry {
                    path: String::from("bucket/git.json"),
                },
                GitTreeEntry {
                    path: String::from("bucket/git-lfs.json"),
                },
                GitTreeEntry {
                    path: String::from("README.md"),
                },
            ],
            &query,
        );

        assert_eq!(matches, vec!["git", "git-lfs"]);
    }

    #[test]
    fn searches_local_buckets_through_sqlite_cache_using_partial_matching() {
        let fixture = Fixture::new();
        fixture.write(
            "buckets\\main\\bucket\\demo.json",
            r#"{"version":"1.2.3","bin":["demo.exe",["bin\\helper.ps1","demo-helper"]],"shortcuts":[["demo.exe","Demo App"]]}"#,
        );

        let helper =
            search_cached_buckets(&fixture.config(), Some("helper")).expect("search should work");
        assert_eq!(helper.len(), 1);
        assert_eq!(helper[0].name, "demo");
        assert_eq!(
            helper[0].binaries,
            vec![String::from("demo"), String::from("demo-helper")]
        );

        let shortcut =
            search_cached_buckets(&fixture.config(), Some("Demo App")).expect("search should work");
        assert_eq!(shortcut.len(), 1);
        assert_eq!(shortcut[0].name, "demo");
    }

    #[test]
    fn matches_remote_bucket_manifest_names_with_partial_matching() {
        let matches = matching_remote_manifest_names_partial(
            &[
                GitTreeEntry {
                    path: String::from("bucket/git.json"),
                },
                GitTreeEntry {
                    path: String::from("bucket/git-lfs.json"),
                },
                GitTreeEntry {
                    path: String::from("README.md"),
                },
            ],
            "git",
        );

        assert_eq!(matches, vec!["git", "git-lfs"]);
    }

    struct Fixture {
        _temp: TempDir,
        local_root: Utf8PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let local_root = Utf8PathBuf::from_path_buf(temp.path().join("local"))
                .expect("temp path should be valid UTF-8");
            fs::create_dir_all(&local_root).expect("local root should exist");

            Self {
                _temp: temp,
                local_root,
            }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::new(self.local_root.clone(), self.local_root.join("global"))
        }

        fn write(&self, relative_path: &str, content: &str) {
            let path = self.local_root.join(relative_path);
            fs::create_dir_all(path.parent().expect("fixture file should have a parent"))
                .expect("fixture parent should exist");
            fs::write(path, content).expect("fixture file should be written");
        }
    }
}
