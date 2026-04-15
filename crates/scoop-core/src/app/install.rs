use std::{collections::BTreeSet, fs, io, process::Command};

use anyhow::{Context, bail};
use camino::{Utf8Path, Utf8PathBuf};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use zip::ZipArchive;

use crate::{
    RuntimeConfig,
    compat::catalog::{ResolvedManifest, render_manifest_json},
    domain::install_context::{HookType, InstallContext, InstallContextPaths},
    infra::{
        cache::canonical_cache_path,
        environment::{
            EnvScope, add_path, get_env_var, remove_path, scoop_path_env_var, set_env_var,
        },
        hash::sha256_file,
        http::build_blocking_http_client,
        powershell::run_install_hook,
        shortcuts::create_shortcut,
        versioned_manifest::resolve_versioned_manifest as resolve_historical_manifest,
        windows::security::is_elevated,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOptions {
    pub global: bool,
    pub independent: bool,
    pub use_cache: bool,
    pub check_hash: bool,
    pub no_update_scoop: bool,
    pub architecture: Option<String>,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            global: false,
            independent: false,
            use_cache: true,
            check_hash: true,
            no_update_scoop: false,
            architecture: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed(InstalledApp),
    AlreadyInstalled { app: String, version: String },
    MissingManifest { app: String, bucket: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    pub app: String,
    pub version: String,
    pub architecture: String,
    pub bucket: Option<String>,
    pub shim_names: Vec<String>,
    pub notes: Vec<String>,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DependencyPlanEntry {
    pub reference: String,
    pub source: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppReferenceParts {
    pub app: String,
    pub bucket: Option<String>,
    pub version: Option<String>,
    pub url_or_path: Option<String>,
}

pub fn install_app(
    config: &RuntimeConfig,
    app_reference: &str,
    options: &InstallOptions,
) -> anyhow::Result<InstallOutcome> {
    let mut outcomes = install_apps(config, &[app_reference.to_owned()], options)?;
    debug_assert_eq!(outcomes.len(), 1);
    Ok(outcomes.remove(0))
}

pub(crate) fn install_app_allow_upgrade(
    config: &RuntimeConfig,
    app_reference: &str,
    options: &InstallOptions,
) -> anyhow::Result<InstallOutcome> {
    let parsed = ParsedAppReference::parse(app_reference)?;
    install_single_app_inner(config, &parsed, options, true)
}

pub fn install_apps(
    config: &RuntimeConfig,
    app_references: &[String],
    options: &InstallOptions,
) -> anyhow::Result<Vec<InstallOutcome>> {
    ensure_admin_for_global_install(options)?;
    purge_failed_installs(config, app_references, options)?;
    let plan = resolve_install_plan(config, app_references, options)?;
    let mut outcomes = Vec::new();
    for app_reference in plan {
        outcomes.push(install_single_app(config, &app_reference, options)?);
    }
    Ok(outcomes)
}

fn ensure_admin_for_global_install(options: &InstallOptions) -> anyhow::Result<()> {
    if !options.global {
        return Ok(());
    }
    if is_admin()? {
        return Ok(());
    }
    bail!("you need admin rights to install global apps");
}

pub(crate) fn is_admin() -> anyhow::Result<bool> {
    #[cfg(windows)]
    {
        is_elevated()
    }
    #[cfg(not(windows))]
    {
        Ok(true)
    }
}

fn purge_failed_installs(
    config: &RuntimeConfig,
    app_references: &[String],
    options: &InstallOptions,
) -> anyhow::Result<()> {
    for app_reference in app_references {
        let parsed = ParsedAppReference::parse(app_reference)?;
        for root in [config.paths().root(), config.global_paths().root()] {
            let app_dir = root.join("apps").join(&parsed.app);
            if !app_dir.exists() || !failed_install(root, &parsed.app)? {
                continue;
            }
            fs::remove_dir_all(&app_dir)
                .with_context(|| format!("failed to purge previous failed install {}", app_dir))?;
            let shims_dir = if options.global {
                config.global_paths().shims()
            } else {
                config.paths().shims()
            };
            let _ = remove_existing_path_if_exists(&shims_dir.join(format!("{}.cmd", parsed.app)));
        }
    }
    Ok(())
}

fn install_single_app(
    config: &RuntimeConfig,
    app_reference: &str,
    options: &InstallOptions,
) -> anyhow::Result<InstallOutcome> {
    let parsed = ParsedAppReference::parse(app_reference)?;
    install_single_app_inner(config, &parsed, options, false)
}

fn install_single_app_inner(
    config: &RuntimeConfig,
    parsed: &ParsedAppReference,
    options: &InstallOptions,
    allow_upgrade: bool,
) -> anyhow::Result<InstallOutcome> {
    let paths = if options.global {
        config.global_paths()
    } else {
        config.paths()
    };

    let installed_version = current_version(paths.root(), &parsed.app)?;
    if !allow_upgrade
        && let Some(version) = installed_version.as_ref()
        && (parsed.version.is_none() || parsed.version.as_deref() == Some(version.as_str()))
    {
        return Ok(InstallOutcome::AlreadyInstalled {
            app: parsed.app.clone(),
            version: version.clone(),
        });
    }

    let resolved = resolve_manifest_for_install(config, parsed)?;
    let Some(manifest) = resolved else {
        return Ok(InstallOutcome::MissingManifest {
            app: parsed.app.clone(),
            bucket: parsed.bucket.clone(),
        });
    };

    let manifest_version = manifest_version(&manifest.manifest)
        .context("Manifest doesn't specify a version.")?
        .to_owned();
    let version = effective_install_version(config, &manifest.manifest, &manifest_version)?;
    if let Some(installed_version) = installed_version
        && installed_version == version
    {
        return Ok(InstallOutcome::AlreadyInstalled {
            app: manifest.app.clone(),
            version: installed_version,
        });
    }
    let check_hash = options.check_hash && !is_nightly_manifest(&manifest.manifest);
    let architecture = choose_architecture(&manifest.manifest, options.architecture.as_deref())
        .with_context(|| format!("'{}' doesn't support current architecture!", manifest.app))?;
    let urls = arch_specific_strings(&manifest.manifest, &architecture, "url");
    if urls.is_empty() {
        bail!("manifest doesn't contain a downloadable URL");
    }
    let hashes = arch_specific_strings(&manifest.manifest, &architecture, "hash");
    if check_hash && !hashes.is_empty() && hashes.len() != urls.len() {
        bail!("manifest hash count doesn't match URL count");
    }

    let version_dir = paths.version_dir(&manifest.app, &version);
    if version_dir.exists() {
        fs::remove_dir_all(&version_dir)
            .with_context(|| format!("failed to remove existing {}", version_dir))?;
    }
    fs::create_dir_all(&version_dir)
        .with_context(|| format!("failed to create install directory {}", version_dir))?;

    let client = build_blocking_http_client()?;
    let downloaded = download_payloads(
        config,
        &client,
        DownloadPlan {
            app: &manifest.app,
            version: &version,
            urls: &urls,
            hashes: &hashes,
            version_dir: &version_dir,
        },
        check_hash,
        options.use_cache,
    )?;
    let downloaded_names: Vec<String> = downloaded
        .iter()
        .map(|payload| payload.filename.clone())
        .collect();

    let extract_dirs = arch_specific_strings(&manifest.manifest, &architecture, "extract_dir");
    let extract_tos = arch_specific_strings(&manifest.manifest, &architecture, "extract_to");
    let mut extracted_index = 0usize;
    for payload in &downloaded {
        if should_extract(config, &manifest.manifest, &architecture, payload) {
            let extract_dir = extract_dirs.get(extracted_index).map(String::as_str);
            let extract_to = extract_tos.get(extracted_index).map(String::as_str);
            extract_payload(
                config,
                &manifest.manifest,
                &architecture,
                payload,
                &version_dir,
                extract_dir,
                extract_to,
            )?;
            extracted_index += 1;
        }
    }

    let original_dir = version_dir.clone();
    let persist_dir = paths.persist().join(&manifest.app);
    let pre_install_context = InstallContext::new(
        manifest.app.clone(),
        version.clone(),
        architecture.clone(),
        options.global,
        InstallContextPaths {
            dir: version_dir.clone(),
            original_dir: original_dir.clone(),
            persist_dir: persist_dir.clone(),
        },
        manifest.manifest.clone(),
    );
    run_manifest_hook(
        HookType::PreInstall,
        &manifest.manifest,
        &architecture,
        &pre_install_context,
    )?;
    run_installer(
        &manifest.manifest,
        &architecture,
        &version_dir,
        &downloaded_names,
        &pre_install_context,
        &manifest.app,
    )?;

    let manifest_json = render_manifest_json(&manifest.manifest)?;
    fs::write(version_dir.join("manifest.json"), &manifest_json)
        .with_context(|| format!("failed to write manifest for {}", manifest.app))?;
    let install_info = render_manifest_json(&json!({
        "architecture": architecture,
        "bucket": manifest.bucket,
        "url": parsed.url_or_path,
    }))?;
    fs::write(version_dir.join("install.json"), install_info)
        .with_context(|| format!("failed to write install info for {}", manifest.app))?;

    let current_dir = activate_current_dir(
        paths.app_dir(&manifest.app),
        &version_dir,
        config.settings().no_junction.unwrap_or(false),
    )?;
    let current_context = InstallContext::new(
        manifest.app.clone(),
        manifest_version.clone(),
        architecture.clone(),
        options.global,
        InstallContextPaths {
            dir: current_dir.clone(),
            original_dir: original_dir.clone(),
            persist_dir: persist_dir.clone(),
        },
        manifest.manifest.clone(),
    );
    ensure_install_dir_not_in_path(&current_context, options.global)?;
    let shim_names = create_cmd_shims(paths.shims(), &manifest.manifest, &current_context)?;
    create_startmenu_shortcuts(&manifest.manifest, &current_context)?;
    install_psmodule(&manifest.manifest, &current_context, paths)?;
    env_add_paths(
        config,
        &manifest.manifest,
        &current_context,
        options.global,
        &architecture,
    )?;
    env_set_values(
        &manifest.manifest,
        &current_context,
        options.global,
        &architecture,
    )?;
    persist_data(&manifest.manifest, &current_context)?;
    let post_install_context = InstallContext::new(
        manifest.app.clone(),
        manifest_version.clone(),
        architecture.clone(),
        options.global,
        InstallContextPaths {
            dir: current_dir.clone(),
            original_dir,
            persist_dir,
        },
        manifest.manifest.clone(),
    );
    run_manifest_hook(
        HookType::PostInstall,
        &manifest.manifest,
        &architecture,
        &post_install_context,
    )?;
    let notes = manifest_notes(&manifest.manifest)
        .into_iter()
        .map(|note| post_install_context.substitute(&note))
        .collect();
    let suggestions = manifest_suggestions(&manifest.manifest);

    Ok(InstallOutcome::Installed(InstalledApp {
        app: manifest.app,
        version,
        architecture,
        bucket: manifest.bucket,
        shim_names,
        notes,
        suggestions,
    }))
}

fn resolve_install_plan(
    config: &RuntimeConfig,
    app_references: &[String],
    options: &InstallOptions,
) -> anyhow::Result<Vec<String>> {
    Ok(resolve_dependency_plan(config, app_references, options)?
        .into_iter()
        .map(|entry| entry.reference)
        .collect())
}

fn resolve_dependency_plan(
    config: &RuntimeConfig,
    app_references: &[String],
    options: &InstallOptions,
) -> anyhow::Result<Vec<DependencyPlanEntry>> {
    let mut ordered = Vec::new();
    let mut planned = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    for app_reference in app_references {
        append_dependency_plan(
            config,
            app_reference,
            options,
            false,
            &mut ordered,
            &mut planned,
            &mut visiting,
        )?;
    }
    Ok(ordered)
}

fn append_dependency_plan(
    config: &RuntimeConfig,
    app_reference: &str,
    options: &InstallOptions,
    required: bool,
    ordered: &mut Vec<DependencyPlanEntry>,
    planned: &mut BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    let parsed = ParsedAppReference::parse(app_reference)?;
    if parsed.version.is_some() || parsed.url_or_path.is_some() {
        let reference = app_reference.to_owned();
        if planned.insert(reference.clone()) {
            ordered.push(planned_dependency_entry(&parsed, None, reference));
        }
        return Ok(());
    }

    let resolved = resolve_manifest_for_install(config, &parsed)?;
    let canonical = resolved
        .as_ref()
        .map(|manifest| {
            manifest
                .bucket
                .as_deref()
                .map(|bucket| format!("{bucket}/{}", manifest.app))
                .unwrap_or_else(|| manifest.app.clone())
        })
        .unwrap_or_else(|| {
            parsed
                .bucket
                .as_deref()
                .map(|bucket| format!("{bucket}/{}", parsed.app))
                .unwrap_or(parsed.app.clone())
        });
    if required && resolved.is_none() {
        bail!("Couldn't find manifest for '{canonical}'.");
    }
    if planned.contains(&canonical) {
        return Ok(());
    }
    if !visiting.insert(canonical.clone()) {
        bail!("Circular dependency detected: '{canonical}'.");
    }

    if let Some(manifest) = &resolved {
        let architecture = choose_architecture(&manifest.manifest, options.architecture.as_deref())
            .with_context(|| format!("'{}' doesn't support current architecture!", manifest.app))?;
        if !options.independent {
            for dependency in manifest_dependencies(config, &manifest.manifest, &architecture) {
                append_dependency_plan(
                    config,
                    &dependency,
                    options,
                    true,
                    ordered,
                    planned,
                    visiting,
                )?;
            }
        }
    }

    visiting.remove(&canonical);
    if planned.insert(canonical.clone()) {
        ordered.push(planned_dependency_entry(
            &parsed,
            resolved.as_ref(),
            canonical,
        ));
    }
    Ok(())
}

fn planned_dependency_entry(
    parsed: &ParsedAppReference,
    resolved: Option<&ResolvedManifest>,
    reference: String,
) -> DependencyPlanEntry {
    let source = resolved
        .and_then(|manifest| manifest.bucket.clone())
        .or_else(|| parsed.bucket.clone())
        .or_else(|| parsed.url_or_path.clone())
        .unwrap_or_default();
    let name = resolved
        .map(|manifest| manifest.app.clone())
        .unwrap_or_else(|| parsed.app.clone());
    DependencyPlanEntry {
        reference,
        source,
        name,
    }
}

#[derive(Debug, Clone)]
struct ParsedAppReference {
    app: String,
    bucket: Option<String>,
    version: Option<String>,
    url_or_path: Option<String>,
}

impl ParsedAppReference {
    fn parse(value: &str) -> anyhow::Result<Self> {
        let (raw_reference, version) = match value.rsplit_once('@') {
            Some((left, right)) if !right.is_empty() => (left, Some(right.to_owned())),
            _ => (value, None),
        };

        let url_or_path = (raw_reference.starts_with("http://")
            || raw_reference.starts_with("https://")
            || raw_reference.starts_with("\\\\")
            || raw_reference.ends_with(".json")
                && (raw_reference.contains('\\') || raw_reference.contains('/')))
        .then(|| raw_reference.to_owned());

        if let Some(url_or_path) = url_or_path {
            let app = Utf8Path::new(&url_or_path)
                .file_stem()
                .unwrap_or("manifest")
                .to_owned();
            return Ok(Self {
                app,
                bucket: None,
                version,
                url_or_path: Some(url_or_path),
            });
        }

        let (bucket, app) = match raw_reference.split_once('/') {
            Some((bucket, app)) if !bucket.is_empty() && !app.is_empty() => {
                (Some(bucket.to_owned()), app.to_owned())
            }
            _ => (None, raw_reference.to_owned()),
        };

        Ok(Self {
            app,
            bucket,
            version,
            url_or_path: None,
        })
    }
}

pub(crate) fn parse_app_reference(value: &str) -> anyhow::Result<AppReferenceParts> {
    let parsed = ParsedAppReference::parse(value)?;
    Ok(AppReferenceParts {
        app: parsed.app,
        bucket: parsed.bucket,
        version: parsed.version,
        url_or_path: parsed.url_or_path,
    })
}

fn resolve_manifest_for_install(
    config: &RuntimeConfig,
    parsed: &ParsedAppReference,
) -> anyhow::Result<Option<ResolvedManifest>> {
    if let Some(source) = &parsed.url_or_path {
        let manifest = match load_manifest_from_source(parsed, source)? {
            Some(manifest) => manifest,
            None => return Ok(None),
        };
        if let Some(requested_version) = &parsed.version {
            let actual_version = manifest_version(&manifest.manifest).with_context(|| {
                format!(
                    "Manifest '{}' does not declare a version, cannot use @{requested_version}.",
                    parsed.app
                )
            })?;
            if actual_version != requested_version {
                bail!(
                    "Version mismatch for manifest '{}': requested {requested_version}, found {actual_version}.",
                    parsed.app
                );
            }
        }
        return Ok(Some(manifest));
    }
    if let Some(version) = &parsed.version {
        return resolve_versioned_manifest(config, parsed, version);
    }
    let reference = match &parsed.bucket {
        Some(bucket) => format!("{bucket}/{}", parsed.app),
        None => parsed.app.clone(),
    };
    let mut manifest = crate::compat::catalog::resolve_manifest(config, &reference)?;
    if let Some(resolved) = &manifest
        && resolved.path.ends_with("current/manifest.json")
    {
        manifest = None;
    }
    Ok(manifest)
}

pub(crate) fn resolve_manifest_reference_for_install(
    config: &RuntimeConfig,
    app_reference: &str,
) -> anyhow::Result<Option<ResolvedManifest>> {
    let parsed = ParsedAppReference::parse(app_reference)?;
    resolve_manifest_for_install(config, &parsed)
}

fn load_manifest_from_source(
    parsed: &ParsedAppReference,
    source: &str,
) -> anyhow::Result<Option<ResolvedManifest>> {
    let manifest = if source.starts_with("http://") || source.starts_with("https://") {
        let client = build_blocking_http_client()?;
        client
            .get(source)
            .send()
            .with_context(|| format!("failed to download manifest {source}"))?
            .error_for_status()
            .with_context(|| format!("failed to download manifest {source}"))?
            .json::<Value>()
            .with_context(|| format!("failed to parse manifest {source}"))?
    } else {
        let source_path = Utf8Path::new(source);
        let contents = fs::read_to_string(source_path)
            .with_context(|| format!("failed to read manifest {}", source_path))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse manifest {}", source_path))?
    };
    Ok(Some(ResolvedManifest {
        app: parsed.app.clone(),
        bucket: None,
        path: Utf8PathBuf::from(source),
        manifest,
    }))
}

fn resolve_versioned_manifest(
    config: &RuntimeConfig,
    parsed: &ParsedAppReference,
    requested_version: &str,
) -> anyhow::Result<Option<ResolvedManifest>> {
    if let Some(bucket) = &parsed.bucket {
        return resolve_versioned_manifest_in_bucket(
            config,
            bucket,
            &parsed.app,
            requested_version,
        );
    }

    let buckets_root = config.paths().buckets();
    if !buckets_root.exists() {
        return Ok(None);
    }
    let mut buckets = fs::read_dir(&buckets_root)
        .with_context(|| format!("failed to read buckets directory {}", buckets_root))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("failed to enumerate buckets directory {}", buckets_root))?;
    buckets.sort_by_key(|entry| entry.file_name());
    for bucket in buckets {
        let bucket_name = bucket.file_name().to_string_lossy().into_owned();
        if let Some(manifest) = resolve_versioned_manifest_in_bucket(
            config,
            &bucket_name,
            &parsed.app,
            requested_version,
        )? {
            return Ok(Some(manifest));
        }
    }
    Ok(None)
}

fn resolve_versioned_manifest_in_bucket(
    config: &RuntimeConfig,
    bucket: &str,
    app: &str,
    requested_version: &str,
) -> anyhow::Result<Option<ResolvedManifest>> {
    for candidate in install_bucket_manifest_candidates(config, bucket, app) {
        if !candidate.is_file() {
            continue;
        }
        let current = load_manifest_json(&candidate)?;
        if let Some(resolved) =
            resolve_historical_manifest(config, app, &candidate, &current, requested_version)?
        {
            return Ok(Some(ResolvedManifest {
                app: app.to_owned(),
                bucket: Some(bucket.to_owned()),
                path: candidate,
                manifest: resolved.manifest,
            }));
        }
    }
    Ok(None)
}

fn install_bucket_manifest_candidates(
    config: &RuntimeConfig,
    bucket: &str,
    app: &str,
) -> [Utf8PathBuf; 2] {
    let bucket_root = config.paths().buckets().join(bucket);
    [
        bucket_root.join("bucket").join(format!("{app}.json")),
        bucket_root.join("deprecated").join(format!("{app}.json")),
    ]
}

pub(crate) fn load_manifest_json(path: &Utf8Path) -> anyhow::Result<Value> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("failed to read manifest {}", path))?;
    serde_json::from_str(&contents).with_context(|| format!("failed to parse manifest {}", path))
}

pub(crate) fn manifest_version(manifest: &Value) -> Option<&str> {
    manifest.get("version").and_then(Value::as_str)
}

pub(crate) fn is_nightly_manifest(manifest: &Value) -> bool {
    manifest_version(manifest) == Some("nightly")
}

pub(crate) fn effective_install_version(
    _config: &RuntimeConfig,
    manifest: &Value,
    fallback: &str,
) -> anyhow::Result<String> {
    if !is_nightly_manifest(manifest) {
        return Ok(fallback.to_owned());
    }
    Ok(format!("nightly-{}", nightly_stamp()?))
}

fn nightly_stamp() -> anyhow::Result<String> {
    Ok(jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y%m%d")
        .to_string())
}

pub(crate) fn current_version(root: &Utf8Path, app: &str) -> anyhow::Result<Option<String>> {
    let current_manifest = root
        .join("apps")
        .join(app)
        .join("current")
        .join("manifest.json");
    if current_manifest.is_file() {
        let source = fs::read_to_string(&current_manifest)
            .with_context(|| format!("failed to read current manifest {}", current_manifest))?;
        let manifest: Value = serde_json::from_str(&source)
            .with_context(|| format!("failed to parse current manifest {}", current_manifest))?;
        if manifest_version(&manifest) == Some("nightly") {
            return Ok(installed_versions(root, app)?.pop());
        }
        return Ok(manifest_version(&manifest).map(str::to_owned));
    }
    Ok(installed_versions(root, app)?.pop())
}

pub(crate) fn installed_versions(root: &Utf8Path, app: &str) -> anyhow::Result<Vec<String>> {
    let app_dir = root.join("apps").join(app);
    if !app_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut versions = fs::read_dir(&app_dir)
        .with_context(|| format!("failed to read app directory {}", app_dir))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            (path.join("install.json").is_file() && path.file_name() != Some("current")).then_some(
                (
                    entry.metadata().ok()?.modified().ok()?,
                    path.file_name()?.to_owned(),
                ),
            )
        })
        .collect::<Vec<_>>();
    versions.sort_by_key(|(modified, _)| *modified);
    Ok(versions.into_iter().map(|(_, version)| version).collect())
}

pub(crate) fn failed_install(root: &Utf8Path, app: &str) -> anyhow::Result<bool> {
    let app_dir = root.join("apps").join(app);
    if !app_dir.exists() {
        return Ok(false);
    }
    let has_current = app_dir.join("current").exists();
    Ok(!(has_current && current_version(root, app)?.is_some()))
}

pub(crate) fn choose_architecture(manifest: &Value, requested: Option<&str>) -> Option<String> {
    let architecture = requested.unwrap_or(default_architecture());
    if arch_specific_value(manifest, architecture, "url").is_some() || manifest.get("url").is_some()
    {
        return Some(architecture.to_owned());
    }
    None
}

pub(crate) fn default_architecture() -> &'static str {
    let architecture = std::env::var("PROCESSOR_ARCHITECTURE")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if architecture.contains("arm64") {
        "arm64"
    } else if architecture == "x86" || architecture == "i386" {
        "32bit"
    } else {
        "64bit"
    }
}

pub(crate) fn arch_specific_value<'a>(
    manifest: &'a Value,
    architecture: &str,
    property: &str,
) -> Option<&'a Value> {
    manifest
        .get("architecture")
        .and_then(|value| value.get(architecture))
        .and_then(|value| value.get(property))
        .or_else(|| manifest.get(property))
}

pub(crate) fn arch_specific_strings(
    manifest: &Value,
    architecture: &str,
    property: &str,
) -> Vec<String> {
    match arch_specific_value(manifest, architecture, property) {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn manifest_dependencies(
    config: &RuntimeConfig,
    manifest: &Value,
    architecture: &str,
) -> Vec<String> {
    let mut dependencies = arch_specific_strings(manifest, architecture, "depends");
    dependencies.extend(installation_helpers(config, manifest, architecture));
    dependencies.sort();
    dependencies.dedup();
    dependencies
}

fn installation_helpers(
    config: &RuntimeConfig,
    manifest: &Value,
    architecture: &str,
) -> Vec<String> {
    let settings = config.settings();
    let urls = arch_specific_strings(manifest, architecture, "url");
    let installer_script = arch_specific_value(manifest, architecture, "installer")
        .and_then(|value| value.get("script"))
        .map(script_value)
        .unwrap_or_default();
    let script = format!(
        "{}\n{}\n{}",
        arch_specific_script(manifest, architecture, "pre_install"),
        installer_script,
        arch_specific_script(manifest, architecture, "post_install"),
    );
    let mut helpers = Vec::new();
    if (urls.iter().any(|url| requires_7zip(url)) || script.contains("Expand-7zipArchive "))
        && !settings.use_external_7zip.unwrap_or(false)
        && !helper_installed(config, "7zip")
    {
        helpers.push(String::from("7zip"));
    }
    if (urls
        .iter()
        .any(|url| url.to_ascii_lowercase().ends_with(".msi"))
        || script.contains("Expand-MsiArchive "))
        && settings.use_lessmsi.unwrap_or(false)
        && !helper_installed(config, "lessmsi")
    {
        helpers.push(String::from("lessmsi"));
    }
    if (manifest.get("innosetup").is_some() || script.contains("Expand-InnoArchive "))
        && !helper_installed(config, "innounp")
    {
        helpers.push(String::from("innounp"));
    }
    if script.contains("Expand-DarkArchive ") && !helper_installed(config, "dark") {
        helpers.push(String::from("dark"));
    }
    helpers
}

fn helper_installed(config: &RuntimeConfig, helper: &str) -> bool {
    helper_candidates(helper).iter().any(|candidate| {
        current_version(config.paths().root(), candidate)
            .ok()
            .flatten()
            .is_some()
    }) || helper_candidates(helper).iter().any(|candidate| {
        current_version(config.global_paths().root(), candidate)
            .ok()
            .flatten()
            .is_some()
    })
}

fn helper_candidates(helper: &str) -> &'static [&'static str] {
    match helper {
        "innounp" => &["innounp-unicode", "innounp"],
        "dark" => &["wixtoolset", "dark"],
        "7zip" => &["7zip"],
        "lessmsi" => &["lessmsi"],
        _ => &[],
    }
}

fn requires_7zip(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    [
        ".001", ".7z", ".bz2", ".bzip2", ".gz", ".img", ".iso", ".lzma", ".lzh", ".nupkg", ".rar",
        ".tar", ".tbz", ".tbz2", ".tgz", ".txz", ".tz2", ".taz", ".xz", ".zst",
    ]
    .iter()
    .any(|extension| url.contains(extension))
}

pub(crate) fn run_manifest_hook(
    hook_type: HookType,
    manifest: &Value,
    architecture: &str,
    context: &InstallContext,
) -> anyhow::Result<()> {
    let script = arch_specific_script(manifest, architecture, hook_type.as_str());
    if script.trim().is_empty() {
        return Ok(());
    }
    run_install_hook(hook_type, &script, context)
}

pub(crate) fn arch_specific_script(manifest: &Value, architecture: &str, property: &str) -> String {
    arch_specific_strings(manifest, architecture, property).join("\r\n")
}

fn run_installer(
    manifest: &Value,
    architecture: &str,
    version_dir: &Utf8Path,
    downloaded_names: &[String],
    context: &InstallContext,
    app_name: &str,
) -> anyhow::Result<()> {
    let Some(installer) = arch_specific_value(manifest, architecture, "installer") else {
        return Ok(());
    };

    if let Some(script) = installer
        .get("script")
        .map(script_value)
        .filter(|script| !script.trim().is_empty())
    {
        run_install_hook(HookType::Installer, &script, context)?;
    }

    let file = installer.get("file").and_then(Value::as_str);
    let args = installer
        .get("args")
        .map(|value| argument_values(value, context))
        .unwrap_or_default();
    if file.is_none() && args.is_empty() {
        return Ok(());
    }

    let file_name = file
        .map(str::to_owned)
        .or_else(|| downloaded_names.first().cloned())
        .context("installer filename could not be determined")?;
    let program = version_dir.join(&file_name);
    if !is_in_dir(version_dir, &program) {
        bail!(
            "Error in manifest: Installer {} is outside the app directory.",
            program
        );
    }
    if !program.exists() {
        bail!("Installer {} is missing.", program);
    }

    let success = if program
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ps1"))
    {
        Command::new("pwsh")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                program.as_str(),
            ])
            .args(&args)
            .status()
            .with_context(|| format!("failed to run installer {}", program))?
            .success()
    } else {
        Command::new(program.as_str())
            .args(&args)
            .status()
            .with_context(|| format!("failed to run installer {}", program))?
            .success()
    };
    if !success {
        bail!(
            "Installation aborted. You might need to run 'scoop uninstall {app_name}' before trying again."
        );
    }

    if !installer
        .get("keep")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let _ = fs::remove_file(&program);
    }

    Ok(())
}

pub(crate) fn script_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\r\n"),
        _ => String::new(),
    }
}

pub(crate) fn argument_values(value: &Value, context: &InstallContext) -> Vec<String> {
    match value {
        Value::String(value) => vec![context.substitute(value)],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(|value| context.substitute(value))
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn ensure_install_dir_not_in_path(
    context: &InstallContext,
    global: bool,
) -> anyhow::Result<()> {
    let dir = windows_path(context.dir())
        .trim_end_matches('\\')
        .to_owned();
    let patterns = vec![dir.clone(), format!("{dir}\\*")];
    let scope = if global {
        EnvScope::System
    } else {
        EnvScope::User
    };
    let _ = remove_path(scope, "PATH", &patterns)?;
    Ok(())
}

pub(crate) fn env_add_paths(
    config: &RuntimeConfig,
    manifest: &Value,
    context: &InstallContext,
    global: bool,
    architecture: &str,
) -> anyhow::Result<()> {
    let additions = arch_specific_strings(manifest, architecture, "env_add_path");
    if additions.is_empty() {
        return Ok(());
    }

    let mut paths = Vec::new();
    for addition in additions {
        let candidate = context.dir().join(&addition);
        if is_in_dir(context.dir(), &candidate) {
            paths.push(windows_path(&candidate));
        }
    }
    if paths.is_empty() {
        return Ok(());
    }

    let settings = config.settings();
    let target_env_var = scoop_path_env_var(&settings);
    let scope = if global {
        EnvScope::System
    } else {
        EnvScope::User
    };
    if target_env_var != "PATH" {
        add_path(scope, "PATH", &[format!("%{target_env_var}%")], false)?;
    }
    add_path(scope, &target_env_var, &paths, true)
}

pub(crate) fn env_set_values(
    manifest: &Value,
    context: &InstallContext,
    global: bool,
    architecture: &str,
) -> anyhow::Result<()> {
    let Some(env_set) = arch_specific_value(manifest, architecture, "env_set") else {
        return Ok(());
    };
    let Some(object) = env_set.as_object() else {
        return Ok(());
    };
    let scope = if global {
        EnvScope::System
    } else {
        EnvScope::User
    };
    for (name, value) in object {
        let Some(value) = value.as_str() else {
            continue;
        };
        set_env_var(scope, name, Some(&context.substitute(value)))?;
    }
    Ok(())
}

pub(crate) fn create_startmenu_shortcuts(
    manifest: &Value,
    context: &InstallContext,
) -> anyhow::Result<()> {
    let Some(shortcuts) = arch_specific_value(manifest, context.architecture(), "shortcuts") else {
        return Ok(());
    };
    let Value::Array(shortcuts) = shortcuts else {
        return Ok(());
    };
    for shortcut in shortcuts {
        let Value::Array(parts) = shortcut else {
            continue;
        };
        let Some(target_rel) = parts.first().and_then(Value::as_str) else {
            continue;
        };
        let Some(name) = parts.get(1).and_then(Value::as_str) else {
            continue;
        };
        let target = context.dir().join(target_rel);
        if !target.is_file() {
            continue;
        }
        let arguments = parts
            .get(2)
            .and_then(Value::as_str)
            .map(|value| context.substitute(value))
            .unwrap_or_default();
        let icon = parts
            .get(3)
            .and_then(Value::as_str)
            .map(|value| windows_path(&context.dir().join(value)));
        create_shortcut(
            &windows_path(&target),
            name,
            &arguments,
            icon.as_deref(),
            context.global(),
        )?;
    }
    Ok(())
}

pub(crate) fn install_psmodule(
    manifest: &Value,
    context: &InstallContext,
    paths: &crate::domain::paths::ScoopPaths,
) -> anyhow::Result<()> {
    let Some(psmodule) = manifest.get("psmodule").and_then(Value::as_object) else {
        return Ok(());
    };
    let Some(module_name) = psmodule.get("name").and_then(Value::as_str) else {
        bail!("Invalid manifest: The 'name' property is missing from 'psmodule'.");
    };
    let modules_dir = paths.root().join("modules");
    fs::create_dir_all(&modules_dir)
        .with_context(|| format!("failed to create modules directory {}", modules_dir))?;
    let scope = if context.global() {
        EnvScope::System
    } else {
        EnvScope::User
    };
    let modules_dir_windows = windows_path(&modules_dir);
    let current_path = get_env_var(scope, "PSModulePath")?;
    if !current_path
        .as_deref()
        .is_some_and(|value| value.contains(&modules_dir_windows))
    {
        let updated = match current_path {
            Some(path) if !path.is_empty() => format!("{modules_dir_windows};{path}"),
            _ => modules_dir_windows.clone(),
        };
        set_env_var(scope, "PSModulePath", Some(&updated))?;
    }
    let link_path = modules_dir.join(module_name);
    if link_path.exists() {
        remove_existing_path(&link_path)?;
    }
    create_directory_link(&link_path, context.dir())
}

pub(crate) fn persist_data(manifest: &Value, context: &InstallContext) -> anyhow::Result<()> {
    let Some(persist) = manifest.get("persist") else {
        return Ok(());
    };
    fs::create_dir_all(context.persist_dir()).with_context(|| {
        format!(
            "failed to create persist directory {}",
            context.persist_dir()
        )
    })?;
    for (source_rel, target_rel) in persist_entries(persist) {
        let source_rel = source_rel.trim_end_matches(['/', '\\']);
        if source_rel.is_empty() {
            continue;
        }
        let source = context.dir().join(source_rel);
        let target = context.persist_dir().join(&target_rel);
        if target.exists() {
            if source.exists() {
                let original = Utf8PathBuf::from(format!("{}.original", source));
                if original.exists() {
                    remove_existing_path(&original)?;
                }
                fs::rename(&source, &original)
                    .with_context(|| format!("failed to preserve existing source {}", source))?;
            }
        } else if source.exists() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create persist parent {}", parent))?;
            }
            fs::rename(&source, &target)
                .with_context(|| format!("failed to move persisted data to {}", target))?;
        } else {
            fs::create_dir_all(&target)
                .with_context(|| format!("failed to create persist target {}", target))?;
        }

        if let Some(parent) = source.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create source parent {}", parent))?;
        }
        if target.is_dir() {
            create_directory_link(&source, &target)?;
        } else {
            if source.exists() {
                remove_existing_path(&source)?;
            }
            fs::hard_link(&target, &source)
                .with_context(|| format!("failed to create persist hard link {}", source))?;
        }
    }
    Ok(())
}

pub(crate) fn persist_entries(value: &Value) -> Vec<(String, String)> {
    match value {
        Value::String(value) => vec![(value.clone(), value.clone())],
        Value::Array(values) => values
            .iter()
            .filter_map(|entry| match entry {
                Value::String(value) => Some((value.clone(), value.clone())),
                Value::Array(parts) if !parts.is_empty() => {
                    let source = parts.first()?.as_str()?.to_owned();
                    let target = parts
                        .get(1)
                        .and_then(Value::as_str)
                        .unwrap_or(&source)
                        .to_owned();
                    Some((source, target))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn is_in_dir(dir: &Utf8Path, check: &Utf8Path) -> bool {
    let base = windows_path(dir).trim_end_matches('\\').to_owned();
    let check = windows_path(check);
    check == base || check.starts_with(&(base + "\\"))
}

#[derive(Debug, Clone)]
struct DownloadedPayload {
    path: Utf8PathBuf,
    filename: String,
}

struct DownloadPlan<'a> {
    app: &'a str,
    version: &'a str,
    urls: &'a [String],
    hashes: &'a [String],
    version_dir: &'a Utf8Path,
}

fn download_payloads(
    config: &RuntimeConfig,
    client: &Client,
    plan: DownloadPlan<'_>,
    check_hash: bool,
    use_cache: bool,
) -> anyhow::Result<Vec<DownloadedPayload>> {
    let mut payloads = Vec::new();
    for (index, url) in plan.urls.iter().enumerate() {
        let filename = download_filename(url)?;
        let cache_path = canonical_cache_path(config, plan.app, plan.version, url)?;
        let destination = plan.version_dir.join(&filename);

        if use_cache {
            if !cache_path.is_file() {
                fetch_to_path(client, url, &cache_path)?;
            }
            fs::create_dir_all(
                destination
                    .parent()
                    .expect("download destination should have a parent"),
            )
            .with_context(|| format!("failed to create parent for {}", destination))?;
            fs::copy(&cache_path, &destination)
                .with_context(|| format!("failed to copy cached payload to {}", destination))?;
        } else {
            fetch_to_path(client, url, &destination)?;
        }

        if check_hash
            && let Some(expected) = plan.hashes.get(index)
            && let Err(error) = validate_hash(&destination, expected, plan.app, url)
        {
            let _ = remove_existing_path_if_exists(&destination);
            let _ = remove_existing_path_if_exists(&cache_path);
            return Err(error);
        }

        payloads.push(DownloadedPayload {
            path: destination,
            filename,
        });
    }

    Ok(payloads)
}

pub(crate) fn download_filename(url: &str) -> anyhow::Result<String> {
    if let Some(name) = Utf8Path::new(url).file_name()
        && !name.is_empty()
        && !url.starts_with("http://")
        && !url.starts_with("https://")
    {
        return Ok(name.to_owned());
    }

    let after_slash = url.rsplit('/').next().unwrap_or(url);
    let name = after_slash.split('?').next().unwrap_or(after_slash);
    if name.is_empty() {
        bail!("failed to determine download filename from {url}");
    }
    Ok(name.to_owned())
}

pub(crate) fn fetch_to_path(client: &Client, url: &str, path: &Utf8Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent))?;
    }

    if Utf8Path::new(url).is_file() {
        fs::copy(url, path).with_context(|| format!("failed to copy {} to {}", url, path))?;
        return Ok(());
    }

    let mut response = client
        .get(url)
        .send()
        .with_context(|| format!("failed to download {url}"))?
        .error_for_status()
        .with_context(|| format!("failed to download {url}"))?;
    let mut file =
        fs::File::create(path).with_context(|| format!("failed to create download {}", path))?;
    io::copy(&mut response, &mut file)
        .with_context(|| format!("failed to write download {}", path))?;
    Ok(())
}

pub(crate) fn validate_hash(
    path: &Utf8Path,
    expected: &str,
    app: &str,
    url: &str,
) -> anyhow::Result<()> {
    let (algorithm, expected) = expected.split_once(':').unwrap_or(("sha256", expected));
    if !algorithm.eq_ignore_ascii_case("sha256") {
        bail!("Hash type '{algorithm}' isn't supported.");
    }

    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected) {
        return Ok(());
    }

    bail!(
        "Hash check failed!\nApp:         {app}\nURL:         {url}\nExpected:    {}\nActual:      {}",
        expected.to_ascii_lowercase(),
        actual
    )
}

fn should_extract(
    config: &RuntimeConfig,
    manifest: &Value,
    architecture: &str,
    payload: &DownloadedPayload,
) -> bool {
    extractor_for_payload(config, manifest, architecture, payload).is_some()
}

fn extract_payload(
    config: &RuntimeConfig,
    manifest: &Value,
    architecture: &str,
    payload: &DownloadedPayload,
    destination: &Utf8Path,
    extract_dir: Option<&str>,
    extract_to: Option<&str>,
) -> anyhow::Result<()> {
    let destination = extract_destination(destination, extract_to);
    match extractor_for_payload(config, manifest, architecture, payload) {
        Some(Extractor::Zip { use_7zip }) => {
            extract_zip(config, &payload.path, &destination, extract_dir, use_7zip)
        }
        Some(Extractor::SevenZip) => extract_7zip(config, &payload.path, &destination, extract_dir),
        Some(Extractor::Msi) => extract_msi(config, &payload.path, &destination, extract_dir),
        Some(Extractor::Inno) => extract_inno(config, &payload.path, &destination, extract_dir),
        None => Ok(()),
    }
}

fn extract_destination(base: &Utf8Path, extract_to: Option<&str>) -> Utf8PathBuf {
    match extract_to.map(str::trim).filter(|value| !value.is_empty()) {
        Some(path) => base.join(path.trim_start_matches(['/', '\\'])),
        None => base.to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Extractor {
    Zip { use_7zip: bool },
    SevenZip,
    Msi,
    Inno,
}

fn extractor_for_payload(
    config: &RuntimeConfig,
    manifest: &Value,
    _architecture: &str,
    payload: &DownloadedPayload,
) -> Option<Extractor> {
    let path = payload.path.as_str().to_ascii_lowercase();
    if path.ends_with(".zip") {
        let settings = config.settings();
        let use_7zip = helper_installed(config, "7zip")
            || settings.use_external_7zip.unwrap_or(false) && which_binary("7z").is_some();
        return Some(Extractor::Zip { use_7zip });
    }
    if path.ends_with(".msi") {
        return Some(Extractor::Msi);
    }
    if path.ends_with(".exe") && manifest.get("innosetup").is_some() {
        return Some(Extractor::Inno);
    }
    if should_extract_filename(&path, true) {
        return Some(Extractor::SevenZip);
    }
    None
}

fn should_extract_filename(path: &str, assume_non_zip: bool) -> bool {
    if !assume_non_zip && path.to_ascii_lowercase().ends_with(".zip") {
        return true;
    }
    let url = path.to_ascii_lowercase();
    url.ends_with(".msi") || requires_7zip(&url)
}

fn extract_7zip(
    config: &RuntimeConfig,
    archive: &Utf8Path,
    destination: &Utf8Path,
    extract_dir: Option<&str>,
) -> anyhow::Result<()> {
    let Some(extractor) = find_7zip_binary(config) else {
        bail!("7zip extractor is required to unpack {}", archive);
    };
    let working_destination = extraction_workdir(destination, extract_dir);
    fs::create_dir_all(&working_destination).with_context(|| {
        format!(
            "failed to create extraction directory {}",
            working_destination
        )
    })?;
    let status = Command::new(&extractor)
        .args([
            "x",
            archive.as_str(),
            &format!("-o{}", working_destination),
            "-xr!*.nsis",
            "-y",
        ])
        .status()
        .with_context(|| format!("failed to launch 7zip extractor {}", extractor))?;
    if !status.success() {
        bail!("failed to extract archive {}", archive);
    }
    finalize_extraction(&working_destination, destination, extract_dir)?;
    let _ = remove_existing_path_if_exists(archive);
    Ok(())
}

fn find_7zip_binary(config: &RuntimeConfig) -> Option<String> {
    for root in [config.paths().root(), config.global_paths().root()] {
        let candidate = root
            .join("apps")
            .join("7zip")
            .join("current")
            .join("7z.exe");
        if candidate.is_file() {
            return Some(candidate.to_string());
        }
    }
    which_binary("7z").or_else(|| which_binary("7za"))
}

pub(crate) fn which_binary(name: &str) -> Option<String> {
    let output = Command::new("where").arg(name).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::to_owned)
}

fn extract_msi(
    config: &RuntimeConfig,
    archive: &Utf8Path,
    destination: &Utf8Path,
    extract_dir: Option<&str>,
) -> anyhow::Result<()> {
    let working_destination = extraction_workdir(destination, extract_dir);
    fs::create_dir_all(&working_destination).with_context(|| {
        format!(
            "failed to create extraction directory {}",
            working_destination
        )
    })?;
    let settings = config.settings();
    let success = if settings.use_lessmsi.unwrap_or(false) {
        if let Some(lessmsi) = find_lessmsi_binary(config) {
            Command::new(&lessmsi)
                .args(["x", archive.as_str(), &format!("{}\\", working_destination)])
                .status()
                .with_context(|| format!("failed to launch lessmsi {}", lessmsi))?
                .success()
        } else {
            false
        }
    } else {
        Command::new("msiexec.exe")
            .args([
                "/a",
                archive.as_str(),
                "/qn",
                &format!("TARGETDIR={}\\SourceDir", working_destination),
            ])
            .status()
            .context("failed to launch msiexec")?
            .success()
    };
    if !success {
        bail!("failed to extract archive {}", archive);
    }
    let source_dir = if working_destination.join("SourceDir").is_dir() {
        working_destination.join("SourceDir")
    } else {
        working_destination.clone()
    };
    finalize_extraction(&source_dir, destination, extract_dir)?;
    if source_dir != working_destination {
        let _ = remove_existing_path_if_exists(&working_destination);
    }
    let _ = remove_existing_path_if_exists(archive);
    Ok(())
}

fn find_lessmsi_binary(config: &RuntimeConfig) -> Option<String> {
    for root in [config.paths().root(), config.global_paths().root()] {
        let candidate = root
            .join("apps")
            .join("lessmsi")
            .join("current")
            .join("lessmsi.exe");
        if candidate.is_file() {
            return Some(candidate.to_string());
        }
    }
    which_binary("lessmsi")
}

fn extract_inno(
    config: &RuntimeConfig,
    archive: &Utf8Path,
    destination: &Utf8Path,
    extract_dir: Option<&str>,
) -> anyhow::Result<()> {
    let Some(extractor) = find_innounp_binary(config) else {
        bail!("innounp extractor is required to unpack {}", archive);
    };
    let working_destination = extraction_workdir(destination, extract_dir);
    fs::create_dir_all(&working_destination).with_context(|| {
        format!(
            "failed to create extraction directory {}",
            working_destination
        )
    })?;
    let filter = match extract_dir {
        Some(dir) if dir.starts_with('{') => format!("-c{dir}"),
        Some(dir) if !dir.is_empty() => format!("-c{{app}}\\{dir}"),
        _ => String::from("-c{app}"),
    };
    let status = Command::new(&extractor)
        .args([
            "-x",
            &format!("-d{}", working_destination),
            archive.as_str(),
            "-y",
            &filter,
        ])
        .status()
        .with_context(|| format!("failed to launch innounp extractor {}", extractor))?;
    if !status.success() {
        bail!("failed to extract archive {}", archive);
    }
    finalize_extraction(&working_destination, destination, extract_dir)?;
    let _ = remove_existing_path_if_exists(archive);
    Ok(())
}

fn find_innounp_binary(config: &RuntimeConfig) -> Option<String> {
    for root in [config.paths().root(), config.global_paths().root()] {
        for app in ["innounp-unicode", "innounp"] {
            let candidate = root
                .join("apps")
                .join(app)
                .join("current")
                .join("innounp.exe");
            if candidate.is_file() {
                return Some(candidate.to_string());
            }
        }
    }
    which_binary("innounp")
}

fn extraction_workdir(destination: &Utf8Path, extract_dir: Option<&str>) -> Utf8PathBuf {
    if extract_dir.is_some() {
        destination.join("_scoop_extract_tmp")
    } else {
        destination.to_owned()
    }
}

fn finalize_extraction(
    working_destination: &Utf8Path,
    destination: &Utf8Path,
    extract_dir: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(extract_dir) = extract_dir.map(str::trim).filter(|value| !value.is_empty()) {
        let source = working_destination.join(extract_dir.trim_matches(['/', '\\']));
        if !source.exists() {
            bail!("failed to locate extracted directory {}", source);
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create extraction parent {}", parent))?;
        }
        fs::create_dir_all(destination)
            .with_context(|| format!("failed to create extraction target {}", destination))?;
        move_contents(&source, destination)?;
        if working_destination.exists() && working_destination != destination {
            let _ = remove_existing_path_if_exists(working_destination);
        }
    }
    Ok(())
}

fn move_contents(from: &Utf8Path, to: &Utf8Path) -> anyhow::Result<()> {
    for entry in fs::read_dir(from).with_context(|| format!("failed to read directory {}", from))? {
        let entry = entry.with_context(|| format!("failed to read directory entry in {}", from))?;
        let source = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|_| anyhow::anyhow!("directory entry path should be valid UTF-8"))?;
        let destination = to.join(entry.file_name().to_string_lossy().as_ref());
        if destination.exists() {
            remove_existing_path(&destination)?;
        }
        fs::rename(&source, &destination)
            .with_context(|| format!("failed to move {} to {}", source, destination))?;
    }
    Ok(())
}

fn extract_zip(
    config: &RuntimeConfig,
    path: &Utf8Path,
    destination: &Utf8Path,
    extract_dir: Option<&str>,
    use_7zip: bool,
) -> anyhow::Result<()> {
    if use_7zip {
        return extract_7zip(config, path, destination, extract_dir);
    }
    let working_destination = extraction_workdir(destination, extract_dir);
    fs::create_dir_all(&working_destination).with_context(|| {
        format!(
            "failed to create extraction directory {}",
            working_destination
        )
    })?;
    let file = fs::File::open(path).with_context(|| format!("failed to open archive {}", path))?;
    let mut archive =
        ZipArchive::new(file).with_context(|| format!("failed to read ZIP archive {}", path))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("failed to access ZIP entry {} in {}", index, path))?;
        let Some(name) = entry.enclosed_name().map(|name| name.to_owned()) else {
            continue;
        };
        let output_path = working_destination.join(
            Utf8PathBuf::from_path_buf(name)
                .map_err(|_| anyhow::anyhow!("ZIP entry path should be valid UTF-8"))?,
        );

        if entry.is_dir() {
            fs::create_dir_all(&output_path)
                .with_context(|| format!("failed to create directory {}", output_path))?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent))?;
        }
        let mut output = fs::File::create(&output_path)
            .with_context(|| format!("failed to create extracted file {}", output_path))?;
        io::copy(&mut entry, &mut output)
            .with_context(|| format!("failed to extract ZIP entry to {}", output_path))?;
    }

    finalize_extraction(&working_destination, destination, extract_dir)?;
    let _ = remove_existing_path_if_exists(path);
    Ok(())
}

pub(crate) fn activate_current_dir(
    app_dir: Utf8PathBuf,
    version_dir: &Utf8Path,
    no_junction: bool,
) -> anyhow::Result<Utf8PathBuf> {
    if no_junction {
        return Ok(version_dir.to_owned());
    }
    let current_dir = app_dir.join("current");
    if current_dir.exists() {
        remove_existing_path(&current_dir)?;
    }

    if let Err(error) = create_current_link(&current_dir, version_dir) {
        copy_dir_recursive(version_dir, &current_dir).with_context(|| {
            format!(
                "failed to create current activation at {} after link creation failed: {}",
                current_dir, error
            )
        })?;
    }

    Ok(current_dir)
}

pub(crate) fn remove_existing_path_if_exists(path: &Utf8Path) -> anyhow::Result<()> {
    if path.exists() {
        remove_existing_path(path)?;
    }
    Ok(())
}

pub(crate) fn remove_existing_path(path: &Utf8Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to read metadata for {}", path))?;
    #[cfg(windows)]
    {
        use std::{os::windows::fs::MetadataExt, process::Command};

        const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

        let attributes = metadata.file_attributes();
        if attributes & FILE_ATTRIBUTE_READONLY != 0 {
            let mut command = Command::new("cmd");
            command.args(["/C", "attrib", "-R", path.as_str()]);
            if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                command.arg("/L");
            }
            let status = command
                .status()
                .with_context(|| format!("failed to clear read-only attribute on {}", path))?;
            if !status.success() {
                bail!("failed to clear read-only attribute on {}", path);
            }
        }

        if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                match fs::remove_dir(path) {
                    Ok(()) => {}
                    Err(_) => {
                        let status = Command::new("cmd")
                            .args(["/C", "rmdir", path.as_str()])
                            .status()
                            .with_context(|| format!("failed to remove link {}", path))?;
                        if !status.success() {
                            bail!("failed to remove link {}", path);
                        }
                    }
                }
            } else {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(_) => {
                        let status = Command::new("cmd")
                            .args(["/C", "del", path.as_str()])
                            .status()
                            .with_context(|| format!("failed to remove link {}", path))?;
                        if !status.success() {
                            bail!("failed to remove link {}", path);
                        }
                    }
                }
            }
            return Ok(());
        }
    }

    if metadata.file_type().is_symlink() {
        fs::remove_dir(path).with_context(|| format!("failed to remove link {}", path))?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove directory {}", path))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove file {}", path))?;
    }
    Ok(())
}

fn create_current_link(current_dir: &Utf8Path, version_dir: &Utf8Path) -> anyhow::Result<()> {
    create_directory_link(current_dir, version_dir)
}

pub(crate) fn create_directory_link(
    link_path: &Utf8Path,
    target_dir: &Utf8Path,
) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_dir(target_dir.as_std_path(), link_path.as_std_path()) {
            Ok(()) => {}
            Err(_) => {
                let status = Command::new("cmd")
                    .args([
                        "/C",
                        "mklink",
                        "/J",
                        link_path.as_str(),
                        target_dir.as_str(),
                    ])
                    .status()
                    .with_context(|| format!("failed to create junction {}", link_path))?;
                if !status.success() {
                    bail!("failed to create junction {}", link_path);
                }
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(target_dir.as_std_path(), link_path.as_std_path())
            .with_context(|| format!("failed to create symlink {}", link_path))?;
        Ok(())
    }
}

pub(crate) fn copy_dir_recursive(from: &Utf8Path, to: &Utf8Path) -> anyhow::Result<()> {
    fs::create_dir_all(to).with_context(|| format!("failed to create directory {}", to))?;
    for entry in fs::read_dir(from).with_context(|| format!("failed to read directory {}", from))? {
        let entry = entry.with_context(|| format!("failed to read directory entry in {}", from))?;
        let source = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|_| anyhow::anyhow!("directory entry path should be valid UTF-8"))?;
        let destination = to.join(entry.file_name().to_string_lossy().as_ref());
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to read file type for {}", source))?;
        if file_type.is_dir() {
            copy_dir_recursive(&source, &destination)?;
        } else {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent))?;
            }
            fs::copy(&source, &destination)
                .with_context(|| format!("failed to copy {} to {}", source, destination))?;
        }
    }
    Ok(())
}

pub(crate) fn create_cmd_shims(
    shims_dir: Utf8PathBuf,
    manifest: &Value,
    context: &InstallContext,
) -> anyhow::Result<Vec<String>> {
    let Some(bin) = arch_specific_value(manifest, context.architecture(), "bin") else {
        return Ok(Vec::new());
    };
    fs::create_dir_all(&shims_dir)
        .with_context(|| format!("failed to create shims directory {}", shims_dir))?;

    let mut names = Vec::new();
    for shim in shim_entries(bin) {
        let shim_path = shims_dir.join(format!("{}.cmd", shim.name.to_ascii_lowercase()));
        let target = resolve_shim_target(context, &shim.target)
            .with_context(|| format!("Can't shim '{}'", shim.name))?;

        let invocation = match target.extension() {
            Some(extension) if extension.eq_ignore_ascii_case("ps1") => {
                format!(
                    "pwsh -NoProfile -ExecutionPolicy Bypass -File \"{}\"{} %*",
                    target,
                    shim.args
                        .as_deref()
                        .map(|args| format!(" {}", context.substitute(args)))
                        .unwrap_or_default()
                )
            }
            Some(extension)
                if extension.eq_ignore_ascii_case("cmd")
                    || extension.eq_ignore_ascii_case("bat") =>
            {
                format!(
                    "call \"{}\"{} %*",
                    target,
                    shim.args
                        .as_deref()
                        .map(|args| format!(" {}", context.substitute(args)))
                        .unwrap_or_default()
                )
            }
            _ => format!(
                "\"{}\"{} %*",
                target,
                shim.args
                    .as_deref()
                    .map(|args| format!(" {}", context.substitute(args)))
                    .unwrap_or_default()
            ),
        };
        let script = format!("@rem {}\r\n@echo off\r\n{}\r\n", target, invocation);
        fs::write(&shim_path, script)
            .with_context(|| format!("failed to write shim {}", shim_path))?;
        names.push(shim.name);
    }
    Ok(names)
}

pub(crate) fn resolve_shim_target(
    context: &InstallContext,
    target: &str,
) -> anyhow::Result<Utf8PathBuf> {
    let substituted = context.substitute(target);
    let candidate = Utf8PathBuf::from(&substituted);
    if candidate.is_file() {
        return Ok(candidate);
    }
    let in_dir = context.dir().join(&substituted);
    if in_dir.is_file() {
        return Ok(in_dir);
    }
    if let Some(found) = which_binary(&substituted) {
        return Ok(Utf8PathBuf::from(found));
    }
    bail!("couldn't find '{}'.", in_dir)
}

#[derive(Debug, Clone)]
pub(crate) struct ShimEntry {
    pub(crate) target: String,
    pub(crate) name: String,
    pub(crate) args: Option<String>,
}

pub(crate) fn shim_entries(value: &Value) -> Vec<ShimEntry> {
    match value {
        Value::String(target) => vec![ShimEntry {
            name: shim_name(target, None),
            target: target.clone(),
            args: None,
        }],
        Value::Array(entries) => entries
            .iter()
            .filter_map(|entry| match entry {
                Value::String(target) => Some(ShimEntry {
                    name: shim_name(target, None),
                    target: target.clone(),
                    args: None,
                }),
                Value::Array(parts) if !parts.is_empty() => {
                    let target = parts.first()?.as_str()?.to_owned();
                    let alias = parts.get(1).and_then(Value::as_str);
                    let args = parts.get(2).and_then(Value::as_str).map(str::to_owned);
                    Some(ShimEntry {
                        name: shim_name(&target, alias),
                        target,
                        args,
                    })
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn shim_name(target: &str, alias: Option<&str>) -> String {
    alias.map(str::to_owned).unwrap_or_else(|| {
        Utf8Path::new(target)
            .file_stem()
            .unwrap_or(target)
            .to_owned()
    })
}

fn manifest_notes(manifest: &Value) -> Vec<String> {
    match manifest.get("notes") {
        Some(Value::String(note)) => vec![note.clone()],
        Some(Value::Array(notes)) => notes
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn manifest_suggestions(manifest: &Value) -> Vec<String> {
    let Some(entries) = manifest.get("suggest").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut suggestions = Vec::new();
    for value in entries.values() {
        match value {
            Value::String(value) => suggestions.push(value.clone()),
            Value::Array(values) => {
                suggestions.extend(values.iter().filter_map(Value::as_str).map(str::to_owned))
            }
            _ => {}
        }
    }
    suggestions.sort();
    suggestions.dedup();
    suggestions
}

pub(crate) fn windows_path(path: &Utf8Path) -> String {
    path.as_str().replace('/', "\\")
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use camino::{Utf8Path, Utf8PathBuf};
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;

    use crate::RuntimeConfig;

    use super::{InstallOptions, InstallOutcome, install_app};

    #[test]
    fn installs_bucket_manifest_with_zip_payload_and_cmd_shim() {
        let fixture = Fixture::new();
        let archive = fixture.write_zip(
            "payloads\\demo.zip",
            &[("demo.exe", b"demo binary"), ("README.txt", b"hello")],
        );
        let hash = crate::infra::hash::sha256_file(&archive).expect("hash should compute");
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            &format!(
                r#"{{
                    "version":"1.2.3",
                    "url":"{}",
                    "hash":"{}",
                    "bin":"demo.exe",
                    "notes":["note 1","note 2"]
                }}"#,
                escape_json_path(&archive),
                hash
            ),
        );

        let outcome = install_app(&fixture.config(), "demo", &InstallOptions::default())
            .expect("install should succeed");

        let InstallOutcome::Installed(installed) = outcome else {
            panic!("expected installed outcome");
        };
        assert_eq!(installed.version, "1.2.3");
        assert_eq!(installed.shim_names, vec![String::from("demo")]);
        assert_eq!(
            installed.notes,
            vec![String::from("note 1"), String::from("note 2")]
        );
        assert!(
            fixture
                .local_root
                .join("apps")
                .join("demo")
                .join("1.2.3")
                .join("demo.exe")
                .is_file()
        );
        assert!(
            fixture
                .local_root
                .join("apps")
                .join("demo")
                .join("current")
                .join("demo.exe")
                .exists()
        );
        assert!(fixture.local_root.join("shims").join("demo.cmd").is_file());
    }

    #[test]
    fn refuses_hash_mismatch() {
        let fixture = Fixture::new();
        let archive = fixture.write_zip("payloads\\demo.zip", &[("demo.exe", b"demo binary")]);
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            &format!(
                r#"{{"version":"1.2.3","url":"{}","hash":"{}","bin":"demo.exe"}}"#,
                escape_json_path(&archive),
                "deadbeef"
            ),
        );

        let error = install_app(&fixture.config(), "demo", &InstallOptions::default())
            .expect_err("install should fail");
        assert!(error.to_string().contains("Hash check failed"));
    }

    #[cfg(windows)]
    #[test]
    fn runs_pre_and_post_install_hooks_with_upstream_variables() {
        let fixture = Fixture::new();
        let archive = fixture.write_zip("payloads\\demo.zip", &[("demo.exe", b"demo binary")]);
        let hash = crate::infra::hash::sha256_file(&archive).expect("hash should compute");
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            &format!(
                r#"{{
                    "version":"1.2.3",
                    "url":"{}",
                    "hash":"{}",
                    "bin":"demo.exe",
                    "pre_install":[
                        "Set-Content -Path (Join-Path $dir 'pre.txt') -Value ($dir + '|' + $original_dir + '|' + $persist_dir + '|' + $manifest.version)"
                    ],
                    "post_install":[
                        "Set-Content -Path (Join-Path $dir 'post.txt') -Value ($dir + '|' + $original_dir + '|' + $persist_dir + '|' + $app + '|' + $version + '|' + $architecture + '|' + $global)"
                    ]
                }}"#,
                escape_json_path(&archive),
                hash
            ),
        );

        install_app(&fixture.config(), "demo", &InstallOptions::default())
            .expect("install should succeed");

        let version_dir = fixture.local_root.join("apps").join("demo").join("1.2.3");
        let current_dir = fixture.local_root.join("apps").join("demo").join("current");
        let persist_dir = fixture.local_root.join("persist").join("demo");
        assert_eq!(
            fs::read_to_string(version_dir.join("pre.txt"))
                .expect("pre-install marker should be written")
                .trim(),
            format!(
                "{}|{}|{}|1.2.3",
                windows_path(&version_dir),
                windows_path(&version_dir),
                windows_path(&persist_dir),
            )
        );
        assert_eq!(
            fs::read_to_string(current_dir.join("post.txt"))
                .expect("post-install marker should be written")
                .trim(),
            format!(
                "{}|{}|{}|demo|1.2.3|64bit|False",
                windows_path(&current_dir),
                windows_path(&version_dir),
                windows_path(&persist_dir),
            )
        );
    }

    #[cfg(windows)]
    #[test]
    fn removes_upstream_style_current_junction_without_touching_target() {
        let fixture = Fixture::new();
        let app_dir = fixture.local_root.join("apps").join("demo");
        let target = app_dir.join("1.2.3");
        let current = app_dir.join("current");
        fs::create_dir_all(&target).expect("target directory should exist");
        fs::write(target.join("manifest.json"), "{}").expect("target file should exist");

        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", current.as_str(), target.as_str()])
            .status()
            .expect("mklink should run");
        assert!(status.success(), "mklink should create the junction");

        super::remove_existing_path(&current).expect("junction should be removable");

        assert!(
            !current.exists(),
            "current junction should be removed without leaving a broken path"
        );
        assert!(target.exists(), "target directory should remain untouched");
    }

    #[cfg(windows)]
    #[test]
    fn removes_read_only_upstream_style_current_junction_without_touching_target() {
        let fixture = Fixture::new();
        let app_dir = fixture.local_root.join("apps").join("demo");
        let target = app_dir.join("1.2.3");
        let current = app_dir.join("current");
        fs::create_dir_all(&target).expect("target directory should exist");
        fs::write(target.join("manifest.json"), "{}").expect("target file should exist");

        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", current.as_str(), target.as_str()])
            .status()
            .expect("mklink should run");
        assert!(status.success(), "mklink should create the junction");

        let status = std::process::Command::new("cmd")
            .args(["/C", "attrib", "+R", current.as_str(), "/L"])
            .status()
            .expect("attrib should run");
        assert!(
            status.success(),
            "attrib should mark the junction as read-only"
        );

        super::remove_existing_path(&current).expect("junction should be removable");

        assert!(
            !current.exists(),
            "current junction should be removed without leaving a broken path"
        );
        assert!(target.exists(), "target directory should remain untouched");
    }

    struct Fixture {
        _temp: TempDir,
        local_root: Utf8PathBuf,
        global_root: Utf8PathBuf,
        payload_root: Utf8PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should exist");
            let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            let local_root = root.join("local");
            let global_root = root.join("global");
            let payload_root = root.join("payloads");
            fs::create_dir_all(&local_root).expect("local root should exist");
            fs::create_dir_all(&global_root).expect("global root should exist");
            fs::create_dir_all(&payload_root).expect("payload root should exist");

            Self {
                _temp: temp,
                local_root,
                global_root,
                payload_root,
            }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::new(self.local_root.clone(), self.global_root.clone())
        }

        fn write(&self, scope: &str, relative_path: &str, content: &str) {
            let root = match scope {
                "local" => &self.local_root,
                "global" => &self.global_root,
                _ => panic!("unknown scope"),
            };
            let path = root.join(relative_path);
            fs::create_dir_all(path.parent().expect("fixture file should have a parent"))
                .expect("fixture parent should exist");
            fs::write(path, content).expect("fixture file should be written");
        }

        fn write_zip(&self, relative_path: &str, files: &[(&str, &[u8])]) -> Utf8PathBuf {
            let path = self.payload_root.join(relative_path);
            fs::create_dir_all(path.parent().expect("zip should have a parent"))
                .expect("zip parent should exist");
            let file = fs::File::create(&path).expect("zip file should be created");
            let mut writer = zip::ZipWriter::new(file);
            for (name, content) in files {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("zip entry should start");
                writer.write_all(content).expect("zip content should write");
            }
            writer.finish().expect("zip writer should finish");
            path
        }
    }

    fn escape_json_path(path: &Utf8Path) -> String {
        path.as_str().replace('\\', "\\\\")
    }

    fn windows_path(path: &Utf8Path) -> String {
        path.as_str().replace('/', "\\")
    }
}
