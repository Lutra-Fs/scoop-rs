use anyhow::Context;
use camino::Utf8Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Architecture<T> {
    #[serde(rename = "32bit", default, skip_serializing_if = "Option::is_none")]
    pub x86: Option<T>,
    #[serde(rename = "64bit", default, skip_serializing_if = "Option::is_none")]
    pub x64: Option<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arm64: Option<T>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum License {
    Identifier(String),
    Detailed {
        identifier: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Autoupdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<OneOrMany<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<OneOrMany<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoopManifest {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<OneOrMany<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<OneOrMany<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<OneOrMany<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends: Option<OneOrMany<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<Architecture<ScopedManifest>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoupdate: Option<Autoupdate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedManifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<OneOrMany<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<OneOrMany<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<OneOrMany<String>>,
}

impl ScoopManifest {
    pub fn from_path(path: &Utf8Path) -> anyhow::Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read manifest {}", path))?;
        serde_json::from_str(&source).with_context(|| format!("failed to parse manifest {}", path))
    }
}

#[cfg(test)]
mod tests {
    use super::{License, OneOrMany, ScoopManifest};

    #[test]
    fn parses_string_or_array_fields() {
        let manifest: ScoopManifest = serde_json::from_str(
            r#"{
                "version": "1.2.3",
                "url": ["https://example.invalid/a.zip", "https://example.invalid/b.zip"],
                "hash": "deadbeef",
                "bin": "tool.exe"
            }"#,
        )
        .expect("manifest should deserialize");

        assert!(matches!(manifest.url, Some(OneOrMany::Many(_))));
        assert!(matches!(manifest.hash, Some(OneOrMany::One(_))));
        assert!(matches!(manifest.bin, Some(OneOrMany::One(_))));
    }

    #[test]
    fn parses_license_variants() {
        let manifest: ScoopManifest = serde_json::from_str(
            r#"{
                "version": "1.2.3",
                "license": {
                    "identifier": "MIT",
                    "url": "https://opensource.org/license/mit"
                }
            }"#,
        )
        .expect("manifest should deserialize");

        assert!(matches!(manifest.license, Some(License::Detailed { .. })));
    }
}
