use anyhow::Context;
use reqwest::{
    StatusCode,
    blocking::Client,
    header::{ACCEPT, HeaderMap, HeaderValue},
};
use serde::Serialize;
use serde_json::Value;

use crate::{
    RuntimeConfig,
    app::install::{
        arch_specific_strings, choose_architecture, default_architecture,
        resolve_manifest_reference_for_install,
    },
};

pub const EXIT_UNSAFE: i32 = 2;
pub const EXIT_EXCEPTION: i32 = 4;
pub const EXIT_NO_INFO: i32 = 8;
pub const EXIT_NO_API_KEY: i32 = 16;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VirusTotalOptions {
    pub scan: bool,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VirusTotalReport {
    #[serde(rename = "App.Name")]
    pub app_name: String,
    #[serde(rename = "App.Url")]
    pub app_url: String,
    #[serde(rename = "App.Hash", skip_serializing_if = "Option::is_none")]
    pub app_hash: Option<String>,
    #[serde(rename = "App.HashType", skip_serializing_if = "Option::is_none")]
    pub app_hash_type: Option<String>,
    #[serde(rename = "App.Size", skip_serializing_if = "Option::is_none")]
    pub app_size: Option<String>,
    #[serde(rename = "FileReport.Url", skip_serializing_if = "Option::is_none")]
    pub file_report_url: Option<String>,
    #[serde(rename = "FileReport.Hash", skip_serializing_if = "Option::is_none")]
    pub file_report_hash: Option<String>,
    #[serde(
        rename = "FileReport.Malicious",
        skip_serializing_if = "Option::is_none"
    )]
    pub file_report_malicious: Option<Value>,
    #[serde(
        rename = "FileReport.Suspicious",
        skip_serializing_if = "Option::is_none"
    )]
    pub file_report_suspicious: Option<Value>,
    #[serde(rename = "FileReport.Timeout", skip_serializing_if = "Option::is_none")]
    pub file_report_timeout: Option<u64>,
    #[serde(
        rename = "FileReport.Undetected",
        skip_serializing_if = "Option::is_none"
    )]
    pub file_report_undetected: Option<u64>,
    #[serde(rename = "UrlReport.Url", skip_serializing_if = "Option::is_none")]
    pub url_report_url: Option<String>,
    #[serde(rename = "UrlReport.Hash", skip_serializing_if = "Option::is_none")]
    pub url_report_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirusTotalRun {
    pub exit_code: i32,
    pub lines: Vec<String>,
    pub reports: Vec<VirusTotalReport>,
}

#[derive(Debug, Default)]
struct RunState {
    exit_code: i32,
    lines: Vec<String>,
    reports: Vec<VirusTotalReport>,
}

impl RunState {
    fn finish(self) -> VirusTotalRun {
        VirusTotalRun {
            exit_code: self.exit_code,
            lines: self.lines,
            reports: self.reports,
        }
    }
}

enum SingleCheckResult {
    Completed {
        lines: Vec<String>,
        report: Option<Box<VirusTotalReport>>,
        unsafe_hit: bool,
        no_info: bool,
    },
    RecoverableError {
        line: String,
    },
    RateLimited {
        line: String,
    },
}

struct RenderedReport {
    lines: Vec<String>,
    report: VirusTotalReport,
    unsafe_hits: u64,
}

enum Lookup {
    Found(Value),
    NotFound,
    RateLimited,
    Error(String),
}

enum Submission {
    Submitted(Box<VirusTotalReport>),
    RateLimited,
    Error(String),
}

pub fn check_apps(
    config: &RuntimeConfig,
    apps: &[String],
    api_key: &str,
    options: &VirusTotalOptions,
) -> anyhow::Result<VirusTotalRun> {
    let client = crate::infra::http::build_blocking_http_client()?;
    let base_url = options
        .base_url
        .clone()
        .unwrap_or_else(|| String::from("https://www.virustotal.com/api/v3"));
    let mut state = RunState::default();

    for app in apps {
        let Some(manifest) = resolve_manifest_reference_for_install(config, app)? else {
            state.exit_code |= EXIT_NO_INFO;
            state.lines.push(format!("WARN  {app}: manifest not found"));
            continue;
        };

        let architecture = choose_architecture(&manifest.manifest, Some(default_architecture()))
            .unwrap_or_else(|| default_architecture().to_owned());
        let urls = arch_specific_strings(&manifest.manifest, &architecture, "url");
        if urls.is_empty() {
            state.exit_code |= EXIT_NO_INFO;
            state.lines.push(format!(
                "WARN  {}: manifest has no downloadable URL",
                manifest.app
            ));
            continue;
        }
        let hashes = arch_specific_strings(&manifest.manifest, &architecture, "hash");

        for (index, url) in urls.iter().enumerate() {
            let hash = hashes.get(index).map(String::as_str);
            match check_single_url(
                &client,
                &base_url,
                api_key,
                &manifest.app,
                url,
                hash,
                options.scan,
            )? {
                SingleCheckResult::Completed {
                    lines,
                    report,
                    unsafe_hit,
                    no_info,
                } => {
                    state.lines.extend(lines);
                    if unsafe_hit {
                        state.exit_code |= EXIT_UNSAFE;
                    }
                    if no_info {
                        state.exit_code |= EXIT_NO_INFO;
                    }
                    if let Some(report) = report {
                        state.reports.push(*report);
                    }
                }
                SingleCheckResult::RecoverableError { line } => {
                    state.exit_code |= EXIT_EXCEPTION;
                    state.lines.push(line);
                }
                SingleCheckResult::RateLimited { line } => {
                    state.exit_code |= EXIT_EXCEPTION;
                    state.lines.push(line);
                    return Ok(state.finish());
                }
            }
        }
    }

    Ok(state.finish())
}

fn check_single_url(
    client: &Client,
    base_url: &str,
    api_key: &str,
    app: &str,
    url: &str,
    configured_hash: Option<&str>,
    scan: bool,
) -> anyhow::Result<SingleCheckResult> {
    let mut lines = Vec::new();
    let parsed_hash = parse_configured_hash(configured_hash);

    if let Some((algorithm, hash)) = parsed_hash.as_ref() {
        if matches!(algorithm.as_str(), "md5" | "sha1" | "sha256") {
            match get_file_report(client, base_url, api_key, hash)? {
                Lookup::Found(report) => {
                    let rendered = render_file_report(app, url, parsed_hash.as_ref(), report);
                    return Ok(SingleCheckResult::Completed {
                        unsafe_hit: rendered.unsafe_hits > 0,
                        lines: rendered.lines,
                        report: Some(Box::new(rendered.report)),
                        no_info: false,
                    });
                }
                Lookup::NotFound => {
                    lines.push(format!(
                        "WARN  {app}: file report not found; falling back to URL lookup."
                    ));
                }
                Lookup::RateLimited => {
                    return Ok(SingleCheckResult::RateLimited {
                        line: format!(
                            "ERROR {app}: VirusTotal request failed: rate limit or quota exceeded."
                        ),
                    });
                }
                Lookup::Error(message) => {
                    return Ok(SingleCheckResult::RecoverableError {
                        line: format!("WARN  {app}: VirusTotal request failed: {message}"),
                    });
                }
            }
        } else {
            lines.push(format!(
                "WARN  {app}: unsupported hash {algorithm}; falling back to URL lookup."
            ));
        }
    } else {
        lines.push(format!(
            "WARN  {app}: hash not found; falling back to URL lookup."
        ));
    }

    match get_url_report(client, base_url, api_key, url)? {
        Lookup::Found(report) => {
            let rendered = render_url_report(app, url, parsed_hash.as_ref(), report);
            lines.extend(rendered.lines);
            if let Some(file_hash) = rendered.report.url_report_hash.as_deref() {
                match get_file_report(client, base_url, api_key, file_hash)? {
                    Lookup::Found(file_report) => {
                        let file_rendered =
                            render_file_report(app, url, parsed_hash.as_ref(), file_report);
                        lines.extend(file_rendered.lines);
                        let mut report = file_rendered.report;
                        report.url_report_url = rendered.report.url_report_url;
                        report.url_report_hash = rendered.report.url_report_hash;
                        return Ok(SingleCheckResult::Completed {
                            unsafe_hit: file_rendered.unsafe_hits > 0,
                            lines,
                            report: Some(Box::new(report)),
                            no_info: false,
                        });
                    }
                    Lookup::NotFound => {
                        lines.push(format!(
                            "WARN  {app}: related file report not found; manual file upload is required."
                        ));
                        return Ok(SingleCheckResult::Completed {
                            unsafe_hit: false,
                            lines,
                            report: Some(Box::new(rendered.report)),
                            no_info: false,
                        });
                    }
                    Lookup::RateLimited => {
                        return Ok(SingleCheckResult::RateLimited {
                            line: format!(
                                "ERROR {app}: VirusTotal request failed: rate limit or quota exceeded."
                            ),
                        });
                    }
                    Lookup::Error(message) => {
                        return Ok(SingleCheckResult::RecoverableError {
                            line: format!("WARN  {app}: VirusTotal request failed: {message}"),
                        });
                    }
                }
            }

            Ok(SingleCheckResult::Completed {
                unsafe_hit: false,
                lines,
                report: Some(Box::new(rendered.report)),
                no_info: false,
            })
        }
        Lookup::NotFound if scan => match submit_url_for_scan(client, base_url, api_key, app, url)?
        {
            Submission::Submitted(report) => {
                lines.push(format!("INFO  {app}: analysis in progress."));
                Ok(SingleCheckResult::Completed {
                    unsafe_hit: false,
                    lines,
                    report: Some(report),
                    no_info: false,
                })
            }
            Submission::RateLimited => Ok(SingleCheckResult::RateLimited {
                line: format!(
                    "ERROR {app}: VirusTotal request failed: rate limit or quota exceeded."
                ),
            }),
            Submission::Error(message) => Ok(SingleCheckResult::RecoverableError {
                line: format!("WARN  {app}: VirusTotal request failed: {message}"),
            }),
        },
        Lookup::NotFound => Ok(SingleCheckResult::Completed {
            unsafe_hit: false,
            lines: {
                lines.push(format!(
                    "WARN  {app}: not found; you can manually submit {url}"
                ));
                lines
            },
            report: None,
            no_info: true,
        }),
        Lookup::RateLimited => Ok(SingleCheckResult::RateLimited {
            line: format!("ERROR {app}: VirusTotal request failed: rate limit or quota exceeded."),
        }),
        Lookup::Error(message) => Ok(SingleCheckResult::RecoverableError {
            line: format!("WARN  {app}: VirusTotal request failed: {message}"),
        }),
    }
}

fn render_file_report(
    app: &str,
    url: &str,
    parsed_hash: Option<&(String, String)>,
    response: Value,
) -> RenderedReport {
    let stats = response
        .pointer("/data/attributes/last_analysis_stats")
        .cloned()
        .unwrap_or(Value::Null);
    let malicious = number_field(&stats, "malicious");
    let suspicious = number_field(&stats, "suspicious");
    let timeout = number_field(&stats, "timeout");
    let undetected = number_field(&stats, "undetected");
    let harmless = number_field(&stats, "harmless");
    let failure = number_field(&stats, "failure");
    let unsafe_hits = malicious + suspicious;
    let total = unsafe_hits + timeout + undetected + harmless + failure;
    let report_hash = response
        .pointer("/data/attributes/sha256")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let file_report_url = (!report_hash.is_empty())
        .then(|| format!("https://www.virustotal.com/gui/file/{report_hash}"));
    let size = response
        .pointer("/data/attributes/size")
        .and_then(Value::as_u64)
        .map(format_filesize);

    let mut lines = Vec::new();
    if total == 0 {
        lines.push(format!("INFO  {app}: analysis in progress."));
    } else {
        lines.push(format!(
            "{} {app}: {unsafe_hits}/{total}, see {}",
            if unsafe_hits == 0 { "INFO " } else { "WARN " },
            file_report_url
                .as_deref()
                .unwrap_or("https://www.virustotal.com/gui")
        ));
    }

    let malicious_engines = collect_engine_names(
        response.pointer("/data/attributes/last_analysis_results"),
        "malicious",
    );
    let suspicious_engines = collect_engine_names(
        response.pointer("/data/attributes/last_analysis_results"),
        "suspicious",
    );

    RenderedReport {
        lines,
        unsafe_hits,
        report: VirusTotalReport {
            app_name: app.to_owned(),
            app_url: url.to_owned(),
            app_hash: parsed_hash.map(|(_, hash)| hash.clone()),
            app_hash_type: parsed_hash.map(|(algorithm, _)| algorithm.clone()),
            app_size: size,
            file_report_url,
            file_report_hash: (!report_hash.is_empty()).then_some(report_hash),
            file_report_malicious: Some(as_engine_value(malicious_engines)),
            file_report_suspicious: Some(as_engine_value(suspicious_engines)),
            file_report_timeout: Some(timeout),
            file_report_undetected: Some(undetected),
            url_report_url: None,
            url_report_hash: None,
        },
    }
}

fn render_url_report(
    app: &str,
    url: &str,
    parsed_hash: Option<&(String, String)>,
    response: Value,
) -> RenderedReport {
    let id = response
        .pointer("/data/id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url_report_url =
        (!id.is_empty()).then(|| format!("https://www.virustotal.com/gui/url/{id}"));
    let url_report_hash = response
        .pointer("/data/attributes/last_http_response_content_sha256")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let last_analysis_date = response
        .pointer("/data/attributes/last_analysis_date")
        .and_then(Value::as_u64);

    let mut lines = vec![format!("INFO  {app}: URL report found.")];
    match (url_report_hash.as_deref(), last_analysis_date) {
        (Some(_), _) => lines.push(format!("INFO  {app}: related file report found.")),
        (None, Some(_)) => {
            lines.push(format!("INFO  {app}: related file report not found."));
            lines.push(format!(
                "WARN  {app}: manual file upload is required instead of URL submission."
            ));
        }
        (None, None) => lines.push(format!("INFO  {app}: analysis in progress.")),
    }

    RenderedReport {
        lines,
        unsafe_hits: 0,
        report: VirusTotalReport {
            app_name: app.to_owned(),
            app_url: url.to_owned(),
            app_hash: parsed_hash.map(|(_, hash)| hash.clone()),
            app_hash_type: parsed_hash.map(|(algorithm, _)| algorithm.clone()),
            app_size: None,
            file_report_url: None,
            file_report_hash: None,
            file_report_malicious: None,
            file_report_suspicious: None,
            file_report_timeout: None,
            file_report_undetected: None,
            url_report_url,
            url_report_hash,
        },
    }
}

fn submit_url_for_scan(
    client: &Client,
    base_url: &str,
    api_key: &str,
    app: &str,
    url: &str,
) -> anyhow::Result<Submission> {
    let response = client
        .post(format!("{base_url}/urls"))
        .headers(api_headers(api_key)?)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!("url={}", urlencoding(url)))
        .send()
        .context("failed to submit URL to VirusTotal")?;

    match response.status() {
        StatusCode::OK => {
            let payload: Value = response
                .json()
                .context("failed to parse VirusTotal URL submission response")?;
            let id = payload
                .pointer("/data/id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let report_id = id.rsplit('-').next().unwrap_or(id);
            Ok(Submission::Submitted(Box::new(VirusTotalReport {
                app_name: app.to_owned(),
                app_url: url.to_owned(),
                app_hash: None,
                app_hash_type: None,
                app_size: None,
                file_report_url: None,
                file_report_hash: None,
                file_report_malicious: None,
                file_report_suspicious: None,
                file_report_timeout: None,
                file_report_undetected: None,
                url_report_url: (!report_id.is_empty())
                    .then(|| format!("https://www.virustotal.com/gui/url/{report_id}")),
                url_report_hash: None,
            })))
        }
        StatusCode::TOO_MANY_REQUESTS | StatusCode::NO_CONTENT => Ok(Submission::RateLimited),
        status => Ok(Submission::Error(format!(
            "VirusTotal returned unexpected status {status}"
        ))),
    }
}

fn get_file_report(
    client: &Client,
    base_url: &str,
    api_key: &str,
    hash: &str,
) -> anyhow::Result<Lookup> {
    let response = client
        .get(format!("{base_url}/files/{hash}"))
        .headers(api_headers(api_key)?)
        .send()
        .context("failed to query VirusTotal file report")?;
    classify_lookup_response(response)
}

fn get_url_report(
    client: &Client,
    base_url: &str,
    api_key: &str,
    url: &str,
) -> anyhow::Result<Lookup> {
    let id = url_safe_base64(url.as_bytes());
    let response = client
        .get(format!("{base_url}/urls/{id}"))
        .headers(api_headers(api_key)?)
        .send()
        .context("failed to query VirusTotal URL report")?;
    classify_lookup_response(response)
}

fn classify_lookup_response(response: reqwest::blocking::Response) -> anyhow::Result<Lookup> {
    match response.status() {
        StatusCode::OK => response
            .json::<Value>()
            .map(Lookup::Found)
            .context("failed to parse VirusTotal response"),
        StatusCode::NOT_FOUND => Ok(Lookup::NotFound),
        StatusCode::TOO_MANY_REQUESTS | StatusCode::NO_CONTENT => Ok(Lookup::RateLimited),
        status => Ok(Lookup::Error(format!(
            "VirusTotal returned unexpected status {status}"
        ))),
    }
}

fn api_headers(api_key: &str) -> anyhow::Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(
        "x-apikey",
        HeaderValue::from_str(api_key).context("invalid VirusTotal API key header")?,
    );
    Ok(headers)
}

fn parse_configured_hash(configured_hash: Option<&str>) -> Option<(String, String)> {
    let value = configured_hash?.trim();
    if value.is_empty() {
        return None;
    }
    if let Some((algorithm, hash)) = value.split_once(':') {
        return Some((
            algorithm.trim().to_ascii_lowercase(),
            hash.trim().to_owned(),
        ));
    }

    match value.len() {
        32 => Some((String::from("md5"), value.to_owned())),
        40 => Some((String::from("sha1"), value.to_owned())),
        64 => Some((String::from("sha256"), value.to_owned())),
        _ => None,
    }
}

fn collect_engine_names(results: Option<&Value>, category: &str) -> Vec<String> {
    let Some(results) = results.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut names = results
        .iter()
        .filter(|(_, result)| result.get("category").and_then(Value::as_str) == Some(category))
        .map(|(engine, _)| engine.clone())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn as_engine_value(engines: Vec<String>) -> Value {
    Value::Array(engines.into_iter().map(Value::String).collect())
}

fn number_field(value: &Value, field: &str) -> u64 {
    value.get(field).and_then(Value::as_u64).unwrap_or_default()
}

fn format_filesize(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    if (bytes as f64) >= GB {
        format!("{:.1} GB", (bytes as f64) / GB)
    } else if (bytes as f64) >= MB {
        format!("{:.1} MB", (bytes as f64) / MB)
    } else if (bytes as f64) >= KB {
        format!("{:.1} KB", (bytes as f64) / KB)
    } else {
        format!("{bytes} B")
    }
}

fn urlencoding(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

fn url_safe_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::new();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let chunk = ((bytes[index] as u32) << 16)
            | ((bytes[index + 1] as u32) << 8)
            | bytes[index + 2] as u32;
        encoded.push(TABLE[((chunk >> 18) & 0x3f) as usize] as char);
        encoded.push(TABLE[((chunk >> 12) & 0x3f) as usize] as char);
        encoded.push(TABLE[((chunk >> 6) & 0x3f) as usize] as char);
        encoded.push(TABLE[(chunk & 0x3f) as usize] as char);
        index += 3;
    }

    match bytes.len() - index {
        1 => {
            let chunk = (bytes[index] as u32) << 16;
            encoded.push(TABLE[((chunk >> 18) & 0x3f) as usize] as char);
            encoded.push(TABLE[((chunk >> 12) & 0x3f) as usize] as char);
        }
        2 => {
            let chunk = ((bytes[index] as u32) << 16) | ((bytes[index + 1] as u32) << 8);
            encoded.push(TABLE[((chunk >> 18) & 0x3f) as usize] as char);
            encoded.push(TABLE[((chunk >> 12) & 0x3f) as usize] as char);
            encoded.push(TABLE[((chunk >> 6) & 0x3f) as usize] as char);
        }
        _ => {}
    }

    encoded
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, VecDeque},
        fs,
        io::{Read, Write},
        net::{Shutdown, TcpListener, TcpStream},
        path::PathBuf,
        sync::{Arc, Mutex, mpsc},
        thread,
        time::Duration,
    };

    use camino::Utf8PathBuf;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{
        EXIT_EXCEPTION, EXIT_NO_INFO, EXIT_UNSAFE, VirusTotalOptions, check_apps, url_safe_base64,
        urlencoding,
    };

    #[test]
    fn safe_file_report_has_zero_exit_code() {
        let hash = "a".repeat(64);
        let server = TestServer::spawn(vec![Route::json(
            "GET",
            format!("/api/v3/files/{hash}"),
            200,
            json!({
                "data": {
                    "attributes": {
                        "sha256": hash,
                        "size": 4,
                        "last_analysis_stats": {
                            "malicious": 0,
                            "suspicious": 0,
                            "undetected": 70,
                            "timeout": 0,
                            "harmless": 0,
                            "failure": 0
                        },
                        "last_analysis_results": {}
                    }
                }
            }),
        )]);
        let fixture = Fixture::new();
        let url = format!("{}/demo.zip", server.base_url());
        fixture.manifest(
            "demo",
            &json!({
                "version": "1.0.0",
                "url": url,
                "hash": format!("sha256:{hash}")
            }),
        );

        let run = check_apps(
            &fixture.config(),
            &[String::from("demo")],
            "demo-api-key",
            &VirusTotalOptions {
                base_url: Some(server.api_base_url()),
                ..VirusTotalOptions::default()
            },
        )
        .expect("check should succeed");

        assert_eq!(run.exit_code, 0);
        assert_eq!(run.reports.len(), 1);
        assert_eq!(
            run.reports[0].file_report_hash.as_deref(),
            Some(hash.as_str())
        );
        assert_eq!(
            server.requests()[0]
                .headers
                .get("x-apikey")
                .map(String::as_str),
            Some("demo-api-key")
        );
    }

    #[test]
    fn unsafe_file_report_sets_unsafe_exit_bit() {
        let hash = "b".repeat(64);
        let server = TestServer::spawn(vec![Route::json(
            "GET",
            format!("/api/v3/files/{hash}"),
            200,
            json!({
                "data": {
                    "attributes": {
                        "sha256": hash,
                        "size": 4,
                        "last_analysis_stats": {
                            "malicious": 1,
                            "suspicious": 1,
                            "undetected": 68,
                            "timeout": 0,
                            "harmless": 0,
                            "failure": 0
                        },
                        "last_analysis_results": {
                            "EngineA": {"category": "malicious"},
                            "EngineB": {"category": "suspicious"}
                        }
                    }
                }
            }),
        )]);
        let fixture = Fixture::new();
        let url = format!("{}/demo.zip", server.base_url());
        fixture.manifest(
            "demo",
            &json!({
                "version": "1.0.0",
                "url": url,
                "hash": format!("sha256:{hash}")
            }),
        );

        let run = check_apps(
            &fixture.config(),
            &[String::from("demo")],
            "demo-api-key",
            &VirusTotalOptions {
                base_url: Some(server.api_base_url()),
                ..VirusTotalOptions::default()
            },
        )
        .expect("check should succeed");

        assert_eq!(run.exit_code, EXIT_UNSAFE);
        assert_eq!(
            run.reports[0].file_report_malicious,
            Some(json!(["EngineA"]))
        );
        assert_eq!(
            run.reports[0].file_report_suspicious,
            Some(json!(["EngineB"]))
        );
    }

    struct Fixture {
        _temp: TempDir,
        local_root: Utf8PathBuf,
        global_root: Utf8PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should exist");
            let root = Utf8PathBuf::from_path_buf(PathBuf::from(temp.path()))
                .expect("temp path should be valid UTF-8");
            let local_root = root.join("local");
            let global_root = root.join("global");
            fs::create_dir_all(local_root.join("buckets/main/bucket"))
                .expect("bucket root should exist");
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

        fn manifest(&self, app: &str, manifest: &Value) {
            let path = self
                .local_root
                .join("buckets")
                .join("main")
                .join("bucket")
                .join(format!("{app}.json"));
            fs::write(
                path,
                serde_json::to_vec_pretty(manifest).expect("manifest should serialize"),
            )
            .expect("manifest should write");
        }
    }

    #[test]
    fn unsupported_hash_falls_back_to_url_lookup() {
        let url = "https://example.invalid/demo.zip";
        let url_id = url_safe_base64(url.as_bytes());
        let file_hash = "c".repeat(64);
        let server = TestServer::spawn(vec![
            Route::json(
                "GET",
                format!("/api/v3/urls/{url_id}"),
                200,
                json!({
                    "data": {
                        "id": "url-report-id",
                        "attributes": {
                            "last_http_response_content_sha256": file_hash,
                            "last_analysis_date": 1710000000
                        }
                    }
                }),
            ),
            Route::json(
                "GET",
                format!("/api/v3/files/{file_hash}"),
                200,
                json!({
                    "data": {
                        "attributes": {
                            "sha256": file_hash,
                            "size": 4,
                            "last_analysis_stats": {
                                "malicious": 0,
                                "suspicious": 0,
                                "undetected": 60,
                                "timeout": 0,
                                "harmless": 0,
                                "failure": 0
                            },
                            "last_analysis_results": {}
                        }
                    }
                }),
            ),
        ]);
        let fixture = Fixture::new();
        fixture.manifest(
            "demo",
            &json!({
                "version": "1.0.0",
                "url": url,
                "hash": "blake3:1234"
            }),
        );

        let run = check_apps(
            &fixture.config(),
            &[String::from("demo")],
            "demo-api-key",
            &VirusTotalOptions {
                base_url: Some(server.api_base_url()),
                ..VirusTotalOptions::default()
            },
        )
        .expect("check should succeed");

        assert_eq!(run.exit_code, 0);
        assert_eq!(server.requests().len(), 2);
        assert_eq!(server.requests()[0].path, format!("/api/v3/urls/{url_id}"));
        assert_eq!(
            server.requests()[1].path,
            format!("/api/v3/files/{file_hash}")
        );
        assert!(
            run.lines
                .iter()
                .any(|line| line.contains("unsupported hash blake3"))
        );
        assert_eq!(
            run.reports[0].url_report_url.as_deref(),
            Some("https://www.virustotal.com/gui/url/url-report-id")
        );
    }

    #[test]
    fn url_lookup_not_found_with_scan_submits_url() {
        let url = "https://example.invalid/demo.zip";
        let url_id = url_safe_base64(url.as_bytes());
        let server = TestServer::spawn(vec![
            Route::empty("GET", format!("/api/v3/urls/{url_id}"), 404),
            Route::json(
                "POST",
                "/api/v3/urls",
                200,
                json!({
                    "data": {
                        "id": "analysis-demo-analysis"
                    }
                }),
            ),
        ]);
        let fixture = Fixture::new();
        fixture.manifest(
            "demo",
            &json!({
                "version": "1.0.0",
                "url": url
            }),
        );

        let run = check_apps(
            &fixture.config(),
            &[String::from("demo")],
            "demo-api-key",
            &VirusTotalOptions {
                scan: true,
                base_url: Some(server.api_base_url()),
            },
        )
        .expect("check should succeed");

        assert_eq!(run.exit_code, 0);
        assert_eq!(server.requests().len(), 2);
        assert_eq!(server.requests()[1].method, "POST");
        assert_eq!(
            server.requests()[1].body,
            format!("url={}", urlencoding(url))
        );
        assert_eq!(
            run.reports[0].url_report_url.as_deref(),
            Some("https://www.virustotal.com/gui/url/analysis")
        );
    }

    #[test]
    fn rate_limited_file_lookup_sets_exception_bit() {
        let hash = "d".repeat(64);
        let server = TestServer::spawn(vec![Route::empty(
            "GET",
            format!("/api/v3/files/{hash}"),
            429,
        )]);
        let fixture = Fixture::new();
        let url = format!("{}/demo.zip", server.base_url());
        fixture.manifest(
            "demo",
            &json!({
                "version": "1.0.0",
                "url": url,
                "hash": format!("sha256:{hash}")
            }),
        );

        let run = check_apps(
            &fixture.config(),
            &[String::from("demo")],
            "demo-api-key",
            &VirusTotalOptions {
                base_url: Some(server.api_base_url()),
                ..VirusTotalOptions::default()
            },
        )
        .expect("check should succeed");

        assert_eq!(run.exit_code, EXIT_EXCEPTION);
        assert!(
            run.lines
                .iter()
                .any(|line| line.contains("rate limit or quota exceeded"))
        );
    }

    #[test]
    fn missing_lookup_sets_no_info_exit_bit() {
        let url = "https://example.invalid/demo.zip";
        let url_id = url_safe_base64(url.as_bytes());
        let server = TestServer::spawn(vec![Route::empty(
            "GET",
            format!("/api/v3/urls/{url_id}"),
            404,
        )]);
        let fixture = Fixture::new();
        fixture.manifest(
            "demo",
            &json!({
                "version": "1.0.0",
                "url": url
            }),
        );

        let run = check_apps(
            &fixture.config(),
            &[String::from("demo")],
            "demo-api-key",
            &VirusTotalOptions {
                base_url: Some(server.api_base_url()),
                ..VirusTotalOptions::default()
            },
        )
        .expect("check should succeed");

        assert_eq!(run.exit_code, EXIT_NO_INFO);
        assert!(run.reports.is_empty());
    }

    #[derive(Clone)]
    struct Route {
        method: String,
        path: String,
        status: u16,
        body: String,
        content_type: String,
    }

    impl Route {
        fn json(
            method: impl Into<String>,
            path: impl Into<String>,
            status: u16,
            body: Value,
        ) -> Self {
            Self {
                method: method.into(),
                path: path.into(),
                status,
                body: serde_json::to_string(&body).expect("body should serialize"),
                content_type: String::from("application/json"),
            }
        }

        fn empty(method: impl Into<String>, path: impl Into<String>, status: u16) -> Self {
            Self {
                method: method.into(),
                path: path.into(),
                status,
                body: String::new(),
                content_type: String::from("text/plain"),
            }
        }
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    struct RequestRecord {
        method: String,
        path: String,
        headers: BTreeMap<String, String>,
        body: String,
    }

    struct TestServer {
        base_url: String,
        api_base_url: String,
        requests: Arc<Mutex<Vec<RequestRecord>>>,
        shutdown: Option<mpsc::Sender<()>>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn spawn(routes: Vec<Route>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
            listener
                .set_nonblocking(true)
                .expect("listener should be non-blocking");
            let address = listener
                .local_addr()
                .expect("listener should expose address");
            let base_url = format!("http://{}", address);
            let api_base_url = format!("{base_url}/api/v3");
            let requests = Arc::new(Mutex::new(Vec::new()));
            let routes = Arc::new(Mutex::new(VecDeque::from(routes)));
            let (shutdown_tx, shutdown_rx) = mpsc::channel();
            let handle_requests = Arc::clone(&requests);
            let handle_routes = Arc::clone(&routes);
            let handle = thread::spawn(move || {
                loop {
                    if shutdown_rx.try_recv().is_ok() {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_nonblocking(false)
                                .expect("accepted stream should be blocking");
                            if let Some(request) = read_request(&mut stream) {
                                handle_requests
                                    .lock()
                                    .expect("requests mutex")
                                    .push(request.clone());
                                let route = handle_routes
                                    .lock()
                                    .expect("routes mutex")
                                    .pop_front()
                                    .unwrap_or_else(|| Route::empty("GET", "/", 500));
                                let response = if route.method == request.method
                                    && route.path == request.path
                                {
                                    route
                                } else {
                                    Route::json(
                                        "GET",
                                        "/",
                                        500,
                                        json!({
                                            "expected": {
                                                "method": route.method,
                                                "path": route.path,
                                            },
                                            "actual": {
                                                "method": request.method,
                                                "path": request.path,
                                            }
                                        }),
                                    )
                                };
                                write_response(&mut stream, &response);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            });

            Self {
                base_url,
                api_base_url,
                requests,
                shutdown: Some(shutdown_tx),
                handle: Some(handle),
            }
        }

        fn base_url(&self) -> String {
            self.base_url.clone()
        }

        fn api_base_url(&self) -> String {
            self.api_base_url.clone()
        }

        fn requests(&self) -> Vec<RequestRecord> {
            self.requests.lock().expect("requests mutex").clone()
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn read_request(stream: &mut TcpStream) -> Option<RequestRecord> {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout should set");
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = stream.read(&mut chunk).ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if find_headers_end(&buffer).is_some() {
                break;
            }
        }

        let headers_end = find_headers_end(&buffer)?;
        let mut headers = BTreeMap::new();
        let header_text = String::from_utf8_lossy(&buffer[..headers_end]);
        let mut lines = header_text.split("\r\n");
        let request_line = lines.next()?.to_owned();
        let mut request_line_parts = request_line.split_whitespace();
        let method = request_line_parts.next()?.to_owned();
        let path = request_line_parts.next()?.to_owned();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
            }
        }

        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        let body_start = headers_end + 4;
        while buffer.len() < body_start + content_length {
            let read = stream.read(&mut chunk).ok()?;
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }
        let body = String::from_utf8_lossy(
            buffer
                .get(body_start..body_start + content_length)
                .unwrap_or_default(),
        )
        .into_owned();

        Some(RequestRecord {
            method,
            path,
            headers,
            body,
        })
    }

    fn find_headers_end(buffer: &[u8]) -> Option<usize> {
        buffer.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn write_response(stream: &mut TcpStream, route: &Route) {
        let status_text = match route.status {
            200 => "OK",
            204 => "No Content",
            404 => "Not Found",
            429 => "Too Many Requests",
            500 => "Internal Server Error",
            _ => "OK",
        };
        let response = format!(
            "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n{}",
            route.status,
            status_text,
            route.body.len(),
            route.content_type,
            route.body
        );
        stream
            .write_all(response.as_bytes())
            .expect("response should write");
        stream.flush().expect("response should flush");
        let _ = stream.shutdown(Shutdown::Write);
    }
}
