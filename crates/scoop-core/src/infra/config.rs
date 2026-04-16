use std::fs;

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::domain::paths::ScoopPaths;

pub const DEFAULT_SCOOP_GLOBAL_ROOT: &str = "C:/ProgramData/scoop";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    local_paths: ScoopPaths,
    global_paths: ScoopPaths,
    cache_dir: Utf8PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ScoopSettings {
    #[serde(default, alias = "ROOT_PATH")]
    pub root_path: Option<String>,
    #[serde(default, alias = "GLOBAL_PATH")]
    pub global_path: Option<String>,
    #[serde(default, alias = "CACHE_PATH")]
    pub cache_path: Option<String>,
    #[serde(default, alias = "CAT_STYLE")]
    pub cat_style: Option<String>,
    #[serde(default, alias = "USE_SQLITE_CACHE")]
    pub use_sqlite_cache: Option<bool>,
    #[serde(default, alias = "USE_ISOLATED_PATH")]
    pub use_isolated_path: Option<IsolatedPathSetting>,
    #[serde(default, alias = "NO_JUNCTION")]
    pub no_junction: Option<bool>,
    #[serde(default, alias = "USE_EXTERNAL_7ZIP")]
    pub use_external_7zip: Option<bool>,
    #[serde(default, alias = "USE_LESSMSI")]
    pub use_lessmsi: Option<bool>,
    #[serde(default, alias = "UPDATE_NIGHTLY")]
    pub update_nightly: Option<bool>,
    #[serde(default, alias = "LAST_UPDATE")]
    pub last_update: Option<String>,
    #[serde(default, alias = "HOLD_UPDATE_UNTIL")]
    pub hold_update_until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum IsolatedPathSetting {
    Enabled(bool),
    Name(String),
}

impl RuntimeConfig {
    pub fn new(local_root: Utf8PathBuf, global_root: Utf8PathBuf) -> Self {
        let cache_dir = local_root.join("cache");
        Self::with_cache(local_root, global_root, cache_dir)
    }

    pub fn with_cache(
        local_root: Utf8PathBuf,
        global_root: Utf8PathBuf,
        cache_dir: Utf8PathBuf,
    ) -> Self {
        Self {
            local_paths: ScoopPaths::new(local_root),
            global_paths: ScoopPaths::new(global_root),
            cache_dir,
        }
    }

    pub fn detect(root_override: Option<Utf8PathBuf>) -> Self {
        let user_settings = load_settings(user_config_path().as_deref());
        let default_local_root = default_scoop_root();
        let default_program_data = std::env::var("PROGRAMDATA")
            .map(|path| Utf8PathBuf::from(path).join("scoop"))
            .unwrap_or_else(|_| Utf8PathBuf::from(DEFAULT_SCOOP_GLOBAL_ROOT));

        Self::from_parts(
            root_override,
            std::env::var("SCOOP").ok().map(Utf8PathBuf::from),
            std::env::var("SCOOP_GLOBAL").ok().map(Utf8PathBuf::from),
            std::env::var("SCOOP_CACHE").ok().map(Utf8PathBuf::from),
            user_settings,
            default_local_root,
            default_program_data,
        )
    }

    fn from_parts(
        root_override: Option<Utf8PathBuf>,
        scoop_env: Option<Utf8PathBuf>,
        scoop_global_env: Option<Utf8PathBuf>,
        scoop_cache_env: Option<Utf8PathBuf>,
        user_settings: ScoopSettings,
        default_local_root: Utf8PathBuf,
        default_global_root: Utf8PathBuf,
    ) -> Self {
        let local_root = root_override
            .or(scoop_env)
            .or_else(|| user_settings.root_path.as_ref().map(Utf8PathBuf::from))
            .unwrap_or(default_local_root);
        let global_root = scoop_global_env
            .or_else(|| user_settings.global_path.as_ref().map(Utf8PathBuf::from))
            .unwrap_or(default_global_root);
        let cache_dir = scoop_cache_env
            .or_else(|| user_settings.cache_path.as_ref().map(Utf8PathBuf::from))
            .unwrap_or_else(|| local_root.join("cache"));

        Self::with_cache(local_root, global_root, cache_dir)
    }

    pub fn paths(&self) -> &ScoopPaths {
        &self.local_paths
    }

    pub fn global_paths(&self) -> &ScoopPaths {
        &self.global_paths
    }

    pub fn cache_dir(&self) -> &Utf8Path {
        &self.cache_dir
    }

    pub fn settings(&self) -> ScoopSettings {
        let user_settings = load_settings(user_config_path().as_deref());
        let portable_settings = load_settings(Some(&self.paths().root().join("config.json")));
        user_settings.merge(portable_settings)
    }
}

pub fn default_scoop_root() -> Utf8PathBuf {
    default_scoop_root_from_env(
        std::env::var("USERPROFILE").ok(),
        std::env::var("HOME").ok(),
    )
}

fn default_scoop_root_from_env(userprofile: Option<String>, home: Option<String>) -> Utf8PathBuf {
    userprofile
        .or(home)
        .map(|path| Utf8PathBuf::from(path.replace('\\', "/")))
        .unwrap_or_default()
        .join("scoop")
}

impl ScoopSettings {
    fn merge(self, override_settings: Self) -> Self {
        Self {
            root_path: override_settings.root_path.or(self.root_path),
            global_path: override_settings.global_path.or(self.global_path),
            cache_path: override_settings.cache_path.or(self.cache_path),
            cat_style: override_settings.cat_style.or(self.cat_style),
            use_sqlite_cache: override_settings.use_sqlite_cache.or(self.use_sqlite_cache),
            use_isolated_path: override_settings
                .use_isolated_path
                .or(self.use_isolated_path),
            no_junction: override_settings.no_junction.or(self.no_junction),
            use_external_7zip: override_settings
                .use_external_7zip
                .or(self.use_external_7zip),
            use_lessmsi: override_settings.use_lessmsi.or(self.use_lessmsi),
            update_nightly: override_settings.update_nightly.or(self.update_nightly),
            last_update: override_settings.last_update.or(self.last_update),
            hold_update_until: override_settings
                .hold_update_until
                .or(self.hold_update_until),
        }
    }
}

pub fn portable_config_path(config: &RuntimeConfig) -> Utf8PathBuf {
    config.paths().root().join("config.json")
}

pub fn active_config_path(config: &RuntimeConfig) -> Option<Utf8PathBuf> {
    let portable = portable_config_path(config);
    if portable.is_file() {
        Some(portable)
    } else {
        user_config_path()
    }
}

pub fn active_config_map(config: &RuntimeConfig) -> Map<String, Value> {
    active_config_path(config)
        .as_deref()
        .map(load_raw_config_map)
        .unwrap_or_default()
}

pub fn active_config_value(config: &RuntimeConfig, name: &str) -> Option<Value> {
    active_config_map(config)
        .get(&name.to_ascii_lowercase())
        .cloned()
}

pub fn user_config_path() -> Option<Utf8PathBuf> {
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map(Utf8PathBuf::from)
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .filter(|value| !value.is_empty())
                .map(|home| Utf8PathBuf::from(home).join(".config"))
        })?;
    Some(config_home.join("scoop").join("config.json"))
}

pub fn set_user_config_value(name: &str, value: Option<Value>) -> anyhow::Result<()> {
    let Some(path) = user_config_path() else {
        return Ok(());
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent))?;
    }

    let mut config = load_raw_config_map(&path);
    let key = name.to_ascii_lowercase();
    match value {
        Some(value) => {
            config.insert(key, value);
        }
        None => {
            config.remove(&key);
        }
    }

    let json = serde_json::to_string_pretty(&Value::Object(config))
        .context("failed to serialize scoop config")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path))?;
    Ok(())
}

pub fn set_active_config_value(
    config: &RuntimeConfig,
    name: &str,
    value: Option<Value>,
) -> anyhow::Result<()> {
    let Some(path) = active_config_path(config).or_else(user_config_path) else {
        return Ok(());
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent))?;
    }

    let mut config = load_raw_config_map(&path);
    let key = name.to_ascii_lowercase();
    match value {
        Some(value) => {
            config.insert(key, value);
        }
        None => {
            config.remove(&key);
        }
    }

    let json = serde_json::to_string_pretty(&Value::Object(config))
        .context("failed to serialize scoop config")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path))?;
    Ok(())
}

fn load_settings(path: Option<&Utf8Path>) -> ScoopSettings {
    let Some(path) = path.filter(|path| path.is_file()) else {
        return ScoopSettings::default();
    };

    let Ok(source) = fs::read_to_string(path) else {
        return ScoopSettings::default();
    };

    serde_json::from_str(&source).unwrap_or_default()
}

fn load_raw_config_map(path: &Utf8Path) -> Map<String, Value> {
    let Ok(source) = fs::read_to_string(path) else {
        return Map::new();
    };
    serde_json::from_str::<Map<String, Value>>(&source).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        DEFAULT_SCOOP_GLOBAL_ROOT, RuntimeConfig, ScoopSettings, active_config_path,
        active_config_value, default_scoop_root_from_env, portable_config_path,
        set_active_config_value,
    };

    #[test]
    fn override_wins_over_environment() {
        let config = RuntimeConfig::from_parts(
            Some(Utf8PathBuf::from("E:/Portable/Scoop")),
            Some(Utf8PathBuf::from("D:/Ignored/Scoop")),
            Some(Utf8PathBuf::from("C:/ProgramData/scoop")),
            None,
            ScoopSettings::default(),
            Utf8PathBuf::from("C:/Users/example/scoop"),
            Utf8PathBuf::from("C:/ProgramData/scoop"),
        );
        assert_eq!(config.paths().root(), "E:/Portable/Scoop");
    }

    #[test]
    fn default_root_uses_userprofile_semantics() {
        assert_eq!(
            default_scoop_root_from_env(
                Some(String::from("C:/Users/example")),
                Some(String::from("C:/Users/fallback"))
            ),
            Utf8PathBuf::from("C:/Users/example/scoop")
        );
    }

    #[test]
    fn default_root_uses_home_as_guard() {
        assert_eq!(
            default_scoop_root_from_env(None, Some(String::from("C:/Users/example"))),
            Utf8PathBuf::from("C:/Users/example/scoop")
        );
    }

    #[test]
    fn default_root_from_parts_is_used_when_override_env_and_config_are_absent() {
        let config = RuntimeConfig::from_parts(
            None,
            None,
            None,
            None,
            ScoopSettings::default(),
            Utf8PathBuf::from("C:/Users/example/scoop"),
            Utf8PathBuf::from(DEFAULT_SCOOP_GLOBAL_ROOT),
        );

        assert_eq!(config.paths().root(), "C:/Users/example/scoop");
    }

    #[test]
    fn global_root_defaults_to_program_data_layout() {
        let config = RuntimeConfig::from_parts(
            None,
            None,
            None,
            None,
            ScoopSettings::default(),
            Utf8PathBuf::from("C:/Users/example/scoop"),
            Utf8PathBuf::from(DEFAULT_SCOOP_GLOBAL_ROOT),
        );
        assert_eq!(config.global_paths().root(), DEFAULT_SCOOP_GLOBAL_ROOT);
    }

    #[test]
    fn explicit_global_root_is_respected() {
        let config = RuntimeConfig::from_parts(
            None,
            None,
            Some(Utf8PathBuf::from("E:/Global/Scoop")),
            None,
            ScoopSettings::default(),
            Utf8PathBuf::from("C:/Users/example/scoop"),
            Utf8PathBuf::from(DEFAULT_SCOOP_GLOBAL_ROOT),
        );
        assert_eq!(config.global_paths().root(), "E:/Global/Scoop");
    }

    #[test]
    fn config_paths_can_override_detected_roots() {
        let config = RuntimeConfig::from_parts(
            None,
            None,
            None,
            None,
            ScoopSettings {
                root_path: Some(String::from("E:/Configured/Scoop")),
                global_path: Some(String::from("E:/Configured/Global")),
                cache_path: Some(String::from("E:/Configured/Cache")),
                ..ScoopSettings::default()
            },
            Utf8PathBuf::from("C:/Users/example/scoop"),
            Utf8PathBuf::from(DEFAULT_SCOOP_GLOBAL_ROOT),
        );

        assert_eq!(config.paths().root(), "E:/Configured/Scoop");
        assert_eq!(config.global_paths().root(), "E:/Configured/Global");
        assert_eq!(config.cache_dir(), "E:/Configured/Cache");
    }

    #[test]
    fn active_config_prefers_portable_file_when_present() {
        let fixture = ConfigFixture::new();
        let portable = portable_config_path(&fixture.config);
        fs::create_dir_all(
            portable
                .parent()
                .expect("portable config should have a parent"),
        )
        .expect("portable parent should exist");
        fs::write(&portable, r#"{"root_path":"E:/Portable/Scoop"}"#)
            .expect("portable config should exist");

        assert_eq!(
            active_config_path(&fixture.config).as_deref(),
            Some(portable.as_path())
        );
    }

    #[test]
    fn set_active_config_writes_to_portable_file_when_present() {
        let fixture = ConfigFixture::new();
        let portable = portable_config_path(&fixture.config);
        fs::create_dir_all(
            portable
                .parent()
                .expect("portable config should have a parent"),
        )
        .expect("portable parent should exist");
        fs::write(&portable, "{}").expect("portable config should exist");

        set_active_config_value(&fixture.config, "cat_style", Some(json!("numbers")))
            .expect("config should be written");

        let rendered = fs::read_to_string(&portable).expect("portable config should be readable");
        assert!(rendered.contains(r#""cat_style": "numbers""#));
    }

    #[test]
    fn active_config_value_reads_case_insensitive_key() {
        let fixture = ConfigFixture::new();
        let portable = portable_config_path(&fixture.config);
        fs::create_dir_all(
            portable
                .parent()
                .expect("portable config should have a parent"),
        )
        .expect("portable parent should exist");
        fs::write(&portable, r#"{"use_sqlite_cache":true}"#).expect("portable config should exist");

        assert_eq!(
            active_config_value(&fixture.config, "USE_SQLITE_CACHE"),
            Some(json!(true))
        );
    }

    struct ConfigFixture {
        _temp: TempDir,
        config: RuntimeConfig,
    }

    impl ConfigFixture {
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
                config: RuntimeConfig::new(local_root, global_root),
            }
        }
    }
}
