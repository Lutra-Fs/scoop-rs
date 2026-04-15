use std::{
    fs,
    time::{Duration, SystemTime},
};

use anyhow::{Context, bail};
use serde_json::{Map, Value};

use crate::{
    RuntimeConfig,
    app::install::{current_version, is_admin},
    infra::config::set_active_config_value,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldOutcome {
    ScoopHeld { hold_until: String },
    ScoopUnheld,
    Held { app: String },
    AlreadyHeld { app: String },
    Unheld { app: String },
    NotHeld { app: String },
    NotInstalled { app: String, global: bool },
}

pub fn hold_apps(
    config: &RuntimeConfig,
    apps: &[String],
    global: bool,
) -> anyhow::Result<Vec<HoldOutcome>> {
    ensure_admin_for_global(global)?;

    let mut outcomes = Vec::with_capacity(apps.len());
    for app in apps {
        if app.eq_ignore_ascii_case("scoop") {
            let hold_until =
                jiff::Timestamp::try_from(SystemTime::now() + Duration::from_secs(86_400))
                    .context("failed to format hold timestamp")?
                    .to_string();
            set_active_config_value(
                config,
                "hold_update_until",
                Some(Value::String(hold_until.clone())),
            )?;
            outcomes.push(HoldOutcome::ScoopHeld { hold_until });
            continue;
        }
        outcomes.push(set_app_hold_state(config, app, global, true)?);
    }
    Ok(outcomes)
}

pub fn unhold_apps(
    config: &RuntimeConfig,
    apps: &[String],
    global: bool,
) -> anyhow::Result<Vec<HoldOutcome>> {
    ensure_admin_for_global(global)?;

    let mut outcomes = Vec::with_capacity(apps.len());
    for app in apps {
        if app.eq_ignore_ascii_case("scoop") {
            set_active_config_value(config, "hold_update_until", None)?;
            outcomes.push(HoldOutcome::ScoopUnheld);
            continue;
        }
        outcomes.push(set_app_hold_state(config, app, global, false)?);
    }
    Ok(outcomes)
}

fn ensure_admin_for_global(global: bool) -> anyhow::Result<()> {
    if !global || is_admin()? {
        return Ok(());
    }
    bail!("You need admin rights to modify hold state for a global app.");
}

fn set_app_hold_state(
    config: &RuntimeConfig,
    app: &str,
    global: bool,
    held: bool,
) -> anyhow::Result<HoldOutcome> {
    let paths = if global {
        config.global_paths()
    } else {
        config.paths()
    };

    let Some(version) = current_version(paths.root(), app)? else {
        return Ok(HoldOutcome::NotInstalled {
            app: app.to_owned(),
            global,
        });
    };

    let install_json_path = paths.version_dir(app, &version).join("install.json");
    let mut install_info = load_install_info(&install_json_path)?;
    let currently_held = install_info
        .get("hold")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if held == currently_held {
        return Ok(if held {
            HoldOutcome::AlreadyHeld {
                app: app.to_owned(),
            }
        } else {
            HoldOutcome::NotHeld {
                app: app.to_owned(),
            }
        });
    }

    if held {
        install_info.insert(String::from("hold"), Value::Bool(true));
    } else {
        install_info.remove("hold");
    }

    let json = serde_json::to_string_pretty(&Value::Object(install_info))
        .context("failed to serialize install info")?;
    fs::write(&install_json_path, json)
        .with_context(|| format!("failed to write install info {}", install_json_path))?;

    Ok(if held {
        HoldOutcome::Held {
            app: app.to_owned(),
        }
    } else {
        HoldOutcome::Unheld {
            app: app.to_owned(),
        }
    })
}

fn load_install_info(path: &camino::Utf8Path) -> anyhow::Result<Map<String, Value>> {
    let source = fs::read_to_string(path).with_context(|| format!("failed to read {}", path))?;
    let value: Value =
        serde_json::from_str(&source).with_context(|| format!("failed to parse {}", path))?;
    let Value::Object(map) = value else {
        bail!("install info {} must be a JSON object", path);
    };
    Ok(map)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{HoldOutcome, hold_apps, unhold_apps};

    #[test]
    fn hold_and_unhold_toggle_install_metadata() {
        let fixture = HoldFixture::new();
        fixture.install_app("demo", "1.2.3", r#"{"bucket":"main"}"#);

        let held = hold_apps(&fixture.config, &[String::from("demo")], false)
            .expect("hold should succeed");
        assert_eq!(
            held,
            vec![HoldOutcome::Held {
                app: String::from("demo")
            }]
        );
        assert_eq!(fixture.install_info("demo", "1.2.3")["hold"], json!(true));

        let unheld = unhold_apps(&fixture.config, &[String::from("demo")], false)
            .expect("unhold should succeed");
        assert_eq!(
            unheld,
            vec![HoldOutcome::Unheld {
                app: String::from("demo")
            }]
        );
        assert!(fixture.install_info("demo", "1.2.3").get("hold").is_none());
    }

    #[test]
    fn hold_scoop_sets_hold_update_until() {
        let fixture = HoldFixture::new();

        let outcomes = hold_apps(&fixture.config, &[String::from("scoop")], false)
            .expect("hold should succeed");

        assert!(matches!(outcomes[0], HoldOutcome::ScoopHeld { .. }));
        let config_path = fixture.config.paths().root().join("config.json");
        let rendered = fs::read_to_string(config_path).expect("config should be written");
        assert!(rendered.contains("hold_update_until"));
    }

    struct HoldFixture {
        _temp: TempDir,
        config: RuntimeConfig,
    }

    impl HoldFixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            let local_root = root.join("local");
            let global_root = root.join("global");
            fs::create_dir_all(&local_root).expect("local root should exist");
            fs::create_dir_all(&global_root).expect("global root should exist");
            fs::write(local_root.join("config.json"), "{}").expect("portable config should exist");

            Self {
                _temp: temp,
                config: RuntimeConfig::new(local_root, global_root),
            }
        }

        fn install_app(&self, app: &str, version: &str, install_json: &str) {
            let version_dir = self
                .config
                .paths()
                .root()
                .join("apps")
                .join(app)
                .join(version);
            let current_dir = self
                .config
                .paths()
                .root()
                .join("apps")
                .join(app)
                .join("current");
            fs::create_dir_all(&version_dir).expect("version dir should exist");
            fs::create_dir_all(&current_dir).expect("current dir should exist");
            fs::write(version_dir.join("install.json"), install_json)
                .expect("install info should exist");
            fs::write(
                current_dir.join("manifest.json"),
                format!(r#"{{"version":"{version}"}}"#),
            )
            .expect("manifest should exist");
        }

        fn install_info(&self, app: &str, version: &str) -> Value {
            let path = self
                .config
                .paths()
                .root()
                .join("apps")
                .join(app)
                .join(version)
                .join("install.json");
            serde_json::from_str(&fs::read_to_string(path).expect("install info should exist"))
                .expect("install info should parse")
        }
    }
}
