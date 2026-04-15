use std::{
    fs,
    time::{Instant, SystemTime},
};

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    ResolvedManifest, RuntimeConfig,
    compat::catalog::resolve_manifest,
    infra::{git::latest_author_for_path, http::build_blocking_http_client, profiling},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub name: String,
    pub deprecated: bool,
    pub description: Option<String>,
    pub version: String,
    pub source: Option<String>,
    pub website: Option<String>,
    pub license: Option<String>,
    pub dependencies: Option<Vec<String>>,
    pub updated_at: Option<SystemTime>,
    pub updated_by: Option<String>,
    pub manifest: Option<String>,
    pub installed_versions: Option<Vec<String>>,
    pub installed_size: Option<Vec<String>>,
    pub download_size: Option<String>,
    pub binaries: Option<Vec<String>>,
    pub shortcuts: Option<Vec<String>>,
    pub environment: Option<Vec<String>>,
    pub path_added: Option<Vec<String>>,
    pub suggestions: Option<Vec<String>>,
    pub notes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
struct InstallMetadata {
    #[serde(default)]
    architecture: Option<String>,
    #[serde(default)]
    bucket: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledState {
    global: bool,
    current_version: String,
    versions: Vec<String>,
    install_metadata: Option<InstallMetadata>,
    current_dir: Utf8PathBuf,
    current_version_dir: Utf8PathBuf,
    persist_dir: Utf8PathBuf,
    app_dir: Utf8PathBuf,
}

pub fn describe_app(
    config: &RuntimeConfig,
    app_reference: &str,
    verbose: bool,
) -> anyhow::Result<Option<AppInfo>> {
    let _profile = profiling::scope(format!("info total ({app_reference})"));

    let manifest_started = Instant::now();
    let mut manifest = match resolve_manifest(config, app_reference)? {
        Some(manifest) => manifest,
        None => return Ok(None),
    };
    profiling::emit_duration("info resolve_manifest", manifest_started.elapsed());

    let installed_started = Instant::now();
    let installed = resolve_installed_state(config, &manifest.app)?;
    profiling::emit_duration("info resolve_installed_state", installed_started.elapsed());

    let bucket_started = Instant::now();
    if let Some(bucket) = installed
        .as_ref()
        .and_then(|state| state.install_metadata.as_ref())
        .and_then(|metadata| metadata.bucket.as_deref())
        && let Some(bucket_manifest) =
            resolve_manifest(config, &format!("{bucket}/{}", manifest.app))?
    {
        manifest = bucket_manifest;
    }
    profiling::emit_duration("info bucket_manifest_override", bucket_started.elapsed());

    let source = installed
        .as_ref()
        .and_then(|state| {
            state
                .install_metadata
                .as_ref()
                .and_then(|metadata| metadata.bucket.clone().or_else(|| metadata.url.clone()))
        })
        .or_else(|| manifest.bucket.clone());
    let version = match &installed {
        Some(state) if state.current_version == manifest_version(&manifest.manifest) => {
            state.current_version.clone()
        }
        Some(state) => format!(
            "{} (Update to {} available)",
            state.current_version,
            manifest_version(&manifest.manifest)
        ),
        None => manifest_version(&manifest.manifest).to_owned(),
    };
    let manifest_path = windows_path(&manifest.path);
    let metadata_started = Instant::now();
    let updated_at = metadata_time(&manifest.path);
    profiling::emit_duration("info metadata_time", metadata_started.elapsed());
    let updated_by_started = Instant::now();
    let updated_by = resolve_updated_by(&manifest.path);
    profiling::emit_duration("info resolve_updated_by", updated_by_started.elapsed());
    let deprecated = windows_path(&manifest.path)
        .to_ascii_lowercase()
        .contains("\\deprecated\\");
    let path_context = path_context(config, &manifest, &installed, verbose);

    let render_started = Instant::now();
    let app_info = AppInfo {
        name: manifest.app.clone(),
        deprecated,
        description: value_to_string(manifest.manifest.get("description")),
        version,
        source,
        website: value_to_string(manifest.manifest.get("homepage"))
            .map(|value| value.trim_end_matches('/').to_owned()),
        license: format_license(manifest.manifest.get("license"), verbose),
        dependencies: value_to_lines(manifest.manifest.get("depends")),
        updated_at,
        updated_by,
        manifest: verbose.then_some(manifest_path),
        installed_versions: render_installed_versions(&installed, verbose),
        installed_size: if verbose {
            installed
                .as_ref()
                .and_then(|state| render_installed_size(config, state))
        } else {
            None
        },
        download_size: if verbose && installed.is_none() {
            render_download_size(config, &manifest).ok().flatten()
        } else {
            None
        },
        binaries: extract_binaries(&manifest, &installed),
        shortcuts: extract_shortcuts(&manifest, &installed),
        environment: extract_environment(&manifest, &installed, &path_context),
        path_added: extract_path_additions(&manifest, &installed, &path_context),
        suggestions: extract_suggestions(&manifest.manifest),
        notes: extract_notes(&manifest.manifest, &path_context),
    };
    profiling::emit_duration("info assemble_output", render_started.elapsed());

    Ok(Some(app_info))
}

pub fn render_info(info: &AppInfo) -> anyhow::Result<String> {
    let mut fields = Vec::new();
    let name = if info.deprecated {
        format!("{} (DEPRECATED)", info.name)
    } else {
        info.name.clone()
    };
    fields.push(("Name", vec![name]));
    if let Some(description) = &info.description {
        fields.push(("Description", vec![description.clone()]));
    }
    fields.push(("Version", vec![info.version.clone()]));
    if let Some(source) = &info.source {
        fields.push(("Source", vec![source.clone()]));
    }
    if let Some(website) = &info.website {
        fields.push(("Website", vec![website.clone()]));
    }
    if let Some(license) = &info.license {
        fields.push(("License", vec![license.clone()]));
    }
    if let Some(dependencies) = &info.dependencies
        && !dependencies.is_empty()
    {
        fields.push(("Dependencies", vec![dependencies.join(" | ")]));
    }
    if let Some(updated_at) = info.updated_at {
        fields.push(("Updated at", vec![format_updated_at(updated_at)?]));
    }
    if let Some(updated_by) = &info.updated_by {
        fields.push(("Updated by", vec![updated_by.clone()]));
    }
    if let Some(manifest) = &info.manifest {
        fields.push(("Manifest", vec![manifest.clone()]));
    }
    if let Some(installed_versions) = &info.installed_versions
        && !installed_versions.is_empty()
    {
        fields.push(("Installed", installed_versions.clone()));
    }
    if let Some(installed_size) = &info.installed_size
        && !installed_size.is_empty()
    {
        fields.push(("Installed size", installed_size.clone()));
    }
    if let Some(download_size) = &info.download_size {
        fields.push(("Download size", vec![download_size.clone()]));
    }
    if let Some(binaries) = &info.binaries
        && !binaries.is_empty()
    {
        fields.push(("Binaries", vec![binaries.join(" | ")]));
    }
    if let Some(shortcuts) = &info.shortcuts
        && !shortcuts.is_empty()
    {
        fields.push(("Shortcuts", vec![shortcuts.join(" | ")]));
    }
    if let Some(environment) = &info.environment
        && !environment.is_empty()
    {
        fields.push(("Environment", environment.clone()));
    }
    if let Some(path_added) = &info.path_added
        && !path_added.is_empty()
    {
        fields.push(("Path Added", path_added.clone()));
    }
    if let Some(suggestions) = &info.suggestions
        && !suggestions.is_empty()
    {
        fields.push(("Suggestions", vec![suggestions.join(" | ")]));
    }
    if let Some(notes) = &info.notes
        && !notes.is_empty()
    {
        fields.push(("Notes", notes.clone()));
    }

    let width = fields
        .iter()
        .map(|(label, _)| label.len())
        .max()
        .unwrap_or_default();
    let continuation_indent = " ".repeat(width + 3);

    let mut output = String::new();
    for (index, (label, values)) in fields.iter().enumerate() {
        let lines = values
            .iter()
            .flat_map(|value| value.split('\n'))
            .collect::<Vec<_>>();
        let first = lines.first().copied().unwrap_or_default();
        output.push_str(&format!("{label:<width$} : {first}\r\n", width = width));
        for line in lines.iter().skip(1) {
            output.push_str(&continuation_indent);
            output.push_str(line);
            output.push_str("\r\n");
        }
        if index + 1 == fields.len() {
            output.push_str("\r\n");
        }
    }

    Ok(output)
}

fn resolve_installed_state(
    config: &RuntimeConfig,
    app: &str,
) -> anyhow::Result<Option<InstalledState>> {
    let (global, root) = if config.global_paths().app_dir(app).exists() {
        (true, config.global_paths().root())
    } else if config.paths().app_dir(app).exists() {
        (false, config.paths().root())
    } else {
        return Ok(None);
    };

    let versions = list_installed_versions(root, app)?;
    if versions.is_empty() {
        return Ok(None);
    }

    let current_version = select_current_version(root, app)?.unwrap_or_else(|| {
        versions
            .last()
            .expect("installed versions should not be empty")
            .clone()
    });
    let current_version_dir = root.join("apps").join(app).join(&current_version);
    let install_metadata = read_install_metadata(&current_version_dir.join("install.json")).ok();

    Ok(Some(InstalledState {
        global,
        current_version,
        versions,
        install_metadata,
        current_dir: root.join("apps").join(app).join("current"),
        current_version_dir,
        persist_dir: root.join("persist").join(app),
        app_dir: root.join("apps").join(app),
    }))
}

fn list_installed_versions(root: &Utf8Path, app: &str) -> anyhow::Result<Vec<String>> {
    let app_dir = root.join("apps").join(app);
    if !app_dir.exists() {
        return Ok(Vec::new());
    }

    let mut versions = fs::read_dir(&app_dir)
        .with_context(|| format!("failed to read installed versions from {}", app_dir))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .and_then(|_| entry.file_name().into_string().ok())
        })
        .filter(|name| name != "current" && !name.starts_with('_'))
        .filter_map(|name| {
            let install_json = app_dir.join(&name).join("install.json");
            install_json
                .is_file()
                .then(|| metadata_time(&install_json).map(|time| (time, name)))
                .flatten()
        })
        .collect::<Vec<_>>();
    versions.sort_by_key(|(updated_at, _)| *updated_at);

    Ok(versions.into_iter().map(|(_, version)| version).collect())
}

fn select_current_version(root: &Utf8Path, app: &str) -> anyhow::Result<Option<String>> {
    let current_manifest = root
        .join("apps")
        .join(app)
        .join("current")
        .join("manifest.json");
    if current_manifest.is_file() {
        let manifest = load_json(&current_manifest)?;
        if let Some(version) = manifest.get("version").and_then(Value::as_str) {
            return Ok(Some(version.to_owned()));
        }
    }

    Ok(None)
}

fn read_install_metadata(path: &Utf8Path) -> anyhow::Result<InstallMetadata> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read install metadata {}", path))?;
    serde_json::from_str(&source)
        .with_context(|| format!("failed to parse install metadata {}", path))
}

fn render_installed_versions(
    installed: &Option<InstalledState>,
    verbose: bool,
) -> Option<Vec<String>> {
    let state = installed.as_ref()?;
    Some(
        state
            .versions
            .iter()
            .map(|version| {
                if verbose {
                    windows_path(&state.app_dir.join(version))
                } else if state.global {
                    format!("{version} *global*")
                } else {
                    version.clone()
                }
            })
            .collect(),
    )
}

fn render_installed_size(config: &RuntimeConfig, state: &InstalledState) -> Option<Vec<String>> {
    let _profile = profiling::scope(format!("info render_installed_size ({})", state.app_dir));
    let app_total = directory_size(&state.app_dir);
    let current_total = directory_size(&state.current_version_dir);
    let persist_total = directory_size(&state.persist_dir);
    let cache_total = cache_size(config, state.app_dir.file_name()?);
    let old_versions = app_total.saturating_sub(current_total);

    if persist_total + cache_total + old_versions == 0 {
        return Some(vec![format_filesize(current_total)]);
    }

    let mut lines = Vec::new();
    if current_total != 0 {
        lines.push(format!(
            "Current version:  {}",
            format_filesize(current_total)
        ));
    }
    if old_versions != 0 {
        lines.push(format!(
            "Old versions:     {}",
            format_filesize(old_versions)
        ));
    }
    if persist_total != 0 {
        lines.push(format!(
            "Persisted data:   {}",
            format_filesize(persist_total)
        ));
    }
    if cache_total != 0 {
        lines.push(format!(
            "Cached downloads: {}",
            format_filesize(cache_total)
        ));
    }
    lines.push(format!(
        "Total:            {}",
        format_filesize(app_total + persist_total + cache_total)
    ));
    Some(lines)
}

fn render_download_size(
    config: &RuntimeConfig,
    manifest: &ResolvedManifest,
) -> anyhow::Result<Option<String>> {
    let _profile = profiling::scope(format!("info render_download_size ({})", manifest.app));
    let Some(urls) = value_to_lines(arch_specific(
        &manifest.manifest,
        Some(default_architecture()),
        "url",
    )) else {
        return Ok(None);
    };
    let client = build_blocking_http_client()?;
    let cached = urls
        .iter()
        .any(|_| has_cached_download(config, &manifest.app, manifest_version(&manifest.manifest)));

    let mut total = 0_u64;
    for url in urls {
        match remote_content_length(&client, &url) {
            Ok(length) => total = total.saturating_add(length),
            Err(reason) => {
                let suffix = if cached {
                    " (latest version is cached)"
                } else {
                    ""
                };
                return Ok(Some(format!("Unknown ({reason}){suffix}")));
            }
        }
    }

    let suffix = if cached {
        " (latest version is cached)"
    } else {
        ""
    };
    Ok(Some(format!("{}{}", format_filesize(total), suffix)))
}

fn remote_content_length(client: &reqwest::blocking::Client, url: &str) -> Result<u64, String> {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| String::from("unknown host"));
    let response = client
        .head(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|_| format!("the server at {host} is down"))?;

    response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| format!("the server at {host} did not send a Content-Length header"))
}

fn has_cached_download(config: &RuntimeConfig, app: &str, version: &str) -> bool {
    let prefix = format!("{app}#{version}#");
    fs::read_dir(config.cache_dir())
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .any(|name| name.starts_with(&prefix))
}

fn cache_size(config: &RuntimeConfig, app: &str) -> u64 {
    let prefix = format!("{app}#");
    fs::read_dir(config.cache_dir())
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name().into_string().ok()?;
            file_name.starts_with(&prefix).then_some(entry.path())
        })
        .map(|path| {
            fs::metadata(path)
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        })
        .sum()
}

fn directory_size(path: &Utf8Path) -> u64 {
    if !path.exists() {
        return 0;
    }

    let mut total = 0_u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    if let Ok(path) = Utf8PathBuf::from_path_buf(path) {
                        stack.push(path);
                    }
                }
                Ok(kind) if kind.is_file() => {
                    total = total.saturating_add(
                        entry.metadata().map(|metadata| metadata.len()).unwrap_or(0),
                    );
                }
                _ => {}
            }
        }
    }
    total
}

fn extract_binaries(
    manifest: &ResolvedManifest,
    installed: &Option<InstalledState>,
) -> Option<Vec<String>> {
    arch_specific_value(manifest, installed, "bin")
        .and_then(value_to_binary_lines)
        .filter(|entries| !entries.is_empty())
}

fn extract_shortcuts(
    manifest: &ResolvedManifest,
    installed: &Option<InstalledState>,
) -> Option<Vec<String>> {
    match arch_specific_value(manifest, installed, "shortcuts")? {
        Value::Array(entries) => Some(
            entries
                .iter()
                .filter_map(|entry| entry.as_array())
                .filter_map(|entry| entry.get(1).and_then(Value::as_str))
                .map(str::to_owned)
                .collect(),
        )
        .filter(|entries: &Vec<String>| !entries.is_empty()),
        _ => None,
    }
}

fn extract_environment(
    manifest: &ResolvedManifest,
    installed: &Option<InstalledState>,
    context: &PathContext,
) -> Option<Vec<String>> {
    let Value::Object(entries) = arch_specific_value(manifest, installed, "env_set")? else {
        return None;
    };
    let mut variables = Vec::new();
    for (name, value) in entries {
        if let Some(value) = value.as_str() {
            variables.push(format!("{name} = {}", substitute_tokens(value, context)));
        }
    }
    (!variables.is_empty()).then_some(variables)
}

fn extract_path_additions(
    manifest: &ResolvedManifest,
    installed: &Option<InstalledState>,
    context: &PathContext,
) -> Option<Vec<String>> {
    let values = value_to_lines(arch_specific_value(manifest, installed, "env_add_path"))?;
    let paths = values
        .into_iter()
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value == "." {
                context.dir.clone()
            } else {
                format!("{}\\{}", context.dir, value.replace('/', "\\"))
            }
        })
        .collect::<Vec<_>>();
    (!paths.is_empty()).then_some(paths)
}

fn extract_suggestions(manifest: &Value) -> Option<Vec<String>> {
    let Value::Object(entries) = manifest.get("suggest")? else {
        return None;
    };
    let suggestions = entries
        .values()
        .filter_map(|value| value_to_lines(Some(value)))
        .flatten()
        .collect::<Vec<_>>();
    (!suggestions.is_empty()).then_some(suggestions)
}

fn extract_notes(manifest: &Value, context: &PathContext) -> Option<Vec<String>> {
    value_to_lines(manifest.get("notes")).map(|notes| {
        notes
            .into_iter()
            .map(|note| substitute_tokens(&note, context))
            .collect()
    })
}

fn arch_specific_value<'a>(
    manifest: &'a ResolvedManifest,
    installed: &Option<InstalledState>,
    property: &str,
) -> Option<&'a Value> {
    arch_specific(
        &manifest.manifest,
        installed
            .as_ref()
            .and_then(|state| state.install_metadata.as_ref())
            .and_then(|metadata| metadata.architecture.as_deref()),
        property,
    )
}

fn arch_specific<'a>(
    manifest: &'a Value,
    architecture: Option<&str>,
    property: &str,
) -> Option<&'a Value> {
    architecture
        .and_then(|architecture| {
            manifest
                .get("architecture")?
                .get(architecture)?
                .get(property)
        })
        .or_else(|| manifest.get(property))
}

fn value_to_binary_lines(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::String(entry) => Some(vec![entry.clone()]),
        Value::Array(entries) => Some(
            entries
                .iter()
                .filter_map(|entry| match entry {
                    Value::String(path) => Some(path.clone()),
                    Value::Array(parts) if parts.len() >= 2 => {
                        let path = parts.first()?.as_str()?;
                        let alias = parts.get(1)?.as_str()?;
                        let extension = Utf8Path::new(path)
                            .extension()
                            .map(|value| format!(".{value}"))
                            .unwrap_or_default();
                        Some(format!("{alias}{extension}"))
                    }
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    }
}

fn format_license(value: Option<&Value>, verbose: bool) -> Option<String> {
    match value? {
        Value::String(license) => {
            let url = license_url(license);
            if verbose && !license.contains("://") {
                Some(format!("{license} ({url})"))
            } else {
                Some(license.clone())
            }
        }
        Value::Object(details) => {
            let identifier = details.get("identifier")?.as_str()?;
            if verbose {
                let url = details
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| license_url(identifier));
                Some(format!("{identifier} ({url})"))
            } else {
                Some(identifier.to_owned())
            }
        }
        _ => None,
    }
}

fn license_url(identifier: &str) -> String {
    format!("https://spdx.org/licenses/{identifier}.html")
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

fn value_to_lines(value: Option<&Value>) -> Option<Vec<String>> {
    match value? {
        Value::String(line) => Some(vec![line.clone()]),
        Value::Array(lines) => Some(
            lines
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        ),
        _ => None,
    }
}

fn manifest_version(manifest: &Value) -> &str {
    manifest
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>")
}

fn metadata_time(path: &Utf8Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn load_json(path: &Utf8Path) -> anyhow::Result<Value> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read JSON file {}", path))?;
    serde_json::from_str(&source).with_context(|| format!("failed to parse JSON file {}", path))
}

fn format_updated_at(updated_at: SystemTime) -> anyhow::Result<String> {
    let timestamp =
        jiff::Timestamp::try_from(updated_at).context("failed to convert system time")?;
    let zoned = timestamp.to_zoned(jiff::tz::TimeZone::system());
    Ok(zoned.strftime("%-d/%m/%Y %-I:%M:%S %p").to_string())
}

fn resolve_updated_by(path: &Utf8Path) -> Option<String> {
    latest_author_for_path(path).or_else(|| std::env::var("USERNAME").ok())
}

#[derive(Debug, Clone)]
struct PathContext {
    dir: String,
    original_dir: String,
    persist_dir: String,
}

fn path_context(
    config: &RuntimeConfig,
    manifest: &ResolvedManifest,
    installed: &Option<InstalledState>,
    verbose: bool,
) -> PathContext {
    if verbose && let Some(installed) = installed {
        return PathContext {
            dir: windows_path(&installed.current_dir),
            original_dir: windows_path(&installed.current_version_dir),
            persist_dir: windows_path(&installed.persist_dir),
        };
    }

    let _ = config;
    let _ = manifest;
    PathContext {
        dir: String::from("<root>"),
        original_dir: String::from("<root>"),
        persist_dir: String::from("<root>"),
    }
}

fn substitute_tokens(value: &str, context: &PathContext) -> String {
    value
        .replace("$original_dir", &context.original_dir)
        .replace("$persist_dir", &context.persist_dir)
        .replace("$dir", &context.dir)
}

fn windows_path(path: &Utf8Path) -> String {
    path.as_str().replace('/', "\\")
}

fn default_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "64bit",
        "aarch64" => "arm64",
        _ => "32bit",
    }
}

fn format_filesize(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit]).replace(".0 ", " ")
    } else {
        format!("{value:.2} {}", UNITS[unit])
            .replace(".00 ", " ")
            .replace("0 ", " ")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{AppInfo, describe_app, render_info};

    #[test]
    fn describes_installed_app_using_bucket_manifest_when_available() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "buckets\\main\\bucket\\demo.json",
            r#"{
                "version":"1.2.3",
                "description":"Bucket demo",
                "homepage":"https://example.invalid/demo/",
                "license":"MIT",
                "bin":["demo.exe"],
                "shortcuts":[["demo.exe","Demo App"]],
                "env_set":{"DEMO_HOME":"$dir\\home"},
                "env_add_path":[".","bin"],
                "suggest":{"Editor":["vscode","vim"]},
                "notes":["note line 1","note line 2"]
            }"#,
        );
        fixture.write(
            "local",
            "apps\\demo\\current\\manifest.json",
            r#"{"version":"1.2.3","description":"Installed demo"}"#,
        );
        fixture.write(
            "local",
            "apps\\demo\\1.2.3\\install.json",
            r#"{"bucket":"main","architecture":"64bit"}"#,
        );

        let info = describe_app(&fixture.config(), "demo", false)
            .expect("info lookup should succeed")
            .expect("app should exist");

        assert_eq!(info.description.as_deref(), Some("Bucket demo"));
        assert_eq!(info.source.as_deref(), Some("main"));
        assert_eq!(
            info.website.as_deref(),
            Some("https://example.invalid/demo")
        );
        assert_eq!(info.license.as_deref(), Some("MIT"));
        assert_eq!(info.installed_versions, Some(vec![String::from("1.2.3")]));
        assert_eq!(info.binaries, Some(vec![String::from("demo.exe")]));
        assert_eq!(info.shortcuts, Some(vec![String::from("Demo App")]));
        assert_eq!(
            info.environment,
            Some(vec![String::from("DEMO_HOME = <root>\\home")])
        );
        assert_eq!(
            info.path_added,
            Some(vec![String::from("<root>"), String::from("<root>\\bin")])
        );
        assert_eq!(
            info.suggestions,
            Some(vec![String::from("vscode"), String::from("vim")])
        );
        assert_eq!(
            info.notes,
            Some(vec![
                String::from("note line 1"),
                String::from("note line 2")
            ])
        );
    }

    #[test]
    fn renders_multiline_fields_like_property_list() {
        let rendered = render_info(&AppInfo {
            name: String::from("demo"),
            deprecated: false,
            description: Some(String::from("Demo app")),
            version: String::from("1.2.3"),
            source: Some(String::from("main")),
            website: None,
            license: None,
            dependencies: None,
            updated_at: None,
            updated_by: None,
            manifest: None,
            installed_versions: Some(vec![String::from("1.2.3")]),
            installed_size: None,
            download_size: None,
            binaries: Some(vec![String::from("demo.exe")]),
            shortcuts: None,
            environment: None,
            path_added: None,
            suggestions: None,
            notes: Some(vec![
                String::from("note line 1"),
                String::from("note line 2"),
            ]),
        })
        .expect("info should render");

        assert!(rendered.contains("Name        : demo\r\n"));
        assert!(rendered.contains("Installed   : 1.2.3\r\n"));
        assert!(rendered.contains("Notes       : note line 1\r\n"));
        assert!(rendered.contains("              note line 2\r\n"));
    }

    struct Fixture {
        _temp: TempDir,
        local_root: Utf8PathBuf,
        global_root: Utf8PathBuf,
    }

    impl Fixture {
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
                local_root,
                global_root,
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
    }
}
