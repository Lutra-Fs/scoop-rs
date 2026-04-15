use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};

use crate::RuntimeConfig;

pub fn canonical_cache_path(
    config: &RuntimeConfig,
    app: &str,
    version: &str,
    url: &str,
) -> anyhow::Result<Utf8PathBuf> {
    Ok(config
        .cache_dir()
        .join(canonical_cache_file_name(app, version, url)?))
}

pub fn canonical_cache_file_name(app: &str, version: &str, url: &str) -> anyhow::Result<String> {
    let hash = short_url_hash(url);
    let extension = cache_extension(url);
    Ok(format!("{app}#{version}#{hash}{extension}"))
}

pub fn app_cache_prefix(app: &str) -> String {
    format!("{app}#")
}

pub fn app_version_cache_prefix(app: &str, version: &str) -> String {
    format!("{app}#{version}#")
}

pub fn parse_cache_entry_name(file_name: &str) -> Option<(&str, &str, &str)> {
    let mut parts = file_name.splitn(3, '#');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

fn short_url_hash(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .chars()
        .take(7)
        .collect()
}

fn cache_extension(url: &str) -> String {
    let stripped = url
        .split('#')
        .next()
        .unwrap_or(url)
        .split('?')
        .next()
        .unwrap_or(url);
    Utf8Path::new(stripped)
        .extension()
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        app_cache_prefix, app_version_cache_prefix, canonical_cache_file_name,
        parse_cache_entry_name,
    };

    #[test]
    fn canonical_cache_file_name_uses_short_url_hash_and_extension() {
        let file_name = canonical_cache_file_name(
            "demo",
            "1.2.3",
            "https://example.invalid/downloads/demo.zip?mirror=1",
        )
        .expect("cache name should render");

        assert_eq!(file_name, "demo#1.2.3#3d07c96.zip");
    }

    #[test]
    fn canonical_cache_file_name_omits_extension_when_missing() {
        let file_name = canonical_cache_file_name("demo", "1.2.3", "https://example.invalid/api")
            .expect("cache name should render");

        assert_eq!(file_name, "demo#1.2.3#dbec236");
    }

    #[test]
    fn parses_canonical_cache_entry_names() {
        let parsed = parse_cache_entry_name("demo#1.2.3#4c803f1.zip").expect("entry should parse");
        assert_eq!(parsed, ("demo", "1.2.3", "4c803f1.zip"));
    }

    #[test]
    fn cache_prefixes_match_app_and_version() {
        assert_eq!(app_cache_prefix("demo"), "demo#");
        assert_eq!(app_version_cache_prefix("demo", "1.2.3"), "demo#1.2.3#");
    }
}
