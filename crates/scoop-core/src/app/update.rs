use std::fs;

use anyhow::{Context, bail};
use camino::{Utf8Path, Utf8PathBuf};
use jiff::Timestamp;
use serde_json::Value;

use crate::{
    RuntimeConfig,
    domain::install_context::{HookType, InstallContext, InstallContextPaths},
    infra::{config::set_user_config_value, git::pull_tags_force},
};

use super::install::{
    InstallOptions, InstallOutcome, current_version, install_app_allow_upgrade, is_admin,
    load_manifest_json, manifest_version, run_manifest_hook,
};
use super::uninstall;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOptions {
    pub global: bool,
    pub force: bool,
    pub independent: bool,
    pub use_cache: bool,
    pub check_hash: bool,
    pub quiet: bool,
    pub all: bool,
}

impl Default for UpdateOptions {
    fn default() -> Self {
        Self {
            global: false,
            force: false,
            independent: false,
            use_cache: true,
            check_hash: true,
            quiet: false,
            all: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// Scoop core / buckets were synced (no-args mode)
    ScoopUpdateHeld {
        hold_until: String,
    },
    ScoopUpdated {
        changelog: Option<String>,
    },
    /// An app was updated from old_version to new_version
    AppUpdated {
        app: String,
        old_version: String,
        new_version: String,
        changelog: Option<String>,
    },
    /// App is already at the latest version
    AlreadyLatest {
        app: String,
        version: String,
    },
    /// No manifest found for the app
    NoManifest {
        app: String,
    },
    /// App is held
    Held {
        app: String,
        version: String,
    },
    /// App skipped because its process tree is still running
    RunningProcess {
        app: String,
        processes: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelfUpdatePlan {
    /// Scoop self-update is intentionally deferred until the configured timestamp.
    Held { hold_until: String },
    /// Refresh the Scoop app referenced by this manifest source.
    Refresh {
        reference: String,
        changelog: Option<String>,
    },
}

const LAST_UPDATE_KEY: &str = "LAST_UPDATE";
const SCOOP_OUTDATED_AFTER_HOURS: f64 = 3.0;
const SCHEME_HOLD_UPDATE_UNTIL: &str = "HOLD_UPDATE_UNTIL";

// ---------------------------------------------------------------------------
// Scoop / bucket sync (no-args)
// ---------------------------------------------------------------------------

pub fn update_scoop(config: &RuntimeConfig) -> anyhow::Result<UpdateOutcome> {
    // Phase 2 boundary: command-layer handles lifecycle intent; installer/updater owns
    // in-process executable replacement when supported by the runtime packaging.
    let plan = plan_scoop_self_update(config)?;
    apply_scoop_self_update(config, plan)
}

pub fn plan_scoop_self_update(config: &RuntimeConfig) -> anyhow::Result<SelfUpdatePlan> {
    if let Some(hold_until) = is_scoop_update_held(config)? {
        return Ok(SelfUpdatePlan::Held { hold_until });
    }

    Ok(SelfUpdatePlan::Refresh {
        reference: scoop_self_reference(config)?,
        changelog: extract_changelog_from_scoop_manifest(config)?,
    })
}

pub fn apply_scoop_self_update(
    config: &RuntimeConfig,
    plan: SelfUpdatePlan,
) -> anyhow::Result<UpdateOutcome> {
    match plan {
        SelfUpdatePlan::Held { hold_until } => Ok(UpdateOutcome::ScoopUpdateHeld { hold_until }),
        SelfUpdatePlan::Refresh {
            reference,
            changelog,
        } => {
            sync_buckets(config)?;
            install_scoop_reference(config, &reference)?;
            mark_last_update_now()?;
            Ok(UpdateOutcome::ScoopUpdated { changelog })
        }
    }
}

pub fn is_scoop_outdated(config: &RuntimeConfig) -> anyhow::Result<bool> {
    let now = Timestamp::now();
    let Some(last_update) = config.settings().last_update else {
        mark_last_update_now()?;
        return Ok(true);
    };

    match last_update.parse::<Timestamp>() {
        Ok(last_update) => Ok(
            now.duration_since(last_update).as_secs_f64() / 3600.0 >= SCOOP_OUTDATED_AFTER_HOURS
        ),
        Err(_) => {
            mark_last_update_now()?;
            Ok(true)
        }
    }
}

fn sync_buckets(config: &RuntimeConfig) -> anyhow::Result<()> {
    let buckets_root = config.paths().buckets();
    if !buckets_root.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(&buckets_root)
        .with_context(|| format!("failed to read buckets directory {}", buckets_root))?;
    for entry in entries.flatten() {
        let path = Utf8PathBuf::try_from(entry.path())
            .unwrap_or_else(|_| Utf8PathBuf::from(entry.path().to_string_lossy().as_ref()));
        if !path.join(".git").is_dir() {
            continue;
        }
        if let Err(error) = pull_tags_force(&path) {
            tracing::warn!("failed to pull bucket {}: {}", path, error);
        }
    }
    Ok(())
}

fn is_scoop_update_held(config: &RuntimeConfig) -> anyhow::Result<Option<String>> {
    let Some(hold_update_until) = config.settings().hold_update_until else {
        return Ok(None);
    };

    let hold_timestamp = match hold_update_until.parse::<Timestamp>() {
        Ok(timestamp) => timestamp,
        Err(_) => {
            set_user_config_value(SCHEME_HOLD_UPDATE_UNTIL, None)?;
            return Ok(None);
        }
    };

    if Timestamp::now() < hold_timestamp {
        return Ok(Some(hold_update_until));
    }
    Ok(None)
}

fn extract_changelog_from_scoop_manifest(config: &RuntimeConfig) -> anyhow::Result<Option<String>> {
    let reference = scoop_self_reference(config)?;
    Ok(
        crate::compat::catalog::resolve_manifest(config, &reference)?
            .and_then(|manifest| extract_changelog(&manifest.manifest)),
    )
}

fn extract_changelog(manifest: &Value) -> Option<String> {
    manifest
        .get("changelog")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn install_scoop_reference(config: &RuntimeConfig, reference: &str) -> anyhow::Result<()> {
    let options = InstallOptions {
        no_update_scoop: true,
        ..InstallOptions::default()
    };

    match install_app_allow_upgrade(config, reference, &options)? {
        InstallOutcome::Installed(_) | InstallOutcome::AlreadyInstalled { .. } => Ok(()),
        InstallOutcome::MissingManifest { .. } => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// App update
// ---------------------------------------------------------------------------

pub fn update_apps(
    config: &RuntimeConfig,
    apps: &[String],
    options: &UpdateOptions,
) -> anyhow::Result<Vec<UpdateOutcome>> {
    if options.global && !is_admin()? {
        bail!("You need admin rights to update global apps.");
    }

    let mut outcomes = Vec::new();
    let update_scoop_first =
        apps.iter().any(|app| app.eq_ignore_ascii_case("scoop")) || is_scoop_outdated(config)?;
    if update_scoop_first {
        outcomes.push(update_scoop(config)?);
    }

    let explicit_apps = apps
        .iter()
        .filter(|app| !app.eq_ignore_ascii_case("scoop"))
        .cloned()
        .collect::<Vec<_>>();

    // If --all or *, discover all installed apps
    let app_list: Vec<(String, bool)> = if options.all || apps.iter().any(|a| a == "*") {
        let mut list = Vec::new();
        list.extend(
            discover_installed_app_names(config.paths())?
                .into_iter()
                .map(|a| (a, false)),
        );
        if options.global {
            list.extend(
                discover_installed_app_names(config.global_paths())?
                    .into_iter()
                    .map(|a| (a, true)),
            );
        }
        list
    } else {
        explicit_apps
            .iter()
            .map(|a| (a.clone(), options.global))
            .collect()
    };

    for (app_name, global) in &app_list {
        let opts = UpdateOptions {
            global: *global,
            ..options.clone()
        };
        outcomes.push(update_single_app(config, app_name, &opts)?);
    }
    Ok(outcomes)
}

fn mark_last_update_now() -> anyhow::Result<()> {
    set_user_config_value(
        LAST_UPDATE_KEY,
        Some(Value::String(Timestamp::now().to_string())),
    )
}

fn scoop_self_reference(config: &RuntimeConfig) -> anyhow::Result<String> {
    let scoop_root = config.paths().root();
    let Some(version) = current_version(scoop_root, "scoop")? else {
        return Ok(String::from("scoop"));
    };
    let install_path = config
        .paths()
        .version_dir("scoop", &version)
        .join("install.json");
    let install_info = match load_manifest_json(&install_path) {
        Ok(info) => info,
        Err(_) => return Ok(String::from("scoop")),
    };

    if let Some(bucket) = install_info.get("bucket").and_then(Value::as_str)
        && !bucket.is_empty()
    {
        return Ok(format!("{bucket}/scoop"));
    }
    if let Some(url) = install_info.get("url").and_then(Value::as_str)
        && !url.is_empty()
    {
        return Ok(url.to_owned());
    }
    Ok(String::from("scoop"))
}

fn update_single_app(
    config: &RuntimeConfig,
    app_name: &str,
    options: &UpdateOptions,
) -> anyhow::Result<UpdateOutcome> {
    let paths = if options.global {
        config.global_paths()
    } else {
        config.paths()
    };

    let old_version = match current_version(paths.root(), app_name)? {
        Some(v) => v,
        None => {
            return Ok(UpdateOutcome::NoManifest {
                app: app_name.to_owned(),
            });
        }
    };

    let old_version_dir = paths.version_dir(app_name, &old_version);
    let old_manifest = load_manifest_json(&old_version_dir.join("manifest.json")).ok();
    let install_info = load_manifest_json(&old_version_dir.join("install.json")).ok();
    let architecture = install_info
        .as_ref()
        .and_then(|info| info.get("architecture").and_then(Value::as_str))
        .unwrap_or("64bit")
        .to_owned();
    let bucket = install_info
        .as_ref()
        .and_then(|info| info.get("bucket").and_then(Value::as_str))
        .map(str::to_owned);
    let url = install_info
        .as_ref()
        .and_then(|info| info.get("url").and_then(Value::as_str))
        .map(str::to_owned);

    // Check if held
    if install_info
        .as_ref()
        .and_then(|info| info.get("hold").and_then(Value::as_bool))
        .unwrap_or(false)
        && !options.force
    {
        return Ok(UpdateOutcome::Held {
            app: app_name.to_owned(),
            version: old_version,
        });
    }

    // Resolve new manifest from bucket
    let reference = if let Some(ref url) = url {
        url.clone()
    } else {
        match &bucket {
            Some(bucket) => format!("{bucket}/{app_name}"),
            None => app_name.to_owned(),
        }
    };
    let new_manifest = crate::compat::catalog::resolve_manifest(config, &reference)?;
    let Some(new_manifest) = new_manifest else {
        return Ok(UpdateOutcome::NoManifest {
            app: app_name.to_owned(),
        });
    };

    let new_version = manifest_version(&new_manifest.manifest)
        .unwrap_or("0")
        .to_owned();
    let changelog = extract_changelog(&new_manifest.manifest);

    if !options.force && old_version == new_version {
        return Ok(UpdateOutcome::AlreadyLatest {
            app: app_name.to_owned(),
            version: old_version,
        });
    }

    let running_processes = uninstall::test_running_process(&paths.app_dir(app_name))?;
    if !running_processes.is_empty() {
        return Ok(UpdateOutcome::RunningProcess {
            app: app_name.to_owned(),
            processes: running_processes,
        });
    }

    // ---- Perform uninstall of old version ----
    let persist_dir = paths.persist().join(app_name);

    if let Some(ref manifest) = old_manifest {
        let ctx = make_context(
            app_name,
            &old_version,
            &architecture,
            options.global,
            &old_version_dir,
            &persist_dir,
            manifest,
        );
        let _ = run_manifest_hook(HookType::PreUninstall, manifest, &architecture, &ctx);
    }

    // Run old uninstaller
    if let Some(ref manifest) = old_manifest {
        let _ = uninstall::run_uninstaller(manifest, &architecture, &old_version_dir, app_name);
    }

    // Remove old shims
    if let Some(ref manifest) = old_manifest {
        uninstall::remove_shims(paths.shims(), manifest, &architecture, app_name)?;
    }

    // Unlink current
    let _ref_dir = uninstall::unlink_current_dir(
        &paths.app_dir(app_name),
        &old_version_dir,
        config.settings().no_junction.unwrap_or(false),
    )?;

    // Uninstall psmodule
    if let Some(ref manifest) = old_manifest {
        uninstall::uninstall_psmodule(manifest, &_ref_dir, paths)?;
    }

    // Undo env_add_path and env_set
    if let Some(ref manifest) = old_manifest {
        uninstall::env_remove_paths(config, manifest, &_ref_dir, options.global, &architecture)?;
        uninstall::env_remove_vars(manifest, options.global, &architecture)?;
    }

    // If forcing update to same version, rename old dir
    if options.force && old_version == new_version && old_version_dir.exists() {
        let backup_name = format!("_{}. old", old_version);
        let backup = paths.app_dir(app_name).join(&backup_name);
        let _ = fs::rename(&old_version_dir, &backup);
    }

    if let Some(ref manifest) = old_manifest {
        let ctx = make_context(
            app_name,
            &old_version,
            &architecture,
            options.global,
            &old_version_dir,
            &persist_dir,
            manifest,
        );
        let _ = run_manifest_hook(HookType::PostUninstall, manifest, &architecture, &ctx);
    }

    // ---- Install new version ----
    let install_reference = if let Some(ref url) = url {
        url.clone()
    } else {
        match &bucket {
            Some(bucket) => format!("{bucket}/{app_name}"),
            None => app_name.to_owned(),
        }
    };
    let install_options = super::install::InstallOptions {
        global: options.global,
        independent: options.independent,
        use_cache: options.use_cache,
        check_hash: options.check_hash,
        no_update_scoop: true,
        architecture: Some(architecture.clone()),
    };
    let outcome = super::install::install_app(config, &install_reference, &install_options)?;

    let actual_version = match &outcome {
        super::install::InstallOutcome::Installed(info) => info.version.clone(),
        _ => new_version.clone(),
    };

    Ok(UpdateOutcome::AppUpdated {
        app: app_name.to_owned(),
        old_version,
        new_version: actual_version,
        changelog,
    })
}

fn make_context(
    app: &str,
    version: &str,
    architecture: &str,
    global: bool,
    version_dir: &Utf8Path,
    persist_dir: &Utf8Path,
    manifest: &Value,
) -> InstallContext {
    InstallContext::new(
        app.to_owned(),
        version.to_owned(),
        architecture.to_owned(),
        global,
        InstallContextPaths {
            dir: version_dir.to_owned(),
            original_dir: version_dir.to_owned(),
            persist_dir: persist_dir.to_owned(),
        },
        manifest.clone(),
    )
}

fn discover_installed_app_names(
    paths: &crate::domain::paths::ScoopPaths,
) -> anyhow::Result<Vec<String>> {
    let apps_dir = paths.apps();
    if !apps_dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&apps_dir)
        .with_context(|| format!("failed to read apps dir {}", apps_dir))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "scoop" {
            continue;
        }
        if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}
