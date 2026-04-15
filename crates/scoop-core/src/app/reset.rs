use anyhow::{Context, bail};
use camino::Utf8Path;
use serde_json::Value;

use crate::{
    RuntimeConfig,
    domain::install_context::{InstallContext, InstallContextPaths},
};

use super::install::{
    activate_current_dir, arch_specific_strings, arch_specific_value, create_cmd_shims,
    create_startmenu_shortcuts, current_version, ensure_install_dir_not_in_path, env_add_paths,
    env_set_values, install_psmodule, installed_versions, is_admin, load_manifest_json,
    persist_data, windows_path,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetOutcome {
    pub app: String,
    pub version: String,
}

pub fn reset_apps(
    config: &RuntimeConfig,
    app_specs: &[String],
) -> anyhow::Result<Vec<ResetOutcome>> {
    let mut outcomes = Vec::new();
    for spec in app_specs {
        outcomes.push(reset_single_app(config, spec)?);
    }
    Ok(outcomes)
}

fn reset_single_app(config: &RuntimeConfig, spec: &str) -> anyhow::Result<ResetOutcome> {
    let (app_name, requested_version) = match spec.rsplit_once('@') {
        Some((left, right)) if !right.is_empty() => (left, Some(right)),
        _ => (spec, None),
    };

    if app_name == "scoop" {
        bail!("Resetting scoop itself is not supported.");
    }

    // Determine scope – prefer local, fall back to global
    let (paths, global) = if config.paths().app_dir(app_name).exists() {
        (config.paths(), false)
    } else if config.global_paths().app_dir(app_name).exists() {
        (config.global_paths(), true)
    } else {
        bail!("'{app_name}' isn't installed.");
    };

    if global && !is_admin()? {
        bail!("'{app_name}' is a global app. You need admin rights to reset it. Skipping.");
    }

    // Determine version
    let version = match requested_version {
        Some(v) => {
            let version_dir = paths.version_dir(app_name, v);
            if !version_dir.join("install.json").is_file() {
                bail!("'{app_name} ({v})' isn't installed.");
            }
            v.to_owned()
        }
        None => match current_version(paths.root(), app_name)? {
            Some(v) => v,
            None => match installed_versions(paths.root(), app_name)?.pop() {
                Some(v) => v,
                None => bail!("'{app_name}' isn't installed correctly."),
            },
        },
    };

    let version_dir = paths.version_dir(app_name, &version);
    let original_dir = version_dir.clone();
    let persist_dir = paths.persist().join(app_name);

    let manifest = load_installed_manifest(&version_dir)
        .with_context(|| format!("'{app_name} ({version})' isn't installed."))?;
    let install_info = load_install_info(&version_dir);
    let architecture = install_info
        .as_ref()
        .and_then(|info| info.get("architecture").and_then(Value::as_str))
        .unwrap_or("64bit")
        .to_owned();

    // Re-link current
    let current_dir = activate_current_dir(
        paths.app_dir(app_name),
        &version_dir,
        config.settings().no_junction.unwrap_or(false),
    )?;

    let context = InstallContext::new(
        app_name.to_owned(),
        version.clone(),
        architecture.clone(),
        global,
        InstallContextPaths {
            dir: current_dir.clone(),
            original_dir: original_dir.clone(),
            persist_dir: persist_dir.clone(),
        },
        manifest.clone(),
    );

    // Re-create shims
    let _ = create_cmd_shims(paths.shims(), &manifest, &context);

    // Re-create shortcuts
    let _ = create_startmenu_shortcuts(&manifest, &context);

    // Undo old environment state, then re-apply
    let _ = env_rm_for_reset(config, &manifest, &current_dir, global, &architecture);
    let _ = env_add_paths(config, &manifest, &context, global, &architecture);
    let _ = env_set_values(&manifest, &context, global, &architecture);
    let _ = install_psmodule(&manifest, &context, paths);

    // Unlink old persist data, re-persist
    let _ = super::uninstall::unlink_persist_data(&manifest, &original_dir);
    let _ = persist_data(&manifest, &context);

    // Ensure install dir is not in PATH
    let _ = ensure_install_dir_not_in_path(&context, global);

    Ok(ResetOutcome {
        app: app_name.to_owned(),
        version,
    })
}

fn load_installed_manifest(version_dir: &Utf8Path) -> anyhow::Result<Value> {
    let path = version_dir.join("manifest.json");
    load_manifest_json(&path)
}

fn load_install_info(version_dir: &Utf8Path) -> Option<Value> {
    let path = version_dir.join("install.json");
    load_manifest_json(&path).ok()
}

/// Remove env_add_path and env_set entries – mirrors uninstall's env removal.
fn env_rm_for_reset(
    config: &RuntimeConfig,
    manifest: &Value,
    ref_dir: &Utf8Path,
    global: bool,
    architecture: &str,
) -> anyhow::Result<()> {
    use super::install::is_in_dir;
    use crate::infra::environment::{EnvScope, remove_path, scoop_path_env_var, set_env_var};

    let scope = if global {
        EnvScope::System
    } else {
        EnvScope::User
    };

    // env_add_path removal
    let additions = arch_specific_strings(manifest, architecture, "env_add_path");
    if !additions.is_empty() {
        let mut paths = Vec::new();
        for addition in &additions {
            let candidate = ref_dir.join(addition);
            if is_in_dir(ref_dir, &candidate) {
                paths.push(windows_path(&candidate));
            }
        }
        if !paths.is_empty() {
            let settings = config.settings();
            let target_env_var = scoop_path_env_var(&settings);
            let _ = remove_path(scope, "PATH", &paths);
            if target_env_var != "PATH" {
                let _ = remove_path(scope, &target_env_var, &paths);
            }
        }
    }

    // env_set removal
    if let Some(env_set) = arch_specific_value(manifest, architecture, "env_set")
        && let Some(object) = env_set.as_object()
    {
        for name in object.keys() {
            let _ = set_env_var(scope, name, None);
        }
    }

    Ok(())
}
