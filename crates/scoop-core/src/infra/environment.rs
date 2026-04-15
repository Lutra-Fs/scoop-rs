use std::{collections::BTreeMap, fs, path::PathBuf};

use super::config::{IsolatedPathSetting, ScoopSettings};
#[cfg(windows)]
use super::windows::env::broadcast_environment_change;
use anyhow::{Context, bail};
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use winreg::{
    HKCU, HKLM, RegValue,
    enums::{KEY_READ, REG_EXPAND_SZ, REG_SZ},
    types::FromRegValue,
};

const MOCK_ENV_STORE: &str = "SCOOP_RS_ENV_STORE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvScope {
    User,
    System,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MockEnvStore {
    #[serde(default)]
    user: BTreeMap<String, String>,
    #[serde(default)]
    system: BTreeMap<String, String>,
}

pub fn scoop_path_env_var(settings: &ScoopSettings) -> String {
    match &settings.use_isolated_path {
        Some(IsolatedPathSetting::Name(value)) if !value.is_empty() => value.to_ascii_uppercase(),
        Some(IsolatedPathSetting::Enabled(true)) => String::from("SCOOP_PATH"),
        _ => String::from("PATH"),
    }
}

pub fn get_env_var(scope: EnvScope, name: &str) -> anyhow::Result<Option<String>> {
    if let Some(path) = mock_store_path()? {
        let store = load_mock_store(&path)?;
        return Ok(match scope {
            EnvScope::User => store.user.get(name).cloned(),
            EnvScope::System => store.system.get(name).cloned(),
        });
    }

    #[cfg(windows)]
    {
        native_get_env_var(scope, name)
    }
    #[cfg(not(windows))]
    {
        Ok(std::env::var(name).ok())
    }
}

pub fn set_env_var(scope: EnvScope, name: &str, value: Option<&str>) -> anyhow::Result<()> {
    if let Some(path) = mock_store_path()? {
        let mut store = load_mock_store(&path)?;
        let target = match scope {
            EnvScope::User => &mut store.user,
            EnvScope::System => &mut store.system,
        };
        match value {
            Some(value) if !value.is_empty() => {
                target.insert(name.to_owned(), value.to_owned());
            }
            _ => {
                target.remove(name);
            }
        }
        save_mock_store(&path, &store)?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        native_set_env_var(scope, name, value)
    }
    #[cfg(not(windows))]
    {
        match value {
            Some(value) if !value.is_empty() => std::env::set_var(name, value),
            _ => std::env::remove_var(name),
        }
        Ok(())
    }
}

pub fn add_path(
    scope: EnvScope,
    target_env_var: &str,
    paths: &[String],
    force: bool,
) -> anyhow::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }

    let current = get_env_var(scope, target_env_var)?;
    let (in_path, stripped) = split_path_like_env_var(paths, current.as_deref());
    if in_path.is_empty() || force {
        let mut values = paths.to_vec();
        values.extend(stripped);
        set_env_var(scope, target_env_var, Some(&values.join(";")))?;
    }
    Ok(())
}

pub fn remove_path(
    scope: EnvScope,
    target_env_var: &str,
    paths: &[String],
) -> anyhow::Result<Vec<String>> {
    let current = get_env_var(scope, target_env_var)?;
    let (removed, stripped) = split_path_like_env_var(paths, current.as_deref());
    if !removed.is_empty() {
        let joined = stripped.join(";");
        set_env_var(
            scope,
            target_env_var,
            (!joined.is_empty()).then_some(joined.as_str()),
        )?;
    }
    Ok(removed)
}

pub fn split_path_like_env_var(
    patterns: &[String],
    path: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let Some(path) = path else {
        return (Vec::new(), Vec::new());
    };
    let mut remaining: Vec<String> = path
        .split(';')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();
    let mut in_path = Vec::new();
    for pattern in patterns {
        let mut rest = Vec::new();
        for segment in remaining {
            if wildcard_like(&segment, pattern) {
                in_path.push(segment);
            } else {
                rest.push(segment);
            }
        }
        remaining = rest;
    }
    (in_path, remaining)
}

fn wildcard_like(value: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        value.starts_with(prefix)
    } else {
        value.eq_ignore_ascii_case(pattern)
    }
}

fn mock_store_path() -> anyhow::Result<Option<Utf8PathBuf>> {
    let Some(path) = std::env::var_os(MOCK_ENV_STORE) else {
        return Ok(None);
    };
    Utf8PathBuf::from_path_buf(PathBuf::from(path))
        .map(Some)
        .map_err(|_| anyhow::anyhow!("{MOCK_ENV_STORE} should be valid UTF-8"))
}

fn load_mock_store(path: &Utf8PathBuf) -> anyhow::Result<MockEnvStore> {
    if !path.exists() {
        return Ok(MockEnvStore::default());
    }
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read mock env store {}", path))?;
    serde_json::from_str(&source)
        .with_context(|| format!("failed to parse mock env store {}", path))
}

fn save_mock_store(path: &Utf8PathBuf, store: &MockEnvStore) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create mock env store parent {}", parent))?;
    } else {
        bail!("mock env store path should have a parent");
    }
    let rendered = serde_json::to_string_pretty(store).context("failed to serialize env store")?;
    fs::write(path, rendered).with_context(|| format!("failed to write mock env store {}", path))
}

#[cfg(windows)]
fn native_get_env_var(scope: EnvScope, name: &str) -> anyhow::Result<Option<String>> {
    let key = open_environment_key(scope, false)?;
    match key.get_raw_value(name) {
        Ok(value) => {
            let decoded = String::from_reg_value(&value)
                .with_context(|| format!("failed to decode environment variable {name}"))?;
            Ok(Some(decoded))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read environment variable {name}"))
        }
    }
}

#[cfg(windows)]
fn native_set_env_var(scope: EnvScope, name: &str, value: Option<&str>) -> anyhow::Result<()> {
    let key = open_environment_key(scope, true)?;
    match value.filter(|value| !value.is_empty()) {
        Some(value) => {
            let value_kind = existing_string_kind(&key, name).unwrap_or_else(|| {
                if value.contains('%') {
                    REG_EXPAND_SZ
                } else {
                    REG_SZ
                }
            });
            let reg_value = encode_string_value(value, value_kind);
            key.set_raw_value(name, &reg_value)
                .with_context(|| format!("failed to set environment variable {name}"))?;
        }
        None => {
            if let Err(error) = key.delete_value(name)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(error)
                    .with_context(|| format!("failed to delete environment variable {name}"));
            }
        }
    }
    if let Err(error) = broadcast_environment_change() {
        tracing::warn!(
            variable = name,
            scope = ?scope,
            error = %error,
            "failed to broadcast environment-variable change"
        );
    }
    Ok(())
}

#[cfg(windows)]
fn open_environment_key(scope: EnvScope, write: bool) -> anyhow::Result<winreg::RegKey> {
    let path = environment_key_path(scope);
    let result = match (scope, write) {
        (EnvScope::User, false) => HKCU.open_subkey_with_flags(path, KEY_READ),
        (EnvScope::System, false) => HKLM.open_subkey_with_flags(path, KEY_READ),
        (EnvScope::User, true) => HKCU.create_subkey(path).map(|(key, _)| key),
        (EnvScope::System, true) => HKLM.create_subkey(path).map(|(key, _)| key),
    };
    result.with_context(|| format!("failed to open environment registry key {path}"))
}

#[cfg(windows)]
fn environment_key_path(scope: EnvScope) -> &'static str {
    match scope {
        EnvScope::User => "Environment",
        EnvScope::System => "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
    }
}

#[cfg(windows)]
fn existing_string_kind(key: &winreg::RegKey, name: &str) -> Option<winreg::enums::RegType> {
    key.get_raw_value(name)
        .ok()
        .and_then(|value| match value.vtype {
            REG_SZ | REG_EXPAND_SZ => Some(value.vtype),
            _ => None,
        })
}

#[cfg(windows)]
fn encode_string_value(value: &str, value_kind: winreg::enums::RegType) -> RegValue<'static> {
    let mut utf16 = value.encode_utf16().collect::<Vec<_>>();
    utf16.push(0);
    let bytes = utf16
        .into_iter()
        .flat_map(|unit| unit.to_le_bytes())
        .collect::<Vec<_>>();
    RegValue {
        bytes: bytes.into(),
        vtype: value_kind,
    }
}

#[cfg(test)]
mod tests {
    use super::split_path_like_env_var;

    #[test]
    fn splits_path_like_values_using_windows_semantics() {
        let (removed, kept) = split_path_like_env_var(
            &[String::from("C:\\demo"), String::from("C:\\tool\\*")],
            Some("C:\\demo;C:\\tool\\bin;C:\\other"),
        );
        assert_eq!(
            removed,
            vec![String::from("C:\\demo"), String::from("C:\\tool\\bin")]
        );
        assert_eq!(kept, vec![String::from("C:\\other")]);
    }
}
