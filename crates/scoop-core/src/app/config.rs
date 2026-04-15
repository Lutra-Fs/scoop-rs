use serde_json::{Map, Value};

use crate::{
    RuntimeConfig,
    infra::{
        config::{active_config_map, active_config_value, set_active_config_value},
        sqlite_cache::rebuild_search_cache,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSetResult {
    pub name: String,
    pub value: Value,
    pub initialized_sqlite_cache: bool,
}

pub fn current_config(config: &RuntimeConfig) -> Map<String, Value> {
    active_config_map(config)
}

pub fn get_config_value(config: &RuntimeConfig, name: &str) -> Option<Value> {
    active_config_value(config, name)
}

pub fn set_config_value(
    config: &RuntimeConfig,
    name: &str,
    raw_value: &str,
) -> anyhow::Result<ConfigSetResult> {
    let value = parse_cli_value(raw_value);
    set_config_json_value(config, name, value)
}

pub fn set_config_json_value(
    config: &RuntimeConfig,
    name: &str,
    value: Value,
) -> anyhow::Result<ConfigSetResult> {
    set_active_config_value(config, name, Some(value.clone()))?;

    let initialized_sqlite_cache =
        name.eq_ignore_ascii_case("use_sqlite_cache") && value == Value::Bool(true);
    if initialized_sqlite_cache {
        rebuild_search_cache(config)?;
    }

    Ok(ConfigSetResult {
        name: name.to_ascii_lowercase(),
        value,
        initialized_sqlite_cache,
    })
}

pub fn remove_config_value(config: &RuntimeConfig, name: &str) -> anyhow::Result<()> {
    set_active_config_value(config, name, None)
}

pub fn parse_cli_value(raw_value: &str) -> Value {
    if raw_value.eq_ignore_ascii_case("true") {
        Value::Bool(true)
    } else if raw_value.eq_ignore_ascii_case("false") {
        Value::Bool(false)
    } else {
        Value::String(raw_value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_cli_value;

    #[test]
    fn parses_boolean_like_values_case_insensitively() {
        assert_eq!(parse_cli_value("true"), json!(true));
        assert_eq!(parse_cli_value("FALSE"), json!(false));
    }

    #[test]
    fn keeps_other_cli_values_as_strings() {
        assert_eq!(parse_cli_value("D:/Custom/Scoop"), json!("D:/Custom/Scoop"));
        assert_eq!(parse_cli_value("2026-04-15"), json!("2026-04-15"));
    }
}
