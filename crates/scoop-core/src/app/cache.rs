use std::fs;

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use regex::RegexBuilder;

use crate::{RuntimeConfig, infra::cache::parse_cache_entry_name};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    pub file_name: String,
    pub name: String,
    pub version: String,
    pub length: u64,
    pub path: Utf8PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheReport {
    pub entries: Vec<CacheEntry>,
    pub total_length: u64,
}

pub fn show_cache(config: &RuntimeConfig, apps: &[String]) -> anyhow::Result<CacheReport> {
    cache_report(config.cache_dir(), apps)
}

pub fn remove_cache(
    config: &RuntimeConfig,
    apps: &[String],
    all: bool,
) -> anyhow::Result<CacheReport> {
    let report = cache_report(config.cache_dir(), if all { &[] } else { apps })?;
    for entry in &report.entries {
        fs::remove_file(&entry.path)
            .with_context(|| format!("failed to remove cached file {}", entry.path))?;

        let note_path = config.cache_dir().join(format!("{}.txt", entry.name));
        if note_path.is_file() {
            let _ = fs::remove_file(note_path);
        }
    }
    Ok(report)
}

fn cache_report(cache_dir: &Utf8Path, apps: &[String]) -> anyhow::Result<CacheReport> {
    if !cache_dir.is_dir() {
        return Ok(CacheReport {
            entries: Vec::new(),
            total_length: 0,
        });
    }

    let matcher = cache_matcher(apps)?;
    let mut entries = fs::read_dir(cache_dir)
        .with_context(|| format!("failed to read cache directory {}", cache_dir))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then_some((entry, metadata.len()))
        })
        .filter_map(|(entry, length)| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            matcher
                .as_ref()
                .is_none_or(|matcher| matcher.is_match(&name))
                .then(|| parse_cache_entry(path, name, length))
                .flatten()
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.file_name.cmp(&right.file_name));

    let total_length = entries.iter().map(|entry| entry.length).sum();
    Ok(CacheReport {
        entries,
        total_length,
    })
}

fn cache_matcher(apps: &[String]) -> anyhow::Result<Option<regex::Regex>> {
    if apps.is_empty() {
        return Ok(None);
    }
    let pattern = format!("^({})#", apps.join("|"));
    RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .build()
        .map(Some)
        .with_context(|| format!("invalid cache app pattern: {pattern}"))
}

fn parse_cache_entry(path: Utf8PathBuf, file_name: String, length: u64) -> Option<CacheEntry> {
    let (name, version, _) = parse_cache_entry_name(&file_name)?;
    let name = name.to_owned();
    let version = version.to_owned();
    Some(CacheEntry {
        file_name,
        name,
        version,
        length,
        path,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{remove_cache, show_cache};

    #[test]
    fn shows_matching_cached_files() {
        let fixture = CacheFixture::new();
        fixture.write("demo#1.2.3#demo.zip", b"demo");
        fixture.write("other#2.0.0#other.zip", b"other");

        let report =
            show_cache(&fixture.config(), &[String::from("demo")]).expect("cache should load");

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].name, "demo");
        assert_eq!(report.entries[0].version, "1.2.3");
        assert_eq!(report.total_length, 4);
    }

    #[test]
    fn remove_cache_deletes_matching_files() {
        let fixture = CacheFixture::new();
        fixture.write("demo#1.2.3#demo.zip", b"demo");
        fixture.write("demo.txt", b"note");
        fixture.write("other#2.0.0#other.zip", b"other");

        let report =
            remove_cache(&fixture.config(), &[String::from("demo")], false).expect("remove works");

        assert_eq!(report.entries.len(), 1);
        assert!(!fixture.cache_dir.join("demo#1.2.3#demo.zip").is_file());
        assert!(!fixture.cache_dir.join("demo.txt").is_file());
        assert!(fixture.cache_dir.join("other#2.0.0#other.zip").is_file());
    }

    struct CacheFixture {
        _temp: TempDir,
        cache_dir: Utf8PathBuf,
        config: RuntimeConfig,
    }

    impl CacheFixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            let local_root = root.join("local");
            let global_root = root.join("global");
            let cache_dir = local_root.join("cache");
            fs::create_dir_all(&cache_dir).expect("cache dir should exist");
            fs::create_dir_all(&global_root).expect("global root should exist");

            Self {
                _temp: temp,
                cache_dir: cache_dir.clone(),
                config: RuntimeConfig::with_cache(local_root, global_root, cache_dir),
            }
        }

        fn config(&self) -> RuntimeConfig {
            self.config.clone()
        }

        fn write(&self, name: &str, content: &[u8]) {
            fs::write(self.cache_dir.join(name), content).expect("cache file should exist");
        }
    }
}
