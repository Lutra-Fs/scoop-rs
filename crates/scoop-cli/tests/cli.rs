use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
};

use regex::Regex;
use scoop_core::normalize_for_text_comparison;
use serde_json::Value;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

const UPSTREAM_SCOOP: &str = "D:\\Applications\\Scoop\\apps\\scoop\\current\\bin\\scoop.ps1";

#[test]
fn help_overview_from_binary_contains_expected_contract() {
    let output = run_binary(&["help"]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.starts_with("Usage: scoop <command> [<args>]"));
    assert!(
        output
            .stdout
            .contains("Type 'scoop help <command>' to get more help for a specific command.")
    );
    assert!(
        output
            .stdout
            .contains("help        Show help for a command")
    );
    assert!(output.stdout.contains("install     Install apps"));
    assert!(output.stdout.contains("Global options:"));
    assert!(output.stdout.contains("--color <auto|always|never>"));
}

#[test]
fn help_for_command_from_binary_includes_global_options() {
    let output = run_binary(&["help", "help"]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        "Usage: scoop help <command>\r\n\r\n\r\nGlobal options:\r\n  --color <auto|always|never>  Control ANSI color output.\r\n"
    );
}

#[test]
fn unknown_command_from_binary_matches_expected_warning_contract() {
    let output = run_binary(&["missing"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        "WARN  scoop: 'missing' isn't a scoop command. See 'scoop help'.\r\n"
    );
}

#[test]
fn parity_unknown_command_matches_upstream_exactly() {
    let ours = run_binary(&["missing"]);
    let upstream = run_upstream(&["missing"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_help_missing_matches_upstream_exactly() {
    let ours = run_binary(&["help", "missing"]);
    let upstream = run_upstream(&["help", "missing"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn color_never_keeps_warning_output_plain() {
    let output = run_binary(&["--color", "never", "missing"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        "WARN  scoop: 'missing' isn't a scoop command. See 'scoop help'.\r\n"
    );
    assert_eq!(strip_ansi(&output.stdout), output.stdout);
}

#[test]
fn color_auto_respects_no_color() {
    let output = run_binary_with_env(&["missing"], &[("NO_COLOR", "1")]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        "WARN  scoop: 'missing' isn't a scoop command. See 'scoop help'.\r\n"
    );
    assert_eq!(strip_ansi(&output.stdout), output.stdout);
}

#[test]
fn color_always_applies_ansi_to_help_and_warning_output() {
    let help = run_binary(&["--color", "always", "help"]);
    assert_eq!(help.status.code(), Some(0));
    assert_eq!(help.stderr, "");
    assert!(help.stdout.contains("\u{1b}["));
    assert!(strip_ansi(&help.stdout).contains("Global options:"));
    assert!(strip_ansi(&help.stdout).contains("--color <auto|always|never>"));

    let warning = run_binary(&["--color", "always", "missing"]);
    assert_eq!(warning.status.code(), Some(1));
    assert_eq!(warning.stderr, "");
    assert!(warning.stdout.contains("\u{1b}["));
    assert_eq!(
        strip_ansi(&warning.stdout),
        "WARN  scoop: 'missing' isn't a scoop command. See 'scoop help'.\r\n"
    );
}

#[test]
fn color_always_overrides_no_color() {
    let output = run_binary_with_env(&["--color", "always", "missing"], &[("NO_COLOR", "1")]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("\u{1b}["));
    assert_eq!(
        strip_ansi(&output.stdout),
        "WARN  scoop: 'missing' isn't a scoop command. See 'scoop help'.\r\n"
    );
}

#[test]
fn bucket_known_reads_names_from_scoop_buckets_json() {
    let fixture = InstallFixture::new();
    fixture.scoop_buckets_json(
        r#"{
            "main":"https://github.com/ScoopInstaller/Main",
            "extras":"https://github.com/ScoopInstaller/Extras"
        }"#,
    );

    let output = run_binary_with_env(
        &["bucket", "known"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(output.stdout, "main\r\nextras\r\n");
}

#[test]
fn bucket_list_renders_local_bucket_table() {
    let fixture = InstallFixture::new();
    fixture.init_remote_git_checkout(
        "main-bucket",
        "buckets\\main",
        &[("bucket\\demo.json", r#"{"version":"1.0.0"}"#)],
    );

    let output = run_binary_with_env(
        &["bucket", "list"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("Name"));
    assert!(output.stdout.contains("Source"));
    assert!(output.stdout.contains("Updated"));
    assert!(output.stdout.contains("Manifests"));
    assert!(output.stdout.contains("main"));
    assert!(output.stdout.contains("1"));
}

#[test]
fn bucket_add_clones_remote_repository() {
    let fixture = InstallFixture::new();
    let remote = fixture
        .create_remote_git_repo("extras", &[("bucket\\demo.json", r#"{"version":"1.0.0"}"#)]);

    let output = run_binary_with_env(
        &["bucket", "add", "extras", &remote],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        "The extras bucket was added successfully.\r\n"
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\buckets\\extras\\bucket\\demo.json",
            fixture.local_root()
        ))
        .exists()
    );
}

#[test]
fn bucket_rm_removes_existing_bucket_directory() {
    let fixture = InstallFixture::new();
    fixture.init_remote_git_checkout(
        "extras-bucket",
        "buckets\\extras",
        &[("bucket\\demo.json", r#"{"version":"1.0.0"}"#)],
    );

    let output = run_binary_with_env(
        &["bucket", "rm", "extras"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        "The extras bucket was removed successfully.\r\n"
    );
    assert!(!std::path::Path::new(&format!("{}\\buckets\\extras", fixture.local_root())).exists());
}

#[test]
fn parity_bucket_add_usage_matches_upstream_exactly() {
    let ours = run_binary(&["bucket", "add"]);
    let upstream = run_upstream(&["bucket", "add"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_bucket_rm_usage_matches_upstream_exactly() {
    let ours = run_binary(&["bucket", "rm"]);
    let upstream = run_upstream(&["bucket", "rm"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn cleanup_removes_old_versions_and_outdated_cache() {
    let fixture = InstallFixture::new();
    fixture.install_metadata("local", "demo", "1.0.0", r#"{"bucket":"main"}"#);
    fixture.install_metadata("local", "demo", "2.0.0", r#"{"bucket":"main"}"#);
    fixture.version_manifest("local", "demo", "1.0.0", r#"{"version":"1.0.0"}"#);
    fixture.version_manifest("local", "demo", "2.0.0", r#"{"version":"2.0.0"}"#);
    fixture.installed_manifest("local", "demo", r#"{"version":"2.0.0"}"#);
    fixture.file("local", "cache\\demo#1.0.0#demo.zip", b"cache");
    fixture.file("local", "cache\\demo#2.0.0#demo.zip", b"cache");

    let output = run_binary_with_env(
        &["cleanup", "demo", "--cache"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(output.stdout, "Removing demo: 1.0.0\r\n");
    assert!(
        !std::path::Path::new(&format!("{}\\apps\\demo\\1.0.0", fixture.local_root())).exists()
    );
    assert!(std::path::Path::new(&format!("{}\\apps\\demo\\2.0.0", fixture.local_root())).exists());
    assert!(
        !std::path::Path::new(&format!(
            "{}\\cache\\demo#1.0.0#demo.zip",
            fixture.local_root()
        ))
        .exists()
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\cache\\demo#2.0.0#demo.zip",
            fixture.local_root()
        ))
        .exists()
    );
}

#[test]
fn cleanup_all_prints_summary() {
    let fixture = InstallFixture::new();
    fixture.install_metadata("local", "demo", "1.0.0", r#"{"bucket":"main"}"#);
    fixture.install_metadata("local", "demo", "2.0.0", r#"{"bucket":"main"}"#);
    fixture.version_manifest("local", "demo", "1.0.0", r#"{"version":"1.0.0"}"#);
    fixture.version_manifest("local", "demo", "2.0.0", r#"{"version":"2.0.0"}"#);
    fixture.installed_manifest("local", "demo", r#"{"version":"2.0.0"}"#);

    let output = run_binary_with_env(
        &["cleanup", "*"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("Removing demo: 1.0.0"));
    assert!(output.stdout.contains("Everything is shiny now!"));
}

#[test]
fn parity_cleanup_usage_matches_upstream_exactly() {
    let ours = run_binary(&["cleanup"]);
    let upstream = run_upstream(&["cleanup"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_cleanup_missing_app_matches_upstream_exactly() {
    let ours = run_binary(&["cleanup", "missing"]);
    let upstream = run_upstream(&["cleanup", "missing"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn depends_renders_dependency_table_in_install_order() {
    let fixture = InstallFixture::new();
    fixture.bucket_manifest("main", "dep", r#"{"version":"1.0.0"}"#);
    fixture.bucket_manifest("main", "demo", r#"{"version":"2.0.0","depends":"dep"}"#);

    let output = run_binary_with_env(
        &["depends", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("Source"));
    assert!(output.stdout.contains("Name"));
    let dep_pos = output
        .stdout
        .find("main   dep")
        .expect("dependency row should be rendered");
    let demo_pos = output
        .stdout
        .find("main   demo")
        .expect("app row should be rendered");
    assert!(dep_pos < demo_pos);
}

#[test]
fn parity_depends_usage_matches_upstream_exactly() {
    let ours = run_binary(&["depends"]);
    let upstream = run_upstream(&["depends"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_depends_missing_manifest_matches_upstream_exactly() {
    let ours = run_binary(&["depends", "missing"]);
    let upstream = run_upstream(&["depends", "missing"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_depends_fixture_matches_upstream_when_normalized() {
    let fixture = InstallFixture::new();
    fixture.bucket_manifest("main", "dep", r#"{"version":"1.0.0"}"#);
    fixture.bucket_manifest("main", "demo", r#"{"version":"2.0.0","depends":"dep"}"#);

    let ours = run_binary_with_env(
        &["depends", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let upstream = run_upstream_with_env(
        &["depends", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(
        normalize_for_text_comparison(&collapse_table_whitespace(&ours.stdout)).trim_end(),
        normalize_for_text_comparison(&collapse_table_whitespace(&strip_ansi(&upstream.stdout)))
            .trim_end()
    );
}

#[test]
fn download_caches_manifest_payload_and_reuses_cache_on_second_run() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("download-demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"1.2.3","url":"{}","hash":"{}"}}"#,
            escape_json_path(&archive),
            hash
        ),
    );

    let first = run_binary_with_env(
        &["download", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(first.status.code(), Some(0));
    assert_eq!(first.stderr, "");
    assert!(
        first
            .stdout
            .contains("INFO  Downloading 'demo' [64bit] from main bucket")
    );
    assert!(
        first
            .stdout
            .contains("'demo' (1.2.3) was downloaded successfully!")
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\cache\\{}",
            fixture.local_root(),
            scoop_core::infra::cache::canonical_cache_file_name("demo", "1.2.3", &archive)
                .expect("cache filename should render")
        ))
        .is_file()
    );

    let second = run_binary_with_env(
        &["download", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(second.status.code(), Some(0));
    assert!(
        second
            .stdout
            .contains("Loading download-demo.zip from cache.")
    );
    assert!(
        second
            .stdout
            .contains("Checking hash of download-demo.zip... OK.")
    );
}

#[test]
fn parity_download_usage_matches_upstream_exactly() {
    let ours = run_binary(&["download"]);
    let upstream = run_upstream(&["download"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_download_missing_manifest_matches_upstream_exactly() {
    let fixture = InstallFixture::new();
    fixture.write_user_config(r#"{"last_update":"9999-01-01T00:00:00Z"}"#);
    let config_home = fixture.config_home();
    let ours = run_binary_with_env(
        &["download", "missing"],
        &[("XDG_CONFIG_HOME", &config_home)],
    );
    let upstream = run_upstream_with_env(
        &["download", "missing"],
        &[("XDG_CONFIG_HOME", &config_home)],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn export_renders_apps_buckets_and_optional_config_as_json() {
    let fixture = InstallFixture::new();
    let config_home = fixture.config_home();
    fixture.write_user_config(
        r#"{"use_sqlite_cache":true,"last_update":"2026-01-01T00:00:00Z","alias":{"ls":"list"}}"#,
    );
    fixture.scoop_buckets_json(
        r#"{"main":"https://github.com/ScoopInstaller/Main","extras":"https://github.com/ScoopInstaller/Extras"}"#,
    );
    fixture.init_remote_git_checkout(
        "main-bucket",
        "buckets\\main",
        &[("bucket\\demo.json", r#"{"version":"1.0.0"}"#)],
    );
    fixture.init_remote_git_checkout(
        "extras-bucket",
        "buckets\\extras",
        &[("bucket\\other.json", r#"{"version":"2.0.0"}"#)],
    );
    fixture.install_metadata("local", "demo", "1.0.0", r#"{"bucket":"main","hold":true}"#);
    fixture.installed_manifest("local", "demo", r#"{"version":"1.0.0"}"#);

    let plain = run_binary_with_env(
        &["export"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );
    let with_config = run_binary_with_env(
        &["export", "--config"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );

    assert_eq!(plain.status.code(), Some(0));
    assert_eq!(plain.stderr, "");
    let exported = parse_json_output(&plain.stdout);
    assert_eq!(exported["apps"][0]["Name"], "demo");
    assert_eq!(exported["apps"][0]["Source"], "main");
    assert_eq!(exported["apps"][0]["Info"], "Held package");
    assert_eq!(exported["buckets"][0]["Name"], "main");
    assert_eq!(exported["buckets"][1]["Name"], "extras");
    assert!(exported.get("config").is_none());

    assert_eq!(with_config.status.code(), Some(0));
    assert_eq!(with_config.stderr, "");
    let exported_with_config = parse_json_output(&with_config.stdout);
    assert_eq!(exported_with_config["config"]["use_sqlite_cache"], true);
    assert!(exported_with_config["config"].get("last_update").is_none());
    assert!(exported_with_config["config"].get("alias").is_none());
}

#[test]
fn parity_export_fixture_matches_upstream_semantically() {
    let fixture = InstallFixture::new();
    let config_home = fixture.config_home();
    fixture.write_user_config(r#"{"use_lessmsi":true,"last_update":"2026-01-01T00:00:00Z"}"#);
    fixture.scoop_buckets_json(r#"{"main":"https://github.com/ScoopInstaller/Main"}"#);
    fixture.init_remote_git_checkout(
        "main-bucket",
        "buckets\\main",
        &[("bucket\\demo.json", r#"{"version":"1.0.0"}"#)],
    );
    fixture.install_metadata("local", "demo", "1.0.0", r#"{"bucket":"main"}"#);
    fixture.installed_manifest("local", "demo", r#"{"version":"1.0.0"}"#);

    let ours = run_binary_with_env(
        &["export", "-c"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );
    let upstream = run_upstream_with_env(
        &["export", "-c"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_eq!(ours.stderr, "");
    assert_eq!(upstream.stderr, "");
    let mut ours_json = parse_json_output(&ours.stdout);
    let mut upstream_json = parse_json_output(&upstream.stdout);
    for export in [&mut ours_json, &mut upstream_json] {
        if let Some(apps) = export.get_mut("apps").and_then(Value::as_array_mut) {
            for app in apps {
                if let Some(object) = app.as_object_mut() {
                    object.insert(
                        String::from("Updated"),
                        Value::String(String::from("<time>")),
                    );
                }
            }
        }
        if let Some(buckets) = export.get_mut("buckets").and_then(Value::as_array_mut) {
            for bucket in buckets {
                if let Some(object) = bucket.as_object_mut() {
                    object.insert(
                        String::from("Updated"),
                        Value::String(String::from("<time>")),
                    );
                }
            }
        }
    }
    assert_eq!(
        canonicalize_json(&ours_json),
        canonicalize_json(&upstream_json)
    );
}

#[test]
fn import_restores_config_buckets_apps_and_hold_from_scoopfile() {
    let fixture = InstallFixture::new();
    let config_home = fixture.config_home();
    let archive = fixture.write_zip("import-demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    let remote = fixture.create_remote_git_repo(
        "import-main",
        &[(
            "bucket\\demo.json",
            &format!(
                r#"{{"version":"1.2.3","url":"{}","hash":"{}","bin":"demo.exe"}}"#,
                escape_json_path(&archive),
                hash
            ),
        )],
    );
    let scoopfile_path = format!("{}\\import.json", fixture.payload_root());
    fs::write(
        &scoopfile_path,
        serde_json::json!({
            "config": { "use_lessmsi": true },
            "buckets": [{ "Name": "main", "Source": remote }],
            "apps": [{ "Name": "demo", "Version": "1.2.3", "Source": "main", "Info": "Held package" }]
        })
        .to_string(),
    )
    .expect("scoopfile should be written");

    let output = run_binary_with_env(
        &["import", &scoopfile_path],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(
        output
            .stdout
            .contains("'use_lessmsi' has been set to 'True'")
    );
    assert!(
        output
            .stdout
            .contains("Installing 'demo' (1.2.3) [64bit] from 'main' bucket")
    );
    assert!(output.stdout.contains("demo is now held"));
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\demo.exe",
            fixture.local_root()
        ))
        .exists()
    );
    let config = fs::read_to_string(format!("{config_home}\\scoop\\config.json"))
        .expect("config should be written");
    assert!(config.contains(r#""use_lessmsi": true"#));
    let install_info = parse_json_output(
        &fs::read_to_string(format!(
            "{}\\apps\\demo\\1.2.3\\install.json",
            fixture.local_root()
        ))
        .expect("install metadata should exist"),
    );
    assert_eq!(install_info["hold"], true);
}

#[test]
fn parity_import_invalid_local_json_matches_upstream_exactly() {
    let fixture = InstallFixture::new();
    let invalid_path = format!("{}\\bad-import.json", fixture.payload_root());
    fs::write(&invalid_path, "{bad").expect("invalid json should exist");

    let ours = run_binary_with_env(
        &["import", &invalid_path],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let upstream = run_upstream_with_env(
        &["import", &invalid_path],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_eq!(ours.stderr, upstream.stderr);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn shim_add_list_info_and_rm_round_trip() {
    let fixture = InstallFixture::new();
    fixture.file("local", "tools\\demo.exe", b"binary");
    let target = format!("{}\\tools\\demo.exe", fixture.local_root());

    let add = run_binary_with_env(
        &["shim", "add", "demo", &target],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    assert_eq!(add.status.code(), Some(0));
    assert_eq!(add.stderr, "");
    assert!(add.stdout.contains("Adding local shim demo..."));

    let list = run_binary_with_env(
        &["shim", "list"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    assert_eq!(list.status.code(), Some(0));
    assert_eq!(list.stderr, "");
    assert!(list.stdout.contains("Name"));
    assert!(list.stdout.contains("demo"));

    let info = run_binary_with_env(
        &["shim", "info", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    assert_eq!(info.status.code(), Some(0));
    assert_eq!(info.stderr, "");
    assert!(info.stdout.contains("Name         : demo"));
    assert!(info.stdout.contains("Source       : External"));

    let rm = run_binary_with_env(
        &["shim", "rm", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    assert_eq!(rm.status.code(), Some(0));
    assert_eq!(rm.stderr, "");
    assert!(!std::path::Path::new(&format!("{}\\shims\\demo.cmd", fixture.local_root())).exists());
}

#[test]
fn parity_shim_usage_matches_upstream_exactly() {
    let ours = run_binary(&["shim"]);
    let upstream = run_upstream(&["shim"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_shim_add_usage_matches_upstream_exactly() {
    let ours = run_binary(&["shim", "add"]);
    let upstream = run_upstream(&["shim", "add"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_virustotal_usage_matches_upstream_exactly() {
    let ours = run_binary(&["virustotal"]);
    let upstream = run_upstream(&["virustotal"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn virustotal_reports_missing_api_key_in_isolated_config() {
    let fixture = InstallFixture::new();
    let config_home = fixture.config_home();
    fixture.write_user_config(r#"{"last_update":"9999-01-01T00:00:00Z"}"#);
    fixture.file(
        "local",
        "config.json",
        b"{\"last_update\":\"9999-01-01T00:00:00Z\"}",
    );
    fixture.bucket_manifest(
        "main",
        "demo",
        r#"{"version":"1.0.0","url":"https://example.invalid/demo.zip","hash":"sha256:abcd"}"#,
    );

    let ours = run_binary_with_env(
        &["virustotal", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );

    assert_eq!(ours.status.code(), Some(16));
    assert_eq!(ours.stderr, "");
    assert!(ours.stdout.contains("VirusTotal API key is not configured"));
}

#[test]
fn reinstall_installs_app_even_when_not_previously_installed() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("reinstall-demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"1.2.3","url":"{}","hash":"{}","bin":"demo.exe"}}"#,
            escape_json_path(&archive),
            hash
        ),
    );

    let output = run_binary_with_env(
        &["reinstall", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("'demo' isn't installed."));
    assert!(
        output
            .stdout
            .contains("Installing 'demo' (1.2.3) [64bit] from 'main' bucket")
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\demo.exe",
            fixture.local_root()
        ))
        .exists()
    );
}

#[test]
fn parity_reinstall_usage_matches_upstream_exactly() {
    let ours = run_binary(&["reinstall"]);
    let upstream = run_upstream(&["reinstall"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn reinstall_missing_returns_documented_orchestration_output() {
    let fixture = InstallFixture::new();
    let config_home = fixture.config_home();
    fixture.write_user_config(r#"{"last_update":"9999-01-01T00:00:00Z"}"#);
    fixture.file(
        "local",
        "config.json",
        b"{\"last_update\":\"9999-01-01T00:00:00Z\"}",
    );
    let ours = run_binary_with_env(
        &["reinstall", "missing"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );

    assert_eq!(ours.status.code(), Some(0));
    assert_eq!(ours.stderr, "");
    assert_eq!(
        ours.stdout,
        "ERROR 'missing' isn't installed.\r\nCouldn't find manifest for 'missing'.\r\n"
    );
}

#[test]
fn config_sets_and_gets_values_in_isolated_config_home() {
    let fixture = InstallFixture::new();
    let config_home = fixture.config_home();

    let set_output = run_binary_with_env(
        &["config", "root_path", "D:/Custom/Scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );

    assert_eq!(set_output.status.code(), Some(0));
    assert_eq!(set_output.stderr, "");
    assert_eq!(
        set_output.stdout,
        "'root_path' has been set to 'D:/Custom/Scoop'\r\n"
    );

    let get_output = run_binary_with_env(
        &["config", "root_path"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );

    assert_eq!(get_output.status.code(), Some(0));
    assert_eq!(get_output.stderr, "");
    assert_eq!(get_output.stdout, "D:/Custom/Scoop\r\n");
    let config = fs::read_to_string(format!("{config_home}\\scoop\\config.json"))
        .expect("config should be written");
    assert!(config.contains(r#""root_path": "D:/Custom/Scoop""#));
}

#[test]
fn config_rm_removes_value_from_active_config() {
    let fixture = InstallFixture::new();
    let config_home = fixture.config_home();
    fixture.write_user_config(r#"{"root_path":"D:/Custom/Scoop"}"#);

    let output = run_binary_with_env(
        &["config", "rm", "root_path"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(output.stdout, "'root_path' has been removed\r\n");
    let config = fs::read_to_string(format!("{config_home}\\scoop\\config.json"))
        .expect("config should still exist");
    assert!(!config.contains("root_path"));
}

#[test]
fn parity_config_missing_value_matches_upstream_with_isolated_config() {
    let fixture = InstallFixture::new();
    let config_home = fixture.config_home();
    let ours = run_binary_with_env(
        &["config", "use_sqlite_cache"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );
    let upstream = run_upstream_with_env(
        &["config", "use_sqlite_cache"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_config_set_root_path_matches_upstream_with_isolated_config() {
    let fixture = InstallFixture::new();
    let config_home = fixture.config_home();
    let ours = run_binary_with_env(
        &["config", "root_path", "D:/Custom/Scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );
    let upstream = run_upstream_with_env(
        &["config", "root_path", "D:/Custom/Scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &config_home),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn cache_show_lists_matching_entries_and_total() {
    let fixture = InstallFixture::new();
    fixture.file("local", "cache\\demo#1.2.3#demo.zip", b"demo");
    fixture.file("local", "cache\\other#2.0.0#other.zip", b"other");

    let output = run_binary_with_env(
        &["cache", "show", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("Name Version Length"));
    assert!(output.stdout.contains("demo 1.2.3"));
    assert!(output.stdout.contains("Total: 1 file, 4 B"));
}

#[test]
fn cache_rm_deletes_selected_entries() {
    let fixture = InstallFixture::new();
    fixture.file("local", "cache\\demo#1.2.3#demo.zip", b"demo");
    fixture.file("local", "cache\\demo.txt", b"note");
    fixture.file("local", "cache\\other#2.0.0#other.zip", b"other");

    let output = run_binary_with_env(
        &["cache", "rm", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("Removing demo#1.2.3#demo.zip..."));
    assert!(output.stdout.contains("Deleted: 1 file, 4 B"));
    assert!(
        !std::path::Path::new(&format!(
            "{}\\cache\\demo#1.2.3#demo.zip",
            fixture.local_root()
        ))
        .exists()
    );
    assert!(!std::path::Path::new(&format!("{}\\cache\\demo.txt", fixture.local_root())).exists());
    assert!(
        std::path::Path::new(&format!(
            "{}\\cache\\other#2.0.0#other.zip",
            fixture.local_root()
        ))
        .exists()
    );
}

#[test]
fn hold_and_unhold_toggle_install_metadata_for_app() {
    let fixture = InstallFixture::new();
    fixture.install_metadata("local", "demo", "1.2.3", r#"{"bucket":"main"}"#);
    fixture.installed_manifest("local", "demo", r#"{"version":"1.2.3"}"#);

    let hold_output = run_binary_with_env(
        &["hold", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(hold_output.status.code(), Some(0));
    assert_eq!(hold_output.stderr, "");
    assert_eq!(
        hold_output.stdout,
        "demo is now held and can not be updated anymore.\r\n"
    );
    let install_info = fs::read_to_string(format!(
        "{}\\apps\\demo\\1.2.3\\install.json",
        fixture.local_root()
    ))
    .expect("install metadata should exist");
    assert!(install_info.contains(r#""hold": true"#));

    let unhold_output = run_binary_with_env(
        &["unhold", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(unhold_output.status.code(), Some(0));
    assert_eq!(unhold_output.stderr, "");
    assert_eq!(
        unhold_output.stdout,
        "demo is no longer held and can be updated again.\r\n"
    );
    let install_info = fs::read_to_string(format!(
        "{}\\apps\\demo\\1.2.3\\install.json",
        fixture.local_root()
    ))
    .expect("install metadata should exist");
    assert!(!install_info.contains("\"hold\""));
}

#[test]
fn hold_scoop_sets_hold_update_until_in_active_config() {
    let fixture = InstallFixture::new();
    fs::write(format!("{}\\config.json", fixture.local_root()), "{}")
        .expect("portable config should exist");

    let output = run_binary_with_env(
        &["hold", "scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(
        output
            .stdout
            .contains("scoop is now held and might not be updated until")
    );
    let config = fs::read_to_string(format!("{}\\config.json", fixture.local_root()))
        .expect("portable config should be written");
    assert!(config.contains("hold_update_until"));
}

#[test]
fn parity_hold_usage_matches_upstream_exactly() {
    let ours = run_binary(&["hold"]);
    let upstream = run_upstream(&["hold"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn list_from_fixture_root_renders_expected_row() {
    let fixture = ListFixture::new();
    fixture.local_app(
        "git",
        "2.53.0.2",
        r#"{"version":"2.53.0.2"}"#,
        r#"{"bucket":"main","architecture":"64bit"}"#,
    );

    let output = run_binary_with_env(
        &["list", "^git$"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(
        output
            .stdout
            .starts_with("Installed apps matching '^git$':\r\n\r\n")
    );
    assert!(
        output
            .stdout
            .contains("Name Version  Source Updated             Info\r\n")
    );
    let row =
        Regex::new(r"(?m)^git\s+2\.53\.0\.2\s+main\s+\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\s*$")
            .expect("regex should compile");
    assert!(
        row.is_match(&output.stdout),
        "unexpected output:\n{}",
        output.stdout
    );
}

#[test]
fn list_from_fixture_root_reports_no_matches_cleanly() {
    let fixture = ListFixture::new();
    fixture.local_app(
        "git",
        "2.53.0.2",
        r#"{"version":"2.53.0.2"}"#,
        r#"{"bucket":"main","architecture":"64bit"}"#,
    );

    let output = run_binary_with_env(
        &["list", "^missing$"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(output.stdout, "Installed apps matching '^missing$':\r\n");
}

#[test]
fn list_from_empty_fixture_root_warns_and_exits_non_zero() {
    let fixture = ListFixture::new();
    let output = run_binary_with_env(
        &["list"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stderr, "");
    assert_eq!(output.stdout, "WARN  There aren't any apps installed.\r\n");
}

#[test]
fn list_invalid_regex_returns_single_actionable_error() {
    let fixture = ListFixture::new();
    let output = run_binary_with_env(
        &["list", "["],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stderr, "");
    assert_eq!(output.stdout, "ERROR invalid list query regex: [\r\n");
}

#[test]
fn list_from_fixture_root_reports_deprecated_and_failed_apps() {
    let fixture = ListFixture::new();
    fixture.local_app(
        "demo",
        "1.0.0",
        r#"{"version":"1.0.0"}"#,
        r#"{"bucket":"main"}"#,
    );
    fs::create_dir_all(format!(
        "{}\\buckets\\main\\deprecated",
        fixture.local_root()
    ))
    .expect("deprecated bucket dir should exist");
    fs::write(
        format!(
            "{}\\buckets\\main\\deprecated\\demo.json",
            fixture.local_root()
        ),
        r#"{"version":"1.0.0"}"#,
    )
    .expect("deprecated manifest should exist");
    fs::create_dir_all(format!("{}\\apps\\failedapp", fixture.local_root()))
        .expect("failed app dir should exist");

    let output = run_binary_with_env(
        &["list"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("Deprecated package"));
    assert!(output.stdout.contains("failedapp"));
    assert!(output.stdout.contains("Install failed"));
}

#[test]
fn cat_from_fixture_root_renders_pretty_manifest_json() {
    let fixture = CatFixture::new();
    fixture.bucket_manifest(
        "main",
        "git",
        r#"{"version":"2.53.0.2","description":"Git","bin":["git.exe"]}"#,
    );

    let output = run_binary_with_env(
        &["cat", "git"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(
        parse_json_output(&output.stdout),
        serde_json::json!({
            "version": "2.53.0.2",
            "description": "Git",
            "bin": ["git.exe"]
        })
    );
    assert!(output.stdout.contains("\r\n    \"version\": \"2.53.0.2\","));
}

#[test]
fn cat_from_fixture_root_prefers_installed_manifest() {
    let fixture = CatFixture::new();
    fixture.bucket_manifest(
        "main",
        "git",
        r#"{"version":"2.53.0.2","description":"bucket"}"#,
    );
    fixture.installed_manifest(
        "local",
        "git",
        r#"{"version":"2.53.0.2","description":"installed"}"#,
    );

    let output = run_binary_with_env(
        &["cat", "git"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(
        parse_json_output(&output.stdout)["description"],
        "installed"
    );
}

#[test]
fn cat_error_contracts_match_fixture_expectations() {
    let fixture = CatFixture::new();

    let missing = run_binary_with_env(
        &["cat", "missing"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let usage = run_binary_with_env(
        &["cat"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(missing.status.code(), Some(0));
    assert_eq!(missing.stderr, "");
    assert_eq!(missing.stdout, "Couldn't find manifest for 'missing'.\r\n");
    assert_eq!(usage.status.code(), Some(0));
    assert_eq!(usage.stderr, "");
    assert_eq!(
        usage.stdout,
        "ERROR <app> missing\r\nUsage: scoop cat <app>\r\r\n"
    );
}

#[test]
fn cat_from_direct_manifest_file_path() {
    let fixture = CatFixture::new();
    let manifest_path = format!("{}\\manual\\manifests\\demo.json", fixture.local_root());
    fs::create_dir_all(format!("{}\\manual\\manifests", fixture.local_root()))
        .expect("manual manifest dir should exist");
    fs::write(
        &manifest_path,
        r#"{"version":"1.0.0","description":"Direct"}"#,
    )
    .expect("direct manifest file should exist");

    let output = run_binary_with_env(
        &["cat", &manifest_path],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    let manifest = parse_json_output(&output.stdout);
    assert_eq!(manifest["version"], "1.0.0");
    assert_eq!(manifest["description"], "Direct");
}

#[test]
fn cat_from_direct_manifest_url() {
    let manifest_url = format!(
        "{}/demo.json",
        spawn_github_tree_server(1, r#"{"version":"2.0.0","description":"Remote"}"#,).base_url
    );
    let output = run_binary(&["cat", &manifest_url]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    let manifest = parse_json_output(&output.stdout);
    assert_eq!(manifest["version"], "2.0.0");
    assert_eq!(manifest["description"], "Remote");
}

#[test]
fn cat_uses_configured_bat_style_when_available() {
    let fixture = CatFixture::new();
    fixture.bucket_manifest("main", "git", r#"{"version":"2.53.0.2"}"#);

    let config_home = TempDir::new().expect("config home should exist");
    let config_dir = config_home.path().join("scoop");
    fs::create_dir_all(&config_dir).expect("config dir should exist");
    fs::write(config_dir.join("config.json"), r#"{"cat_style":"numbers"}"#)
        .expect("config file should exist");

    let fake_bin = TempDir::new().expect("fake bin dir should exist");
    let args_log = fake_bin.path().join("bat-args.txt");
    let script = "@echo off\r\necho %* > \"%BAT_LOG%\"\r\npowershell -NoProfile -Command \"$input | ForEach-Object { $_ }\"\r\n";
    fs::write(fake_bin.path().join("bat.cmd"), script).expect("fake bat should exist");

    let path_env = format!(
        "{};{}",
        fake_bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = run_binary_with_env(
        &["cat", "git"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            (
                "XDG_CONFIG_HOME",
                config_home
                    .path()
                    .to_str()
                    .expect("config home path should be UTF-8"),
            ),
            (
                "BAT_LOG",
                args_log.to_str().expect("args log path should be UTF-8"),
            ),
            ("PATH", &path_env),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(parse_json_output(&output.stdout)["version"], "2.53.0.2");
    let args = fs::read_to_string(args_log).expect("bat args log should exist");
    assert!(args.contains("--style numbers --language json"));
}

#[test]
fn info_from_fixture_root_renders_expected_property_list() {
    let fixture = InfoFixture::new();
    fixture.bucket_manifest(
        "main",
        "demo",
        r#"{
            "version":"1.2.3",
            "description":"Demo app",
            "homepage":"https://example.invalid/demo/",
            "license":"MIT",
            "bin":["demo.exe"],
            "notes":["note line 1","note line 2"]
        }"#,
    );
    fixture.installed_manifest(
        "local",
        "demo",
        r#"{"version":"1.2.3","description":"Installed demo"}"#,
    );
    fixture.install_metadata(
        "local",
        "demo",
        "1.2.3",
        r#"{"bucket":"main","architecture":"64bit"}"#,
    );

    let output = run_binary_with_env(
        &["info", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    let fields = parse_property_list(&output.stdout);
    assert_eq!(fields.get("Name").map(String::as_str), Some("demo"));
    assert_eq!(
        fields.get("Description").map(String::as_str),
        Some("Demo app")
    );
    assert_eq!(fields.get("Version").map(String::as_str), Some("1.2.3"));
    assert_eq!(fields.get("Source").map(String::as_str), Some("main"));
    assert_eq!(
        fields.get("Website").map(String::as_str),
        Some("https://example.invalid/demo")
    );
    assert_eq!(fields.get("License").map(String::as_str), Some("MIT"));
    assert_eq!(fields.get("Installed").map(String::as_str), Some("1.2.3"));
    assert_eq!(fields.get("Binaries").map(String::as_str), Some("demo.exe"));
    assert_eq!(
        fields.get("Notes").map(String::as_str),
        Some("note line 1\nnote line 2")
    );
}

#[test]
fn info_from_direct_manifest_url() {
    let manifest_url = format!(
        "{}/demo.json",
        spawn_github_tree_server(
            1,
            r#"{"version":"2.1.0","description":"Direct remote app"}"#,
        )
        .base_url
    );
    let output = run_binary(&["info", &manifest_url]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    let fields = parse_property_list(&output.stdout);
    assert_eq!(fields.get("Name").map(String::as_str), Some("demo"));
    assert_eq!(fields.get("Version").map(String::as_str), Some("2.1.0"));
    assert_eq!(
        fields.get("Description").map(String::as_str),
        Some("Direct remote app")
    );
}

#[test]
fn info_from_direct_manifest_file_path() {
    let fixture = InfoFixture::new();
    let manifest_path = format!("{}\\manual\\manifests\\demo.json", fixture.local_root());
    fs::create_dir_all(format!("{}\\manual\\manifests", fixture.local_root()))
        .expect("manual manifest dir should exist");
    fs::write(
        &manifest_path,
        r#"{"version":"3.0.0","description":"Direct local app"}"#,
    )
    .expect("direct manifest file should exist");

    let output = run_binary_with_env(
        &["info", &manifest_path],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    let fields = parse_property_list(&output.stdout);
    assert_eq!(fields.get("Name").map(String::as_str), Some("demo"));
    assert_eq!(fields.get("Version").map(String::as_str), Some("3.0.0"));
    assert_eq!(
        fields.get("Description").map(String::as_str),
        Some("Direct local app")
    );
}

#[test]
fn info_verbose_from_fixture_root_renders_extended_fields() {
    let fixture = InfoFixture::new();
    fixture.bucket_manifest(
        "main",
        "demo",
        r#"{
            "version":"1.2.3",
            "description":"Demo app",
            "homepage":"https://example.invalid/demo/",
            "license":"MIT",
            "bin":["demo.exe",["bin\\helper.ps1","demo-helper"]],
            "shortcuts":[["demo.exe","Demo App"]],
            "env_set":{"DEMO_HOME":"$dir\\home"},
            "env_add_path":[".","bin"],
            "suggest":{"Editor":["vscode","vim"]},
            "notes":["dir=$dir","persist=$persist_dir"]
        }"#,
    );
    fixture.installed_manifest(
        "local",
        "demo",
        r#"{"version":"1.2.3","description":"Installed demo"}"#,
    );
    fixture.install_metadata(
        "local",
        "demo",
        "1.2.3",
        r#"{"bucket":"main","architecture":"64bit"}"#,
    );
    fixture.file("local", "apps\\demo\\1.2.3\\demo.exe", b"demo");
    fixture.file("local", "apps\\demo\\current\\marker.txt", b"marker");
    fixture.file("local", "persist\\demo\\persist.txt", b"persist");
    fixture.file("local", "cache\\demo#1.2.3#demo.zip", b"cache");

    let output = run_binary_with_env(
        &["info", "demo", "-v"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    let fields = parse_property_list(&output.stdout);
    assert!(fields.contains_key("Manifest"));
    assert!(fields.contains_key("Updated by"));
    assert!(fields.contains_key("Installed size"));
    assert_eq!(
        fields.get("Binaries").map(String::as_str),
        Some("demo.exe | demo-helper.ps1")
    );
    assert_eq!(
        fields.get("Shortcuts").map(String::as_str),
        Some("Demo App")
    );
    assert!(
        fields
            .get("Environment")
            .is_some_and(|value| value.contains("DEMO_HOME = "))
    );
    assert!(
        fields
            .get("Path Added")
            .is_some_and(|value| value.contains("\\apps\\demo\\current"))
    );
    assert_eq!(
        fields.get("Suggestions").map(String::as_str),
        Some("vscode | vim")
    );
}

#[test]
fn info_error_contracts_match_fixture_expectations() {
    let fixture = InfoFixture::new();
    let missing = run_binary_with_env(
        &["info", "missing"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let usage = run_binary_with_env(
        &["info"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(missing.stderr, "");
    assert_eq!(
        missing.stdout,
        "Could not find manifest for 'missing' in local buckets.\r\n"
    );
    assert_eq!(usage.status.code(), Some(1));
    assert_eq!(usage.stderr, "");
    assert_eq!(usage.stdout, "Usage: scoop info <app>\r\n");
}

#[test]
fn search_from_fixture_root_renders_local_bucket_results() {
    let fixture = SearchStatusFixture::new();
    fixture.bucket_manifest(
        "main",
        "demo",
        r#"{"version":"1.2.3","bin":["demo.exe",["bin\\helper.exe","demo-helper"]]}"#,
    );
    fixture.bucket_manifest(
        "extras",
        "other",
        r#"{"version":"2.0.0","bin":"other.exe"}"#,
    );

    let all = run_binary_with_env(
        &["search"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let helper = run_binary_with_env(
        &["search", "helper"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(all.status.code(), Some(0));
    assert_eq!(all.stderr, "");
    assert!(
        all.stdout
            .starts_with("Results from local buckets...\r\n\r\n")
    );
    assert!(all.stdout.contains("demo  1.2.3   main"));
    assert!(all.stdout.contains("other 2.0.0   extras"));

    assert_eq!(helper.status.code(), Some(0));
    assert_eq!(helper.stderr, "");
    assert!(helper.stdout.contains("demo 1.2.3   main   helper.exe"));
}

#[test]
fn search_no_match_and_invalid_regex_contracts_are_stable() {
    let fixture = SearchStatusFixture::new();
    let no_match = run_binary_with_env(
        &["search", "missing"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let invalid = run_binary_with_env(
        &["search", "["],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(no_match.status.code(), Some(1));
    assert_eq!(no_match.stderr, "");
    assert_eq!(no_match.stdout, "WARN  No matches found.\r\n");
    assert_eq!(invalid.status.code(), Some(0));
    assert_eq!(invalid.stderr, "");
    assert_eq!(
        invalid.stdout,
        "Invalid regular expression: invalid pattern '['.\r\n"
    );
}

#[test]
fn search_queries_other_known_buckets_when_local_buckets_miss() {
    let fixture = SearchStatusFixture::new();
    fixture.scoop_buckets_json(
        r#"{"remote":"https://github.com/test/remote.git","main":"https://github.com/ScoopInstaller/Main"}"#,
    );
    let server = spawn_github_tree_server(
        1,
        r#"{"tree":[{"path":"bucket/git-plus.json"},{"path":"bucket/grep.json"}]}"#,
    );

    let output = run_binary_with_env(
        &["search", "git"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_GITHUB_API_BASE", &server.base_url),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.starts_with(
        "Results from other known buckets...\r\n(add them using 'scoop bucket add <bucket name>')\r\n\r\n"
    ));
    assert!(output.stdout.contains("git-plus remote"));
    server.join();
}

#[test]
fn search_with_sqlite_cache_uses_partial_matching_and_literal_queries() {
    let fixture = SearchStatusFixture::new();
    fixture.bucket_manifest(
        "main",
        "demo",
        r#"{"version":"1.2.3","bin":["demo.exe",["bin\\helper.ps1","demo-helper"]],"shortcuts":[["demo.exe","Demo App"]]}"#,
    );

    let config_home = TempDir::new().expect("config home should exist");
    let config_dir = config_home.path().join("scoop");
    fs::create_dir_all(&config_dir).expect("config dir should exist");
    fs::write(
        config_dir.join("config.json"),
        r#"{"use_sqlite_cache":true}"#,
    )
    .expect("config file should exist");

    let helper = run_binary_with_env(
        &["search", "helper"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            (
                "XDG_CONFIG_HOME",
                config_home
                    .path()
                    .to_str()
                    .expect("config home path should be UTF-8"),
            ),
        ],
    );
    let literal = run_binary_with_env(
        &["search", "["],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            (
                "XDG_CONFIG_HOME",
                config_home
                    .path()
                    .to_str()
                    .expect("config home path should be UTF-8"),
            ),
        ],
    );

    assert_eq!(helper.status.code(), Some(0));
    assert_eq!(helper.stderr, "");
    assert!(
        helper
            .stdout
            .contains("demo 1.2.3   main   demo | demo-helper")
    );

    assert_eq!(literal.status.code(), Some(1));
    assert_eq!(literal.stderr, "");
    assert_eq!(literal.stdout, "WARN  No matches found.\r\n");
}

#[test]
fn parity_search_gitignore_with_sqlite_cache_matches_upstream_after_normalization() {
    let config_home = TempDir::new().expect("config home should exist");
    let config_dir = config_home.path().join("scoop");
    fs::create_dir_all(&config_dir).expect("config dir should exist");
    fs::write(
        config_dir.join("config.json"),
        r#"{"use_sqlite_cache":true}"#,
    )
    .expect("config file should exist");
    let config_home = config_home
        .path()
        .to_str()
        .expect("config home path should be UTF-8")
        .to_owned();

    let ours = run_binary_with_env(
        &["search", "gitignore"],
        &[("XDG_CONFIG_HOME", &config_home)],
    );
    let upstream = run_upstream_with_env(
        &["search", "gitignore"],
        &[("XDG_CONFIG_HOME", &config_home)],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(
        collapse_table_whitespace(&strip_ansi(&ours.stdout)).trim_end(),
        collapse_table_whitespace(&strip_ansi(&upstream.stdout)).trim_end()
    );
}

#[test]
fn install_from_fixture_bucket_extracts_payload_and_creates_cmd_shim() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip(
        "demo.zip",
        &[("demo.exe", b"demo binary"), ("README.txt", b"hello")],
    );
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe",
                "notes":["note line 1","note line 2"]
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );

    let output = run_binary_with_env(
        &["install", "demo", "--verbose", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(
        output
            .stdout
            .contains("Installing 'demo' (1.2.3) [64bit] from 'main' bucket")
    );
    assert!(output.stdout.contains("Creating shim for 'demo'."));
    assert!(
        output
            .stdout
            .contains("'demo' (1.2.3) was installed successfully!")
    );
    assert!(
        output
            .stdout
            .contains("Notes\r\n-----\r\nnote line 1\r\nnote line 2\r\n-----\r\n")
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\1.2.3\\demo.exe",
            fixture.local_root()
        ))
        .is_file()
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\demo.exe",
            fixture.local_root()
        ))
        .exists()
    );
    let shim = fs::read_to_string(format!("{}\\shims\\demo.cmd", fixture.local_root()))
        .expect("shim should exist");
    assert!(shim.contains("@rem "));
    assert!(shim.contains("demo.exe"));
}

#[test]
fn color_always_colorizes_install_success_output() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );

    let output = run_binary_with_env(
        &["--color", "always", "install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("\u{1b}["));
    let plain = strip_ansi(&output.stdout);
    assert!(plain.contains("Installing 'demo' (1.2.3) [64bit] from 'main' bucket"));
    assert!(plain.contains("'demo' (1.2.3) was installed successfully!"));
}

#[test]
fn install_hash_mismatch_returns_actionable_error() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"1.2.3","url":"{}","hash":"deadbeef","bin":"demo.exe"}}"#,
            escape_json_path(&archive)
        ),
    );

    let output = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("ERROR Hash check failed!"));
    assert!(output.stdout.contains("Expected:    deadbeef"));
}

#[test]
fn install_runs_installer_and_applies_side_effects() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip(
        "demo.zip",
        &[
            (
                "setup.ps1",
                br#"param([string]$Marker,[string]$Version)
Set-Content -Path $Marker -Value "installed-$Version""#,
            ),
            ("demo.exe", b"demo binary"),
            ("data\\state.txt", b"state"),
            ("config\\settings.json", b"{\"demo\":true}"),
            ("module\\Demo.psm1", b"function Get-Demo { 'demo' }"),
        ],
    );
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe",
                "installer":{{"file":"setup.ps1","args":["$dir\\installed.txt","$version"]}},
                "persist":["data",["config\\settings.json","settings.json"]],
                "shortcuts":[["demo.exe","Demo App","--dir $dir"]],
                "env_add_path":[".","module"],
                "env_set":{{"DEMO_HOME":"$dir\\home"}},
                "psmodule":{{"name":"Demo.Module"}},
                "notes":["dir=$dir","persist=$persist_dir"]
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    let env_store = format!("{}\\env-store.json", fixture.payload_root());
    let startmenu_root = format!("{}\\startmenu", fixture.payload_root());

    let output = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("SCOOP_RS_STARTMENU_ROOT", &startmenu_root),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\1.2.3\\installed.txt",
            fixture.local_root()
        ))
        .is_file()
    );
    assert!(
        !std::path::Path::new(&format!(
            "{}\\apps\\demo\\1.2.3\\setup.ps1",
            fixture.local_root()
        ))
        .exists()
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\persist\\demo\\data\\state.txt",
            fixture.local_root()
        ))
        .is_file()
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\persist\\demo\\settings.json",
            fixture.local_root()
        ))
        .is_file()
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\data",
            fixture.local_root()
        ))
        .exists()
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\config\\settings.json",
            fixture.local_root()
        ))
        .exists()
    );
    assert!(std::path::Path::new(&format!("{startmenu_root}\\Demo App.lnk")).is_file());
    assert!(
        std::path::Path::new(&format!("{}\\modules\\Demo.Module", fixture.local_root())).exists()
    );

    let env_json = fs::read_to_string(&env_store).expect("mock env store should exist");
    assert!(env_json.contains("DEMO_HOME"));
    assert!(env_json.contains("demo\\\\current\\\\home"));
    assert!(env_json.contains("PATH"));
    assert!(env_json.contains("demo\\\\current"));
    assert!(env_json.contains("demo\\\\current\\\\module"));
    assert!(env_json.contains("PSModulePath"));
    assert!(env_json.contains("local\\\\modules"));
    assert!(output.stdout.contains("dir="));
    assert!(output.stdout.contains("persist="));
}

#[test]
fn install_extract_dir_and_extract_to_materialize_selected_subtree() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip(
        "demo-extract.zip",
        &[
            ("package\\bin\\demo.exe", b"demo"),
            ("package\\README.txt", b"readme"),
        ],
    );
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "extract_dir":"package",
                "extract_to":"app",
                "bin":"app\\bin\\demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );

    let output = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\app\\bin\\demo.exe",
            fixture.local_root()
        ))
        .is_file()
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\app\\README.txt",
            fixture.local_root()
        ))
        .is_file()
    );
}

#[test]
fn install_nightly_skips_hash_check_and_uses_dated_version_dir() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("nightly.zip", &[("nightly.exe", b"demo binary")]);
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"nightly","url":"{}","hash":"deadbeef","bin":"nightly.exe"}}"#,
            escape_json_path(&archive)
        ),
    );

    let output = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    let app_dir = std::path::Path::new(fixture.local_root())
        .join("apps")
        .join("demo");
    let dated = fs::read_dir(&app_dir)
        .expect("app dir should exist")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .find(|name| name.starts_with("nightly-"))
        .expect("nightly install should create a dated version directory");
    assert!(
        output
            .stdout
            .contains(&format!("Installing 'demo' ({dated}) [64bit]"))
    );
}

#[test]
fn install_repairs_failed_previous_install_before_retry() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("repair.zip", &[("demo.exe", b"fixed binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"2.0.0","url":"{}","hash":"{}","bin":"demo.exe"}}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    let broken_dir = format!("{}\\apps\\demo\\1.0.0", fixture.local_root());
    fs::create_dir_all(&broken_dir).expect("broken dir should exist");
    fs::write(format!("{broken_dir}\\install.json"), "{}").expect("install info should exist");
    fs::write(format!("{broken_dir}\\stale.txt"), "stale").expect("stale marker should exist");

    let output = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(
        !std::path::Path::new(&broken_dir).exists(),
        "failed install tree should be purged before retry"
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\demo.exe",
            fixture.local_root()
        ))
        .is_file()
    );
}

#[test]
fn install_renders_manifest_suggestions() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("suggest.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe",
                "suggest":{{"Extras":["vim","nano"]}}
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );

    let output = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.contains("'demo' suggests installing 'nano'."));
    assert!(output.stdout.contains("'demo' suggests installing 'vim'."));
}

#[test]
fn install_filters_manifest_suggestions_against_already_installed_apps() {
    let fixture = InstallFixture::new();
    let demo_archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let demo_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&demo_archive))
        .expect("demo hash should compute");
    let vim_archive = fixture.write_zip("vim.zip", &[("vim.exe", b"vim binary")]);
    let vim_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&vim_archive))
        .expect("vim hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe",
                "suggest":{{"Extras":["vim","nano"]}}
            }}"#,
            escape_json_path(&demo_archive),
            demo_hash
        ),
    );
    fixture.bucket_manifest(
        "main",
        "vim",
        &format!(
            r#"{{
                "version":"9.1.0",
                "url":"{}",
                "hash":"{}",
                "bin":"vim.exe"
            }}"#,
            escape_json_path(&vim_archive),
            vim_hash
        ),
    );
    let env_store = format!("{}\\env-store.json", fixture.payload_root());

    let vim_install = run_binary_with_env(
        &["install", "vim", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );
    assert_eq!(vim_install.status.code(), Some(0));

    let output = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("'demo' suggests installing 'nano'."));
    assert!(!output.stdout.contains("'demo' suggests installing 'vim'."));
}

#[test]
fn install_expands_shim_arguments_with_install_context() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("shim-args.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":[["demo.exe","demo","--root $dir --persist $persist_dir"]]
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );

    let output = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    let shim = fs::read_to_string(format!("{}\\shims\\demo.cmd", fixture.local_root()))
        .expect("shim should exist");
    assert!(shim.contains("--root"));
    assert!(shim.contains("apps\\demo\\current"));
    assert!(shim.contains("persist\\demo"));
}

#[test]
fn install_resolves_dependencies_and_multiple_apps() {
    let fixture = InstallFixture::new();
    let dep_archive = fixture.write_zip("dep.zip", &[("dep.exe", b"dep binary")]);
    let dep_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&dep_archive))
        .expect("dep hash should compute");
    fixture.bucket_manifest(
        "main",
        "dep",
        &format!(
            r#"{{"version":"1.0.0","url":"{}","hash":"{}","bin":"dep.exe"}}"#,
            escape_json_path(&dep_archive),
            dep_hash
        ),
    );
    let demo_archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let demo_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&demo_archive))
        .expect("demo hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"2.0.0","url":"{}","hash":"{}","bin":"demo.exe","depends":"dep"}}"#,
            escape_json_path(&demo_archive),
            demo_hash
        ),
    );

    let output = run_binary_with_env(
        &["install", "demo", "dep", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    let dep_pos = output
        .stdout
        .find("Installing 'dep' (1.0.0) [64bit] from 'main' bucket")
        .expect("dependency install should be rendered");
    let demo_pos = output
        .stdout
        .find("Installing 'demo' (2.0.0) [64bit] from 'main' bucket")
        .expect("main install should be rendered");
    assert!(dep_pos < demo_pos);
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\dep\\current\\dep.exe",
            fixture.local_root()
        ))
        .exists()
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\demo.exe",
            fixture.local_root()
        ))
        .exists()
    );
}

#[test]
fn install_from_manifest_file_path_succeeds() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("path-demo.zip", &[("path-demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    let manifest_path = format!("{}\\demo.json", fixture.payload_root());
    fs::write(
        &manifest_path,
        format!(
            r#"{{"version":"3.0.0","url":"{}","hash":"{}","bin":"path-demo.exe"}}"#,
            escape_json_path(&archive),
            hash
        ),
    )
    .expect("manifest path should be written");

    let output = run_binary_with_env(
        &["install", &manifest_path, "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\path-demo.exe",
            fixture.local_root()
        ))
        .exists()
    );
}

#[test]
fn install_from_manifest_url_succeeds() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("url-demo.zip", &[("url-demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    let server = spawn_github_tree_server(
        1,
        &format!(
            r#"{{"version":"4.0.0","url":"{}","hash":"{}","bin":"url-demo.exe"}}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    let manifest_url = format!("{}/demo.json", server.base_url);

    let output = run_binary_with_env(
        &["install", &manifest_url, "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    server.join();

    assert_eq!(output.status.code(), Some(0));
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\url-demo.exe",
            fixture.local_root()
        ))
        .exists()
    );
}

#[test]
fn install_versioned_manifest_from_bucket_git_history_succeeds() {
    let fixture = InstallFixture::new();
    let old_archive = fixture.write_zip("demo-1.zip", &[("demo.exe", b"v1")]);
    let old_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&old_archive))
        .expect("old hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"1.0.0","url":"{}","hash":"{}","bin":"demo.exe"}}"#,
            escape_json_path(&old_archive),
            old_hash
        ),
    );
    fixture.commit_bucket_manifest("main", "seed v1");

    let new_archive = fixture.write_zip("demo-2.zip", &[("demo.exe", b"v2")]);
    let new_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&new_archive))
        .expect("new hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"2.0.0","url":"{}","hash":"{}","bin":"demo.exe"}}"#,
            escape_json_path(&new_archive),
            new_hash
        ),
    );
    fixture.commit_bucket_manifest("main", "update v2");

    let output = run_binary_with_env(
        &["install", "demo@1.0.0", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(
        output
            .stdout
            .contains("Installing 'demo' (1.0.0) [64bit] from 'main' bucket")
    );
    let installed = fs::read(format!(
        "{}\\apps\\demo\\current\\demo.exe",
        fixture.local_root()
    ))
    .expect("versioned install should materialize current exe");
    assert_eq!(installed, b"v1");
}

#[test]
fn install_direct_manifest_with_matching_version_suffix_succeeds() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("direct-demo.zip", &[("direct-demo.exe", b"direct binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    let manifest = format!("{}\\manual-demo.json", fixture.payload_root());
    fs::write(
        &manifest,
        format!(
            r#"{{
                "version":"4.0.0",
                "url":"{}",
                "hash":"{}",
                "bin":"direct-demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    )
    .expect("direct manifest should be written");

    let install_reference = format!("{manifest}@4.0.0");
    let args: Vec<&str> = vec!["install", install_reference.as_str(), "--no-update-scoop"];
    let output = run_binary_with_env(
        &args,
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.contains("Installing 'manual-demo' (4.0.0)"));
    assert_eq!(
        fs::read(format!(
            "{}\\apps\\manual-demo\\current\\direct-demo.exe",
            fixture.local_root()
        ))
        .expect("versioned install should materialize current exe"),
        b"direct binary"
    );
}

#[test]
fn install_direct_manifest_with_mismatched_version_suffix_fails_with_clear_error() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("direct-demo.zip", &[("direct-demo.exe", b"direct binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    let manifest = format!("{}\\manual-demo.json", fixture.payload_root());
    fs::write(
        &manifest,
        format!(
            r#"{{
                "version":"4.0.0",
                "url":"{}",
                "hash":"{}",
                "bin":"direct-demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    )
    .expect("direct manifest should be written");

    let install_reference = format!("{manifest}@4.0.1");
    let args: Vec<&str> = vec!["install", install_reference.as_str(), "--no-update-scoop"];
    let output = run_binary_with_env(
        &args,
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output
            .stdout
            .contains("Version mismatch for manifest 'manual-demo': requested 4.0.1, found 4.0.0.")
    );
}

#[test]
fn install_independent_skips_dependencies() {
    let fixture = InstallFixture::new();
    let dep_archive = fixture.write_zip("dep-skip.zip", &[("dep.exe", b"dep binary")]);
    let dep_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&dep_archive))
        .expect("dep hash should compute");
    fixture.bucket_manifest(
        "main",
        "dep",
        &format!(
            r#"{{"version":"1.0.0","url":"{}","hash":"{}","bin":"dep.exe"}}"#,
            escape_json_path(&dep_archive),
            dep_hash
        ),
    );
    let demo_archive = fixture.write_zip("demo-skip.zip", &[("demo.exe", b"demo binary")]);
    let demo_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&demo_archive))
        .expect("demo hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"2.0.0","url":"{}","hash":"{}","bin":"demo.exe","depends":"dep"}}"#,
            escape_json_path(&demo_archive),
            demo_hash
        ),
    );

    let output = run_binary_with_env(
        &["install", "demo", "--independent", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\demo.exe",
            fixture.local_root()
        ))
        .exists()
    );
    assert!(
        !std::path::Path::new(&format!("{}\\apps\\dep", fixture.local_root())).exists(),
        "dependency should not install in independent mode"
    );
}

#[test]
fn install_missing_dependency_fails_before_main_app() {
    let fixture = InstallFixture::new();
    let demo_archive = fixture.write_zip("demo-missing.zip", &[("demo.exe", b"demo binary")]);
    let demo_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&demo_archive))
        .expect("demo hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"2.0.0","url":"{}","hash":"{}","bin":"demo.exe","depends":"missingdep"}}"#,
            escape_json_path(&demo_archive),
            demo_hash
        ),
    );

    let output = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(
        output
            .stdout
            .contains("ERROR Couldn't find manifest for 'missingdep'.")
    );
    assert!(
        !std::path::Path::new(&format!("{}\\apps\\demo", fixture.local_root())).exists(),
        "main app should not install when a dependency is missing"
    );
}

#[test]
fn install_usage_and_missing_manifest_match_upstream_exactly() {
    let fixture = InstallFixture::new();
    fixture.write_user_config(r#"{"last_update":"9999-01-01T00:00:00Z"}"#);
    let config_home = fixture.config_home();
    let usage = run_binary_with_env(&["install"], &[("XDG_CONFIG_HOME", &config_home)]);
    let usage_upstream = run_upstream_with_env(&["install"], &[("XDG_CONFIG_HOME", &config_home)]);
    assert_eq!(usage.status.code(), usage_upstream.status.code());
    assert_upstream_stderr_is_environmental(&usage, &usage_upstream);
    assert_eq!(usage.stdout, usage_upstream.stdout);

    let missing = run_binary_with_env(
        &["install", "missing", "--no-update-scoop"],
        &[("XDG_CONFIG_HOME", &config_home)],
    );
    let missing_upstream = run_upstream_with_env(
        &["install", "missing", "--no-update-scoop"],
        &[("XDG_CONFIG_HOME", &config_home)],
    );
    assert_eq!(missing.status.code(), missing_upstream.status.code());
    assert_upstream_stderr_is_environmental(&missing, &missing_upstream);
    assert_eq!(missing.stdout, missing_upstream.stdout);
}

#[test]
fn parity_install_already_installed_matches_upstream_exactly() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("installed.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"1.2.3","url":"{}","hash":"{}","bin":"demo.exe"}}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    let version_dir = format!("{}\\apps\\demo\\1.2.3", fixture.local_root());
    let current_dir = format!("{}\\apps\\demo\\current", fixture.local_root());
    fs::create_dir_all(&version_dir).expect("version dir should exist");
    fs::create_dir_all(&current_dir).expect("current dir should exist");
    fs::write(
        format!("{version_dir}\\install.json"),
        r#"{"bucket":"main","architecture":"64bit"}"#,
    )
    .expect("install info should exist");
    fs::write(
        format!("{current_dir}\\manifest.json"),
        r#"{"version":"1.2.3"}"#,
    )
    .expect("manifest should exist");

    let ours = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let upstream = run_upstream_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn status_from_fixture_root_renders_expected_rows() {
    let fixture = SearchStatusFixture::new();
    fixture.bucket_manifest("main", "demo", r#"{"version":"1.2.3","depends":["other"]}"#);
    fixture.installed_manifest("local", "demo", r#"{"version":"1.2.2"}"#);
    fixture.install_metadata("local", "demo", "1.2.2", r#"{"bucket":"main","hold":true}"#);
    fixture.installed_manifest("local", "removedapp", r#"{"version":"1.0.0"}"#);
    fixture.install_metadata("local", "removedapp", "1.0.0", r#"{"bucket":"main"}"#);

    let output = run_binary_with_env(
        &["status", "-l"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("Name"));
    assert!(output.stdout.contains("demo       1.2.2"));
    assert!(output.stdout.contains("1.2.3"));
    assert!(output.stdout.contains("other"));
    assert!(output.stdout.contains("Held package"));
    assert!(output.stdout.contains("removedapp 1.0.0"));
    assert!(output.stdout.contains("Manifest removed"));
}

#[test]
fn status_reports_clean_state_when_everything_is_ok() {
    let fixture = SearchStatusFixture::new();
    fixture.bucket_manifest("main", "demo", r#"{"version":"1.2.3"}"#);
    fixture.installed_manifest("local", "demo", r#"{"version":"1.2.3"}"#);
    fixture.install_metadata("local", "demo", "1.2.3", r#"{"bucket":"main"}"#);

    let output = run_binary_with_env(
        &["status", "-l"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(output.stdout, "Everything is ok!\r\n");
}

#[test]
fn color_always_colorizes_status_success_output() {
    let fixture = SearchStatusFixture::new();
    fixture.bucket_manifest("main", "demo", r#"{"version":"1.2.3"}"#);
    fixture.installed_manifest("local", "demo", r#"{"version":"1.2.3"}"#);
    fixture.install_metadata("local", "demo", "1.2.3", r#"{"bucket":"main"}"#);

    let output = run_binary_with_env(
        &["--color", "always", "status", "-l"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("\u{1b}["));
    assert_eq!(strip_ansi(&output.stdout), "Everything is ok!\r\n");
}

#[test]
fn status_reports_network_failure_with_warning() {
    let fixture = SearchStatusFixture::new();
    let scoop_current = format!("{}\\apps\\scoop\\current", fixture.local_root());
    fs::create_dir_all(&scoop_current).expect("scoop current dir should exist");
    let scoop_current = std::path::Path::new(&scoop_current);
    run_git(scoop_current, &["init"]);
    run_git(scoop_current, &["config", "user.name", "Codex"]);
    run_git(
        scoop_current,
        &["config", "user.email", "codex@example.invalid"],
    );
    run_git(scoop_current, &["config", "commit.gpgsign", "false"]);
    fs::write(scoop_current.join("README.md"), "seed").expect("seed file should exist");
    run_git(scoop_current, &["add", "."]);
    run_git(scoop_current, &["commit", "-m", "seed", "--no-gpg-sign"]);

    let output = run_binary_with_env(
        &["status"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        "WARN  Could not check for Scoop updates due to network failures.\r\n"
    );
}

#[test]
fn parity_list_exact_git_query_matches_upstream_when_normalized() {
    let ours = run_binary(&["list", "^git$"]);
    let upstream = run_upstream(&["list", "^git$"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(
        normalize_for_text_comparison(&ours.stdout).trim_end(),
        normalize_for_text_comparison(&strip_ansi(&upstream.stdout)).trim_end()
    );
}

#[test]
fn parity_list_failed_and_deprecated_rows_match_upstream_when_normalized() {
    let fixture = ListFixture::new();
    fixture.local_app(
        "demo",
        "1.0.0",
        r#"{"version":"1.0.0"}"#,
        r#"{"bucket":"main"}"#,
    );
    fs::create_dir_all(format!("{}\\shims", fixture.local_root()))
        .expect("local shims dir should exist");
    fs::create_dir_all(format!("{}\\shims", fixture.global_root()))
        .expect("global shims dir should exist");
    fs::create_dir_all(format!(
        "{}\\buckets\\main\\deprecated",
        fixture.local_root()
    ))
    .expect("deprecated bucket dir should exist");
    fs::write(
        format!(
            "{}\\buckets\\main\\deprecated\\demo.json",
            fixture.local_root()
        ),
        r#"{"version":"1.0.0"}"#,
    )
    .expect("deprecated manifest should exist");
    fs::create_dir_all(format!("{}\\apps\\failedapp", fixture.local_root()))
        .expect("failed app dir should exist");

    let ours = run_binary_with_env(
        &["list"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let upstream = run_upstream_with_env(
        &["list"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(
        normalize_for_text_comparison(&collapse_table_whitespace(&ours.stdout)).trim_end(),
        normalize_for_text_comparison(&collapse_table_whitespace(&strip_ansi(&upstream.stdout)))
            .trim_end()
    );
}

#[test]
fn parity_info_fixture_matches_upstream_stable_fields() {
    let fixture = InfoFixture::new();
    fixture.bucket_manifest(
        "main",
        "demo",
        r#"{
            "version":"1.2.3",
            "description":"Demo app",
            "homepage":"https://example.invalid/demo/",
            "license":"MIT",
            "bin":["demo.exe"],
            "notes":["note line 1","note line 2"]
        }"#,
    );
    fixture.installed_manifest(
        "local",
        "demo",
        r#"{"version":"1.2.3","description":"Installed demo"}"#,
    );
    fixture.install_metadata(
        "local",
        "demo",
        "1.2.3",
        r#"{"bucket":"main","architecture":"64bit"}"#,
    );

    let ours = run_binary_with_env(
        &["info", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let upstream = run_upstream_with_env(
        &["info", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);

    let ours_fields = parse_property_list(&ours.stdout);
    let upstream_fields = parse_property_list(&strip_ansi(&upstream.stdout));
    for key in [
        "Name",
        "Description",
        "Version",
        "Source",
        "Website",
        "License",
        "Installed",
        "Binaries",
        "Notes",
    ] {
        assert_eq!(
            ours_fields.get(key),
            upstream_fields.get(key),
            "field mismatch for {key}"
        );
    }
}

#[test]
fn parity_info_verbose_fixture_matches_upstream_extended_fields() {
    let fixture = InfoFixture::new();
    fixture.bucket_manifest(
        "main",
        "demo",
        r#"{
            "version":"1.2.3",
            "description":"Demo app",
            "homepage":"https://example.invalid/demo/",
            "license":"MIT",
            "bin":["demo.exe",["bin\\helper.ps1","demo-helper"]],
            "shortcuts":[["demo.exe","Demo App"]],
            "env_set":{"DEMO_HOME":"$dir\\home"},
            "env_add_path":[".","bin"],
            "suggest":{"Editor":["vscode","vim"]},
            "notes":["dir=$dir","persist=$persist_dir"]
        }"#,
    );
    fixture.installed_manifest(
        "local",
        "demo",
        r#"{"version":"1.2.3","description":"Installed demo"}"#,
    );
    fixture.install_metadata(
        "local",
        "demo",
        "1.2.3",
        r#"{"bucket":"main","architecture":"64bit"}"#,
    );
    fixture.file("local", "apps\\demo\\1.2.3\\demo.exe", b"demo");
    fixture.file("local", "apps\\demo\\current\\marker.txt", b"marker");
    fixture.file("local", "persist\\demo\\persist.txt", b"persist");
    fixture.file("local", "cache\\demo#1.2.3#demo.zip", b"cache");

    let ours = run_binary_with_env(
        &["info", "demo", "-v"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let upstream = run_upstream_with_env(
        &["info", "demo", "-v"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);

    let ours_fields = parse_property_list(&ours.stdout);
    let upstream_fields = parse_property_list(&strip_ansi(&upstream.stdout));
    for key in [
        "Manifest",
        "Installed",
        "Binaries",
        "Shortcuts",
        "Environment",
        "Path Added",
        "Suggestions",
        "Notes",
    ] {
        assert_eq!(
            ours_fields.get(key),
            upstream_fields.get(key),
            "field mismatch for {key}"
        );
    }
}

#[test]
fn parity_search_fixture_matches_upstream_when_normalized() {
    let fixture = SearchStatusFixture::new();
    fixture.bucket_manifest(
        "main",
        "demo",
        r#"{"version":"1.2.3","bin":["demo.exe",["bin\\helper.exe","demo-helper"]]}"#,
    );
    fixture.bucket_manifest(
        "extras",
        "other",
        r#"{"version":"2.0.0","bin":"other.exe"}"#,
    );

    let ours = run_binary_with_env(
        &["search", "helper"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let upstream = run_upstream_with_env(
        &["search", "helper"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(
        normalize_for_text_comparison(&collapse_table_whitespace(&ours.stdout)).trim_end(),
        normalize_for_text_comparison(&collapse_table_whitespace(&strip_ansi(&upstream.stdout)))
            .trim_end()
    );
}

#[test]
fn parity_status_local_fixture_matches_upstream_when_normalized() {
    let fixture = SearchStatusFixture::new();
    fixture.bucket_manifest("main", "demo", r#"{"version":"1.2.3","depends":["other"]}"#);
    fixture.installed_manifest("local", "demo", r#"{"version":"1.2.2"}"#);
    fixture.install_metadata("local", "demo", "1.2.2", r#"{"bucket":"main","hold":true}"#);
    fixture.installed_manifest("local", "removedapp", r#"{"version":"1.0.0"}"#);
    fixture.install_metadata("local", "removedapp", "1.0.0", r#"{"bucket":"main"}"#);

    let ours = run_binary_with_env(
        &["status", "-l"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let upstream = run_upstream_with_env(
        &["status", "-l"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(
        normalize_for_text_comparison(&collapse_table_whitespace(&ours.stdout)).trim_end(),
        normalize_for_text_comparison(&collapse_table_whitespace(&strip_ansi(&upstream.stdout)))
            .trim_end()
    );
}

#[test]
fn parity_cat_missing_matches_upstream_stdout_exactly() {
    let ours = run_binary(&["cat", "missing"]);
    let upstream = run_upstream(&["cat", "missing"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_cat_usage_matches_upstream_stdout_exactly() {
    let ours = run_binary(&["cat"]);
    let upstream = run_upstream(&["cat"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_cat_git_matches_upstream_json_when_normalized() {
    let ours = run_binary(&["cat", "git"]);
    let upstream = run_upstream(&["cat", "git"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_eq!(
        parse_json_output(&ours.stdout),
        parse_json_output(&upstream.stdout)
    );
}

#[test]
fn parity_list_no_match_query_matches_upstream_exactly() {
    let ours = run_binary(&["list", "^definitely-not-installed$"]);
    let upstream = run_upstream(&["list", "^definitely-not-installed$"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn prefix_from_fixture_root_prefers_local_then_global() {
    let fixture = PrefixWhichFixture::new();
    fixture.current_dir("local", "git");
    fixture.current_dir("global", "nodejs");

    let local = run_binary_with_env(
        &["prefix", "git"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let global = run_binary_with_env(
        &["prefix", "nodejs"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(local.status.code(), Some(0));
    assert_eq!(
        local.stdout,
        format!("{}\\apps\\git\\current\r\n", fixture.local_root())
    );
    assert_eq!(global.status.code(), Some(0));
    assert_eq!(
        global.stdout,
        format!("{}\\apps\\nodejs\\current\r\n", fixture.global_root())
    );
}

#[test]
fn which_from_fixture_root_resolves_exe_and_cmd_shims() {
    let fixture = PrefixWhichFixture::new();
    fixture.file(
        "local",
        "apps\\fixturegit\\current\\cmd\\fixturegit.exe",
        &[],
    );
    fixture.file(
        "local",
        "apps\\fixturegitignore\\current\\fixturegitignore.ps1",
        b"Write-Output 'fixture'",
    );
    fixture.shim(
        "local",
        "fixturegit.shim",
        &format!(
            "path = \"{}\\apps\\fixturegit\\current\\cmd\\fixturegit.exe\"\n",
            fixture.local_root()
        ),
    );
    fixture.shim(
        "local",
        "fixturegitignore.cmd",
        &format!(
            "@rem {}\\apps\\fixturegitignore\\current\\fixturegitignore.ps1\r\n@echo off\r\n",
            fixture.local_root()
        ),
    );

    let git = run_binary_with_env(
        &["which", "fixturegit"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("PATH", &fixture.shim_path_env()),
        ],
    );
    let gitignore = run_binary_with_env(
        &["which", "fixturegitignore"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("PATH", &fixture.shim_path_env()),
        ],
    );

    assert_eq!(git.status.code(), Some(0));
    assert_eq!(
        git.stdout,
        format!(
            "{}\\apps\\fixturegit\\current\\cmd\\fixturegit.exe\r\n",
            fixture.local_root()
        )
    );
    assert_eq!(gitignore.status.code(), Some(0));
    assert_eq!(
        gitignore.stdout,
        format!(
            "{}\\apps\\fixturegitignore\\current\\fixturegitignore.ps1\r\n",
            fixture.local_root()
        )
    );
}

#[test]
fn prefix_and_which_error_contracts_match_fixture_expectations() {
    let fixture = PrefixWhichFixture::new();

    let prefix_missing = run_binary_with_env(
        &["prefix", "missing"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let prefix_usage = run_binary_with_env(
        &["prefix"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let which_missing = run_binary_with_env(
        &["which", "missing"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    let which_usage = run_binary_with_env(
        &["which"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );

    assert_eq!(prefix_missing.status.code(), Some(0));
    assert_eq!(
        prefix_missing.stdout,
        "Could not find app path for 'missing'.\r\n"
    );
    assert_eq!(prefix_usage.status.code(), Some(0));
    assert_eq!(prefix_usage.stdout, "Usage: scoop prefix <app>\r\n");
    assert_eq!(which_missing.status.code(), Some(0));
    assert_eq!(
        which_missing.stdout,
        "WARN  'missing' not found, not a scoop shim, or a broken shim.\r\n"
    );
    assert_eq!(which_usage.status.code(), Some(0));
    assert_eq!(
        which_usage.stdout,
        "ERROR <command> missing\r\nUsage: scoop which <command>\r\n"
    );
}

#[test]
fn parity_prefix_git_matches_upstream_exactly() {
    let ours = run_binary(&["prefix", "git"]);
    let upstream = run_upstream(&["prefix", "git"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_which_git_matches_upstream_exactly() {
    let ours = run_binary(&["which", "git"]);
    let upstream = run_upstream(&["which", "git"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_which_gitignore_matches_upstream_exactly() {
    let ours = run_binary(&["which", "gitignore"]);
    let upstream = run_upstream(&["which", "gitignore"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_prefix_missing_matches_upstream_exactly() {
    let ours = run_binary(&["prefix", "missing"]);
    let upstream = run_upstream(&["prefix", "missing"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

#[test]
fn parity_which_missing_matches_upstream_exactly() {
    let ours = run_binary(&["which", "missing"]);
    let upstream = run_upstream(&["which", "missing"]);

    assert_eq!(ours.status.code(), upstream.status.code());
    assert_upstream_stderr_is_environmental(&ours, &upstream);
    assert_eq!(ours.stdout, upstream.stdout);
}

struct ProcessOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_binary(args: &[&str]) -> ProcessOutput {
    let exe = env!("CARGO_BIN_EXE_scoop");
    run_process(exe, args)
}

fn run_binary_with_env(args: &[&str], envs: &[(&str, &str)]) -> ProcessOutput {
    let exe = env!("CARGO_BIN_EXE_scoop");
    run_process_with_env(exe, args, envs)
}

fn run_upstream(args: &[&str]) -> ProcessOutput {
    run_upstream_with_env(args, &[])
}

fn run_upstream_with_env(args: &[&str], envs: &[(&str, &str)]) -> ProcessOutput {
    let mut pwsh_args = vec![
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        UPSTREAM_SCOOP,
    ];
    pwsh_args.extend_from_slice(args);
    run_process_with_env("pwsh", &pwsh_args, envs)
}

fn run_process(program: &str, args: &[&str]) -> ProcessOutput {
    run_process_with_env(program, args, &[])
}

fn run_process_with_env(program: &str, args: &[&str], envs: &[(&str, &str)]) -> ProcessOutput {
    let mut command = Command::new(program);
    command.args(args);
    command.envs(envs.iter().copied());

    let has_config_home = envs
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("XDG_CONFIG_HOME"));
    if !has_config_home
        && let Some((_, scoop_root)) = envs
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("SCOOP"))
    {
        let config_home = format!("{scoop_root}\\.test-config");
        let config_path = format!("{config_home}\\scoop\\config.json");
        if !std::path::Path::new(&config_path).exists() {
            let parent = std::path::Path::new(&config_path)
                .parent()
                .expect("config path should have a parent");
            fs::create_dir_all(parent).expect("test config parent should exist");
            fs::write(&config_path, r#"{"last_update":"9999-01-01T00:00:00Z"}"#)
                .expect("test config should be written");
        }
        command.env("XDG_CONFIG_HOME", config_home);
    }

    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to run {program}: {error}"));

    ProcessOutput {
        status: output.status,
        stdout: String::from_utf8(output.stdout).expect("stdout must be valid UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr must be valid UTF-8"),
    }
}

struct TestServer {
    base_url: String,
    handle: thread::JoinHandle<()>,
}

impl TestServer {
    fn join(self) {
        self.handle
            .join()
            .expect("server thread should exit cleanly");
    }
}

fn spawn_github_tree_server(requests: usize, body: &str) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("server should bind");
    let address = listener.local_addr().expect("server address should exist");
    let body = body.to_owned();
    let handle = thread::spawn(move || {
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should be written");
        }
    });

    TestServer {
        base_url: format!("http://{}", address),
        handle,
    }
}

struct ListFixture {
    _temp: TempDir,
    local_root: String,
    global_root: String,
}

impl ListFixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp dir should be created");
        let base = temp.path().to_path_buf();
        let local_root = base.join("local");
        let global_root = base.join("global");
        fs::create_dir_all(local_root.join("apps")).expect("local apps dir should exist");
        fs::create_dir_all(global_root.join("apps")).expect("global apps dir should exist");

        Self {
            _temp: temp,
            local_root: local_root.to_string_lossy().into_owned(),
            global_root: global_root.to_string_lossy().into_owned(),
        }
    }

    fn local_root(&self) -> &str {
        &self.local_root
    }

    fn global_root(&self) -> &str {
        &self.global_root
    }

    fn local_app(&self, name: &str, version: &str, manifest_json: &str, install_json: &str) {
        self.app(
            self.local_root(),
            name,
            version,
            manifest_json,
            install_json,
        );
    }

    fn app(&self, root: &str, name: &str, version: &str, manifest_json: &str, install_json: &str) {
        let version_dir = format!("{root}\\apps\\{name}\\{version}");
        let current_dir = format!("{root}\\apps\\{name}\\current");
        fs::create_dir_all(&version_dir).expect("version dir should exist");
        fs::create_dir_all(&current_dir).expect("current dir should exist");
        fs::write(format!("{version_dir}\\install.json"), install_json)
            .expect("install json should exist");
        fs::write(format!("{current_dir}\\manifest.json"), manifest_json)
            .expect("manifest json should exist");
    }
}

fn strip_ansi(text: &str) -> String {
    let pattern = Regex::new(r"\x1b\[[0-9;]*m").expect("ansi regex should compile");
    pattern.replace_all(text, "").into_owned()
}

fn parse_json_output(text: &str) -> Value {
    serde_json::from_str(text.trim()).expect("output should contain valid JSON")
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let mut normalized = serde_json::Map::new();
            for (key, value) in entries {
                normalized.insert(key.clone(), canonicalize_json(value));
            }
            Value::Object(normalized)
        }
        other => other.clone(),
    }
}

fn parse_property_list(text: &str) -> std::collections::BTreeMap<String, String> {
    let mut fields = std::collections::BTreeMap::new();
    let mut current_key = None::<String>;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Some((label, value)) = line.split_once(" : ") {
            let key = label.trim().to_owned();
            fields.insert(key.clone(), value.to_owned());
            current_key = Some(key);
            continue;
        }
        if let Some(key) = &current_key
            && let Some(existing) = fields.get_mut(key)
        {
            if !existing.is_empty() {
                existing.push('\n');
            }
            existing.push_str(line.trim_start());
        }
    }
    fields
}

fn collapse_table_whitespace(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn escape_json_path(path: &str) -> String {
    serde_json::to_string(path)
        .expect("manifest path should be serializable")
        .trim_matches('"')
        .to_owned()
}

fn assert_upstream_stderr_is_environmental(ours: &ProcessOutput, upstream: &ProcessOutput) {
    assert_eq!(ours.stderr, "");
    if upstream.stderr.is_empty() {
        return;
    }

    let normalized = strip_ansi(&upstream.stderr);
    assert!(
        normalized.contains(
            "Access to the path 'C:\\Users\\lutra\\.config\\scoop\\config.json' is denied."
        ),
        "unexpected upstream stderr: {}",
        upstream.stderr
    );
}

struct PrefixWhichFixture {
    _temp: TempDir,
    local_root: String,
    global_root: String,
}

struct CatFixture {
    _temp: TempDir,
    local_root: String,
    global_root: String,
}

struct InfoFixture {
    _temp: TempDir,
    local_root: String,
    global_root: String,
}

struct SearchStatusFixture {
    _temp: TempDir,
    local_root: String,
    global_root: String,
}

struct InstallFixture {
    _temp: TempDir,
    local_root: String,
    global_root: String,
    payload_root: String,
}

impl CatFixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp dir should be created");
        let base = temp.path().to_path_buf();
        let local_root = base.join("local");
        let global_root = base.join("global");
        fs::create_dir_all(local_root.join("buckets")).expect("local buckets dir should exist");
        fs::create_dir_all(local_root.join("apps")).expect("local apps dir should exist");
        fs::create_dir_all(global_root.join("apps")).expect("global apps dir should exist");

        Self {
            _temp: temp,
            local_root: local_root.to_string_lossy().into_owned(),
            global_root: global_root.to_string_lossy().into_owned(),
        }
    }

    fn local_root(&self) -> &str {
        &self.local_root
    }

    fn global_root(&self) -> &str {
        &self.global_root
    }

    fn bucket_manifest(&self, bucket: &str, app: &str, manifest_json: &str) {
        let path = format!(
            "{}\\buckets\\{}\\bucket\\{}.json",
            self.local_root(),
            bucket,
            app
        );
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("bucket manifest should have a parent");
        fs::create_dir_all(parent).expect("bucket manifest parent should exist");
        fs::write(path, manifest_json).expect("bucket manifest should exist");
    }

    fn installed_manifest(&self, scope: &str, app: &str, manifest_json: &str) {
        let root = match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        };
        let path = format!("{root}\\apps\\{app}\\current\\manifest.json");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("installed manifest should have a parent");
        fs::create_dir_all(parent).expect("installed manifest parent should exist");
        fs::write(path, manifest_json).expect("installed manifest should exist");
    }
}

impl InfoFixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp dir should be created");
        let base = temp.path().to_path_buf();
        let local_root = base.join("local");
        let global_root = base.join("global");
        fs::create_dir_all(local_root.join("buckets")).expect("local buckets dir should exist");
        fs::create_dir_all(local_root.join("apps")).expect("local apps dir should exist");
        fs::create_dir_all(local_root.join("shims")).expect("local shims dir should exist");
        fs::create_dir_all(global_root.join("apps")).expect("global apps dir should exist");
        fs::create_dir_all(global_root.join("shims")).expect("global shims dir should exist");

        Self {
            _temp: temp,
            local_root: local_root.to_string_lossy().into_owned(),
            global_root: global_root.to_string_lossy().into_owned(),
        }
    }

    fn local_root(&self) -> &str {
        &self.local_root
    }

    fn global_root(&self) -> &str {
        &self.global_root
    }

    fn bucket_manifest(&self, bucket: &str, app: &str, manifest_json: &str) {
        let path = format!(
            "{}\\buckets\\{}\\bucket\\{}.json",
            self.local_root(),
            bucket,
            app
        );
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("bucket manifest should have a parent");
        fs::create_dir_all(parent).expect("bucket manifest parent should exist");
        fs::write(path, manifest_json).expect("bucket manifest should exist");
    }

    fn installed_manifest(&self, scope: &str, app: &str, manifest_json: &str) {
        let root = match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        };
        let path = format!("{root}\\apps\\{app}\\current\\manifest.json");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("installed manifest should have a parent");
        fs::create_dir_all(parent).expect("installed manifest parent should exist");
        fs::write(path, manifest_json).expect("installed manifest should exist");
    }

    fn install_metadata(&self, scope: &str, app: &str, version: &str, install_json: &str) {
        let root = match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        };
        let path = format!("{root}\\apps\\{app}\\{version}\\install.json");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("install metadata should have a parent");
        fs::create_dir_all(parent).expect("install metadata parent should exist");
        fs::write(path, install_json).expect("install metadata should exist");
    }

    fn file(&self, scope: &str, relative_path: &str, content: &[u8]) {
        let root = match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        };
        let path = format!("{root}\\{relative_path}");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("fixture path should have a parent");
        fs::create_dir_all(parent).expect("fixture parent should exist");
        fs::write(path, content).expect("fixture file should exist");
    }
}

impl SearchStatusFixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp dir should be created");
        let base = temp.path().to_path_buf();
        let local_root = base.join("local");
        let global_root = base.join("global");
        fs::create_dir_all(local_root.join("buckets")).expect("local buckets dir should exist");
        fs::create_dir_all(local_root.join("apps")).expect("local apps dir should exist");
        fs::create_dir_all(local_root.join("shims")).expect("local shims dir should exist");
        fs::create_dir_all(global_root.join("apps")).expect("global apps dir should exist");
        fs::create_dir_all(global_root.join("shims")).expect("global shims dir should exist");

        Self {
            _temp: temp,
            local_root: local_root.to_string_lossy().into_owned(),
            global_root: global_root.to_string_lossy().into_owned(),
        }
    }

    fn local_root(&self) -> &str {
        &self.local_root
    }

    fn global_root(&self) -> &str {
        &self.global_root
    }

    fn bucket_manifest(&self, bucket: &str, app: &str, manifest_json: &str) {
        let path = format!(
            "{}\\buckets\\{}\\bucket\\{}.json",
            self.local_root(),
            bucket,
            app
        );
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("bucket manifest should have a parent");
        fs::create_dir_all(parent).expect("bucket manifest parent should exist");
        fs::write(path, manifest_json).expect("bucket manifest should exist");
    }

    fn installed_manifest(&self, scope: &str, app: &str, manifest_json: &str) {
        let root = match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        };
        let path = format!("{root}\\apps\\{app}\\current\\manifest.json");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("installed manifest should have a parent");
        fs::create_dir_all(parent).expect("installed manifest parent should exist");
        fs::write(path, manifest_json).expect("installed manifest should exist");
    }

    fn install_metadata(&self, scope: &str, app: &str, version: &str, install_json: &str) {
        let root = match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        };
        let path = format!("{root}\\apps\\{app}\\{version}\\install.json");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("install metadata should have a parent");
        fs::create_dir_all(parent).expect("install metadata parent should exist");
        fs::write(path, install_json).expect("install metadata should exist");
    }

    fn scoop_buckets_json(&self, content: &str) {
        let path = format!("{}\\apps\\scoop\\current\\buckets.json", self.local_root());
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("buckets json should have a parent");
        fs::create_dir_all(parent).expect("buckets json parent should exist");
        fs::write(path, content).expect("buckets json should exist");
    }
}

impl InstallFixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp dir should be created");
        let base = temp.path().to_path_buf();
        let local_root = base.join("local");
        let global_root = base.join("global");
        let payload_root = base.join("payloads");
        fs::create_dir_all(local_root.join("buckets")).expect("local buckets dir should exist");
        fs::create_dir_all(local_root.join("apps")).expect("local apps dir should exist");
        fs::create_dir_all(local_root.join("shims")).expect("local shims dir should exist");
        fs::create_dir_all(global_root.join("apps")).expect("global apps dir should exist");
        fs::create_dir_all(global_root.join("shims")).expect("global shims dir should exist");
        fs::create_dir_all(&payload_root).expect("payload root should exist");

        Self {
            _temp: temp,
            local_root: local_root.to_string_lossy().into_owned(),
            global_root: global_root.to_string_lossy().into_owned(),
            payload_root: payload_root.to_string_lossy().into_owned(),
        }
    }

    fn local_root(&self) -> &str {
        &self.local_root
    }

    fn global_root(&self) -> &str {
        &self.global_root
    }

    fn payload_root(&self) -> &str {
        &self.payload_root
    }

    fn config_home(&self) -> String {
        format!("{}\\config-home", self.payload_root())
    }

    fn write_user_config(&self, content: &str) {
        let path = format!("{}\\scoop\\config.json", self.config_home());
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("config should have a parent");
        fs::create_dir_all(parent).expect("config parent should exist");
        fs::write(path, content).expect("config should be written");
    }

    fn scoop_buckets_json(&self, content: &str) {
        let path = format!("{}\\apps\\scoop\\current\\buckets.json", self.local_root());
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("buckets json should have a parent");
        fs::create_dir_all(parent).expect("buckets json parent should exist");
        fs::write(path, content).expect("buckets json should exist");
    }

    fn bucket_manifest(&self, bucket: &str, app: &str, manifest_json: &str) {
        let path = format!(
            "{}\\buckets\\{}\\bucket\\{}.json",
            self.local_root(),
            bucket,
            app
        );
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("bucket manifest should have a parent");
        fs::create_dir_all(parent).expect("bucket manifest parent should exist");
        fs::write(path, manifest_json).expect("bucket manifest should exist");
    }

    fn installed_manifest(&self, scope: &str, app: &str, manifest_json: &str) {
        let root = match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        };
        let path = format!("{root}\\apps\\{app}\\current\\manifest.json");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("installed manifest should have a parent");
        fs::create_dir_all(parent).expect("installed manifest parent should exist");
        fs::write(path, manifest_json).expect("installed manifest should exist");
    }

    fn version_manifest(&self, scope: &str, app: &str, version: &str, manifest_json: &str) {
        let root = match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        };
        let path = format!("{root}\\apps\\{app}\\{version}\\manifest.json");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("version manifest should have a parent");
        fs::create_dir_all(parent).expect("version manifest parent should exist");
        fs::write(path, manifest_json).expect("version manifest should exist");
    }

    fn install_metadata(&self, scope: &str, app: &str, version: &str, install_json: &str) {
        let root = match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        };
        let path = format!("{root}\\apps\\{app}\\{version}\\install.json");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("install metadata should have a parent");
        fs::create_dir_all(parent).expect("install metadata parent should exist");
        fs::write(path, install_json).expect("install metadata should exist");
    }

    fn file(&self, scope: &str, relative_path: &str, content: &[u8]) {
        let root = match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        };
        let path = format!("{root}\\{relative_path}");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("fixture path should have a parent");
        fs::create_dir_all(parent).expect("fixture parent should exist");
        fs::write(path, content).expect("fixture file should exist");
    }

    fn commit_bucket_manifest(&self, bucket: &str, message: &str) {
        let repo = format!("{}\\buckets\\{}", self.local_root(), bucket);
        if !std::path::Path::new(&format!("{repo}\\.git")).exists() {
            fs::create_dir_all(&repo).expect("bucket repo should exist");
            run_git(std::path::Path::new(self.local_root()), &["init", &repo]);
            run_git(
                std::path::Path::new(self.local_root()),
                &["-C", &repo, "config", "user.name", "Codex"],
            );
            run_git(
                std::path::Path::new(self.local_root()),
                &["-C", &repo, "config", "user.email", "codex@example.invalid"],
            );
            run_git(
                std::path::Path::new(self.local_root()),
                &["-C", &repo, "config", "commit.gpgsign", "false"],
            );
        }
        run_git(
            std::path::Path::new(self.local_root()),
            &["-C", &repo, "add", "."],
        );
        run_git(
            std::path::Path::new(self.local_root()),
            &["-C", &repo, "commit", "-m", message],
        );
    }

    fn init_remote_git_checkout(
        &self,
        repo_name: &str,
        target_relative: &str,
        seed_files: &[(&str, &str)],
    ) -> String {
        let repo_root = format!("{}\\git-fixtures", self.payload_root());
        let source = format!("{repo_root}\\{repo_name}-src");
        let remote = format!("{repo_root}\\{repo_name}-remote.git");
        let target = format!("{}\\{target_relative}", self.local_root());

        fs::create_dir_all(&repo_root).expect("git fixture root should exist");
        fs::create_dir_all(&source).expect("git source should exist");
        run_git(std::path::Path::new(&repo_root), &["init", &source]);
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "config", "user.name", "Codex"],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &[
                "-C",
                &source,
                "config",
                "user.email",
                "codex@example.invalid",
            ],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "config", "commit.gpgsign", "false"],
        );
        for (relative, content) in seed_files {
            let path = format!("{source}\\{relative}");
            let parent = std::path::Path::new(&path)
                .parent()
                .expect("seed file should have a parent");
            fs::create_dir_all(parent).expect("seed parent should exist");
            fs::write(path, content).expect("seed file should be written");
        }
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "add", "."],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "commit", "-m", "seed"],
        );

        run_git(
            std::path::Path::new(&repo_root),
            &["init", "--bare", &remote],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "remote", "add", "origin", &remote],
        );
        let branch = git_stdout(
            std::path::Path::new(&repo_root),
            &["-C", &source, "branch", "--show-current"],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "push", "-u", "origin", branch.trim()],
        );

        let target_parent = std::path::Path::new(&target)
            .parent()
            .expect("target checkout should have a parent");
        fs::create_dir_all(target_parent).expect("target checkout parent should exist");
        run_git(
            std::path::Path::new(&repo_root),
            &["clone", "-q", &remote, &target],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &target, "config", "user.name", "Codex"],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &[
                "-C",
                &target,
                "config",
                "user.email",
                "codex@example.invalid",
            ],
        );
        remote
    }

    fn create_remote_git_repo(&self, repo_name: &str, seed_files: &[(&str, &str)]) -> String {
        let repo_root = format!("{}\\git-fixtures", self.payload_root());
        let source = format!("{repo_root}\\{repo_name}-src");
        let remote = format!("{repo_root}\\{repo_name}-remote.git");

        fs::create_dir_all(&repo_root).expect("git fixture root should exist");
        fs::create_dir_all(&source).expect("git source should exist");
        run_git(std::path::Path::new(&repo_root), &["init", &source]);
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "config", "user.name", "Codex"],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &[
                "-C",
                &source,
                "config",
                "user.email",
                "codex@example.invalid",
            ],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "config", "commit.gpgsign", "false"],
        );
        for (relative, content) in seed_files {
            let path = format!("{source}\\{relative}");
            let parent = std::path::Path::new(&path)
                .parent()
                .expect("seed file should have a parent");
            fs::create_dir_all(parent).expect("seed parent should exist");
            fs::write(path, content).expect("seed file should be written");
        }
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "add", "."],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "commit", "-m", "seed"],
        );

        run_git(
            std::path::Path::new(&repo_root),
            &["init", "--bare", &remote],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "remote", "add", "origin", &remote],
        );
        let branch = git_stdout(
            std::path::Path::new(&repo_root),
            &["-C", &source, "branch", "--show-current"],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "push", "-u", "origin", branch.trim()],
        );
        remote
    }

    fn push_remote_git_update(
        &self,
        repo_name: &str,
        changed_files: &[(&str, &str)],
        message: &str,
    ) {
        let repo_root = format!("{}\\git-fixtures", self.payload_root());
        let source = format!("{repo_root}\\{repo_name}-src");
        for (relative, content) in changed_files {
            let path = format!("{source}\\{relative}");
            let parent = std::path::Path::new(&path)
                .parent()
                .expect("changed file should have a parent");
            fs::create_dir_all(parent).expect("changed file parent should exist");
            fs::write(path, content).expect("changed file should be written");
        }
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "add", "."],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "commit", "-m", message],
        );
        let branch = git_stdout(
            std::path::Path::new(&repo_root),
            &["-C", &source, "branch", "--show-current"],
        );
        run_git(
            std::path::Path::new(&repo_root),
            &["-C", &source, "push", "origin", branch.trim()],
        );
    }

    fn write_zip(&self, filename: &str, files: &[(&str, &[u8])]) -> String {
        let path = format!("{}\\{}", self.payload_root, filename);
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("zip should have a parent");
        fs::create_dir_all(parent).expect("zip parent should exist");
        let file = fs::File::create(&path).expect("zip should be created");
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

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run git {:?}: {error}", args));
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run git {:?}: {error}", args));
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn seed_installed_scoop(fixture: &InstallFixture, version: &str, bucket: &str, binary: &[u8]) {
    let version_dir = format!("{}\\apps\\scoop\\{version}", fixture.local_root());
    let current_dir = format!("{}\\apps\\scoop\\current", fixture.local_root());
    fs::create_dir_all(&version_dir).expect("scoop version dir should exist");
    fs::create_dir_all(&current_dir).expect("scoop current dir should exist");
    fs::write(
        format!("{version_dir}\\install.json"),
        format!(r#"{{"bucket":"{bucket}","architecture":"64bit"}}"#),
    )
    .expect("scoop install info should exist");
    fs::write(
        format!("{version_dir}\\manifest.json"),
        format!(r#"{{"version":"{version}"}}"#),
    )
    .expect("scoop version manifest should exist");
    fs::write(format!("{version_dir}\\scoop.exe"), binary).expect("scoop binary should exist");
    fs::write(
        format!("{current_dir}\\manifest.json"),
        format!(r#"{{"version":"{version}"}}"#),
    )
    .expect("scoop current manifest should exist");
}

impl PrefixWhichFixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp dir should be created");
        let base = temp.path().to_path_buf();
        let local_root = base.join("local");
        let global_root = base.join("global");
        fs::create_dir_all(local_root.join("apps")).expect("local apps dir should exist");
        fs::create_dir_all(local_root.join("shims")).expect("local shims dir should exist");
        fs::create_dir_all(global_root.join("apps")).expect("global apps dir should exist");
        fs::create_dir_all(global_root.join("shims")).expect("global shims dir should exist");

        Self {
            _temp: temp,
            local_root: local_root.to_string_lossy().into_owned(),
            global_root: global_root.to_string_lossy().into_owned(),
        }
    }

    fn local_root(&self) -> &str {
        &self.local_root
    }

    fn global_root(&self) -> &str {
        &self.global_root
    }

    fn current_dir(&self, scope: &str, app: &str) {
        let root = self.root(scope);
        fs::create_dir_all(format!("{root}\\apps\\{app}\\current"))
            .expect("current dir should exist");
    }

    fn shim(&self, scope: &str, name: &str, content: &str) {
        let root = self.root(scope);
        fs::write(format!("{root}\\shims\\{name}"), content).expect("shim should be written");
        if let Some(base_name) = name.strip_suffix(".shim") {
            fs::write(format!("{root}\\shims\\{base_name}.exe"), [])
                .expect("shim exe should exist");
        }
    }

    fn root(&self, scope: &str) -> &str {
        match scope {
            "local" => self.local_root(),
            "global" => self.global_root(),
            _ => panic!("unknown scope"),
        }
    }

    fn file(&self, scope: &str, relative_path: &str, content: &[u8]) {
        let root = self.root(scope);
        let path = format!("{root}\\{relative_path}");
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("fixture path should have a parent");
        fs::create_dir_all(parent).expect("fixture parent should exist");
        fs::write(path, content).expect("fixture file should exist");
    }

    fn shim_path_env(&self) -> String {
        format!("{}\\shims;{}\\shims", self.local_root(), self.global_root())
    }
}

// ===========================================================================
// Uninstall CLI tests
// ===========================================================================

#[test]
fn uninstall_usage_error_when_no_app_given() {
    let output = run_binary(&["uninstall"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.contains("ERROR <app> missing"));
}

#[test]
fn uninstall_reports_not_installed_for_missing_app() {
    let fixture = InstallFixture::new();
    let output = run_binary_with_env(
        &["uninstall", "nonexistent"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.contains("isn't installed"));
}

#[test]
fn uninstall_removes_installed_app_shims_and_current() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    let env_store = format!("{}\\env-store.json", fixture.payload_root());

    // Install first
    let install_out = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );
    assert_eq!(install_out.status.code(), Some(0));
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\demo.exe",
            fixture.local_root()
        ))
        .exists(),
        "demo.exe should exist before uninstall"
    );

    // Uninstall
    let output = run_binary_with_env(
        &["uninstall", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("'demo' (1.2.3) was uninstalled."));

    // Verify cleanup
    assert!(
        !std::path::Path::new(&format!("{}\\apps\\demo\\current", fixture.local_root())).exists(),
        "current dir should be removed after uninstall"
    );
    assert!(
        !std::path::Path::new(&format!("{}\\shims\\demo.cmd", fixture.local_root())).exists(),
        "shim should be removed after uninstall"
    );
}

#[test]
fn uninstall_purge_removes_persist_directory() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip(
        "demo.zip",
        &[("demo.exe", b"demo binary"), ("data/state.txt", b"saved")],
    );
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe",
                "persist":"data"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    let env_store = format!("{}\\env-store.json", fixture.payload_root());

    // Install
    let _ = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );
    let persist_dir = format!("{}\\persist\\demo", fixture.local_root());
    assert!(
        std::path::Path::new(&persist_dir).exists(),
        "persist dir should exist after install"
    );

    // Uninstall with --purge
    let output = run_binary_with_env(
        &["uninstall", "demo", "--purge"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.contains("was uninstalled"));
    assert!(
        !std::path::Path::new(&persist_dir).exists(),
        "persist dir should be removed with --purge"
    );
}

#[test]
fn uninstall_skips_app_with_running_process() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    let env_store = format!("{}\\env-store.json", fixture.payload_root());

    let install_out = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );
    assert_eq!(install_out.status.code(), Some(0));

    let running_path = format!("{}\\apps\\demo\\current\\demo.exe", fixture.local_root());
    let output = run_binary_with_env(
        &["uninstall", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("SCOOP_RS_RUNNING_PROCESS_PATHS", &running_path),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(
        output
            .stdout
            .contains("are still running. Close them and try again.")
    );
    assert!(output.stdout.contains(&running_path));
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\demo.exe",
            fixture.local_root()
        ))
        .exists(),
        "app should remain installed when a running process is detected"
    );
}

#[test]
fn uninstall_quiet_suppresses_summary() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );

    // Install first
    let install_out = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    assert_eq!(install_out.status.code(), Some(0));

    // Uninstall with quiet output
    let output = run_binary_with_env(
        &["uninstall", "-q", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(!output.stdout.contains("'demo' (1.2.3) was uninstalled."));
}

#[test]
fn uninstall_running_process_keeps_error_with_quiet() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );

    let install_out = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
        ],
    );
    assert_eq!(install_out.status.code(), Some(0));

    let running_path = format!("{}\\apps\\demo\\current\\demo.exe", fixture.local_root());
    let output = run_binary_with_env(
        &["uninstall", "-q", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_RUNNING_PROCESS_PATHS", &running_path),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(
        output
            .stdout
            .contains("are still running. Close them and try again.")
    );
    assert!(output.stdout.contains(&running_path));
}

// ===========================================================================
// Reset CLI tests
// ===========================================================================

#[test]
fn reset_usage_error_when_no_app_given() {
    let output = run_binary(&["reset"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.contains("ERROR <app> missing"));
}

#[test]
fn reset_rebuilds_shims_for_installed_app() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    let env_store = format!("{}\\env-store.json", fixture.payload_root());

    // Install
    let _ = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );

    // Delete shim to simulate breakage
    let shim_path = format!("{}\\shims\\demo.cmd", fixture.local_root());
    assert!(
        std::path::Path::new(&shim_path).is_file(),
        "shim should exist before deletion"
    );
    fs::remove_file(&shim_path).expect("shim should be deletable");
    assert!(!std::path::Path::new(&shim_path).is_file());

    // Reset
    let output = run_binary_with_env(
        &["reset", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("'demo' (1.2.3) was reset."));

    // Shim should be restored
    assert!(
        std::path::Path::new(&shim_path).is_file(),
        "shim should be restored after reset"
    );
}

#[test]
fn reset_quiet_suppresses_summary() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    let env_store = format!("{}\\env-store.json", fixture.payload_root());

    // Install
    let _ = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );

    // Delete shim to simulate breakage
    let shim_path = format!("{}\\shims\\demo.cmd", fixture.local_root());
    assert!(
        std::path::Path::new(&shim_path).is_file(),
        "shim should exist before deletion"
    );
    fs::remove_file(&shim_path).expect("shim should be deletable");
    assert!(!std::path::Path::new(&shim_path).is_file());

    let output = run_binary_with_env(
        &["reset", "-q", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(!output.stdout.contains("'demo' (1.2.3) was reset."));

    // Shim should be restored
    assert!(
        std::path::Path::new(&shim_path).is_file(),
        "shim should be restored after reset"
    );
}

// ===========================================================================
// Update CLI tests
// ===========================================================================

#[test]
fn update_no_args_syncs_buckets_and_self_updates_versioned_scoop_binary() {
    let fixture = InstallFixture::new();
    let old_archive = fixture.write_zip("scoop-2.0.0.zip", &[("scoop.exe", b"new binary")]);
    let old_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&old_archive))
        .expect("hash should compute");
    fixture.init_remote_git_checkout(
        "main-bucket",
        "buckets\\main",
        &[("bucket\\scoop.json", r#"{"version":"1.0.0"}"#)],
    );
    fixture.push_remote_git_update(
        "main-bucket",
        &[(
            "bucket\\scoop.json",
            &format!(
                r#"{{"version":"2.0.0","url":"{}","hash":"{}","bin":"scoop.exe"}}"#,
                escape_json_path(&old_archive),
                old_hash
            ),
        )],
        "bucket update",
    );
    seed_installed_scoop(&fixture, "1.0.0", "main", b"old binary");

    let output = run_binary_with_env(
        &["update"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.contains("Scoop was updated successfully!"));
    assert_eq!(
        fs::read(format!(
            "{}\\apps\\scoop\\current\\scoop.exe",
            fixture.local_root()
        ))
        .expect("updated scoop binary should exist"),
        b"new binary"
    );
    assert!(
        fs::read_to_string(format!(
            "{}\\buckets\\main\\bucket\\scoop.json",
            fixture.local_root()
        ))
        .expect("updated bucket manifest should exist")
        .contains(r#""version":"2.0.0""#)
    );
    let config = fs::read_to_string(format!("{}\\scoop\\config.json", fixture.config_home()))
        .expect("last_update config should be written");
    assert!(config.contains("\"last_update\""));
}

#[test]
fn update_no_args_skips_scoop_update_when_on_hold() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("scoop-1.0.0.zip", &[("scoop.exe", b"old binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "scoop",
        &format!(
            r#"{{"version":"2.0.0","url":"{}","hash":"{}","bin":"scoop.exe"}}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    seed_installed_scoop(&fixture, "1.0.0", "main", b"old binary");
    fs::write(
        format!("{}\\apps\\scoop\\current\\scoop.exe", fixture.local_root()),
        b"old binary",
    )
    .expect("scoop binary should exist");
    fixture.write_user_config(
        r#"{"hold_update_until":"2099-01-01T00:00:00Z","last_update":"2000-01-01T00:00:00Z"}"#,
    );

    let output = run_binary_with_env(
        &["update"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(
        output
            .stdout
            .contains("Skipping self-update of Scoop Core until")
    );
    assert_eq!(
        fs::read(format!(
            "{}\\apps\\scoop\\current\\scoop.exe",
            fixture.local_root()
        ))
        .expect("scoop binary should remain"),
        b"old binary"
    );
}

#[test]
fn lifecycle_install_update_reset_uninstall_round_trip() {
    let fixture = InstallFixture::new();
    fixture.write_user_config(r#"{"last_update":"9999-01-01T00:00:00Z"}"#);
    let env_store = format!("{}\\env-store.json", fixture.payload_root());

    let old_archive = fixture.write_zip("demo-v1.zip", &[("demo.exe", b"demo v1 binary")]);
    let old_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&old_archive))
        .expect("old hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&old_archive),
            old_hash
        ),
    );

    let install_out = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );
    assert_eq!(install_out.status.code(), Some(0), "install should succeed");
    assert!(
        install_out.stderr.is_empty(),
        "install should be silent on stderr"
    );
    assert!(
        fs::read(format!(
            "{}\\apps\\demo\\current\\demo.exe",
            fixture.local_root()
        ))
        .is_ok_and(|bytes| bytes == b"demo v1 binary")
    );

    let new_archive = fixture.write_zip("demo-v2.zip", &[("demo.exe", b"demo v2 binary")]);
    let new_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&new_archive))
        .expect("new hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.4",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&new_archive),
            new_hash
        ),
    );

    let update_out = run_binary_with_env(
        &["update", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );
    assert_eq!(update_out.status.code(), Some(0), "update should succeed");
    assert_eq!(update_out.stderr, "");
    assert!(
        update_out
            .stdout
            .contains("'demo' was updated from 1.2.3 to 1.2.4.")
    );

    let current_binary = fs::read(format!(
        "{}\\apps\\demo\\current\\demo.exe",
        fixture.local_root()
    ))
    .expect("current binary should exist after update");
    assert_eq!(current_binary, b"demo v2 binary");

    let reset_out = run_binary_with_env(
        &["reset", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );
    assert_eq!(reset_out.status.code(), Some(0), "reset should succeed");
    assert_eq!(reset_out.stderr, "");
    assert!(reset_out.stdout.contains("'demo' (1.2.4) was reset."));

    let uninstall_out = run_binary_with_env(
        &["uninstall", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );
    assert_eq!(
        uninstall_out.status.code(),
        Some(0),
        "uninstall should succeed"
    );
    assert_eq!(uninstall_out.stderr, "");
    assert!(
        uninstall_out
            .stdout
            .contains("'demo' (1.2.4) was uninstalled.")
    );
}

#[test]
fn update_explicit_scoop_installs_new_versioned_binary() {
    let fixture = InstallFixture::new();
    let archive = fixture.write_zip("scoop-2.0.0.zip", &[("scoop.exe", b"new binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "scoop",
        &format!(
            r#"{{"version":"2.0.0","url":"{}","hash":"{}","bin":"scoop.exe"}}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    seed_installed_scoop(&fixture, "1.0.0", "main", b"old binary");
    fixture.write_user_config(r#"{"last_update":"9999-01-01T00:00:00Z"}"#);

    let output = run_binary_with_env(
        &["update", "scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("Scoop was updated successfully!"));
    assert_eq!(
        fs::read(format!(
            "{}\\apps\\scoop\\current\\scoop.exe",
            fixture.local_root()
        ))
        .expect("updated scoop binary should exist"),
        b"new binary"
    );
}

#[test]
fn update_app_reports_changelog_when_manifest_has_entry() {
    let fixture = InstallFixture::new();
    fixture.write_user_config(r#"{"last_update":"9999-01-01T00:00:00Z"}"#);
    let old_archive = fixture.write_zip("demo-old.zip", &[("demo.exe", b"old binary")]);
    let old_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&old_archive))
        .expect("old hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&old_archive),
            old_hash
        ),
    );
    let env_store = format!("{}\\env-store.json", fixture.payload_root());
    let install_out = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );
    assert_eq!(install_out.status.code(), Some(0));

    let new_archive = fixture.write_zip("demo-new.zip", &[("demo.exe", b"new binary")]);
    let new_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&new_archive))
        .expect("new hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.4",
                "url":"{}",
                "hash":"{}",
                "changelog":"https://example.invalid/changelog",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&new_archive),
            new_hash
        ),
    );

    let output = run_binary_with_env(
        &["update", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(
        output
            .stdout
            .contains("'demo' was updated from 1.2.3 to 1.2.4.")
    );
    assert!(
        output
            .stdout
            .contains("CHANGELOG: https://example.invalid/changelog")
    );
}

#[test]
fn install_self_updates_versioned_scoop_binary_when_outdated_and_flag_not_set() {
    let fixture = InstallFixture::new();
    let scoop_archive = fixture.write_zip("scoop-2.0.0.zip", &[("scoop.exe", b"new binary")]);
    let scoop_hash =
        scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&scoop_archive))
            .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "scoop",
        &format!(
            r#"{{"version":"2.0.0","url":"{}","hash":"{}","bin":"scoop.exe"}}"#,
            escape_json_path(&scoop_archive),
            scoop_hash
        ),
    );
    seed_installed_scoop(&fixture, "1.0.0", "main", b"old binary");
    fixture.write_user_config(r#"{"last_update":"2000-01-01T00:00:00Z"}"#);

    let archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{"version":"1.2.3","url":"{}","hash":"{}","bin":"demo.exe"}}"#,
            escape_json_path(&archive),
            hash
        ),
    );

    let output = run_binary_with_env(
        &["install", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.contains("Scoop was updated successfully!"));
    assert!(
        output
            .stdout
            .contains("'demo' (1.2.3) was installed successfully!")
    );
    assert_eq!(
        fs::read(format!(
            "{}\\apps\\scoop\\current\\scoop.exe",
            fixture.local_root()
        ))
        .expect("updated scoop binary should exist"),
        b"new binary"
    );
    assert!(
        std::path::Path::new(&format!(
            "{}\\apps\\demo\\current\\demo.exe",
            fixture.local_root()
        ))
        .exists()
    );
}

#[test]
fn update_already_latest_for_current_version() {
    let fixture = InstallFixture::new();
    fixture.write_user_config(r#"{"last_update":"9999-01-01T00:00:00Z"}"#);
    let archive = fixture.write_zip("demo.zip", &[("demo.exe", b"demo binary")]);
    let hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&archive))
        .expect("hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&archive),
            hash
        ),
    );
    let env_store = format!("{}\\env-store.json", fixture.payload_root());

    // Install
    let _ = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );

    // Update should report already latest
    let output = run_binary_with_env(
        &["update", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(
        output
            .stdout
            .contains("'demo' (1.2.3) is already up to date.")
    );
}

#[test]
fn update_skips_app_with_running_process() {
    let fixture = InstallFixture::new();
    fixture.write_user_config(r#"{"last_update":"9999-01-01T00:00:00Z"}"#);
    let old_archive = fixture.write_zip("demo-old.zip", &[("demo.exe", b"old binary")]);
    let old_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&old_archive))
        .expect("old hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.3",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&old_archive),
            old_hash
        ),
    );
    let env_store = format!("{}\\env-store.json", fixture.payload_root());

    let install_out = run_binary_with_env(
        &["install", "demo", "--no-update-scoop"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );
    assert_eq!(install_out.status.code(), Some(0));

    let new_archive = fixture.write_zip("demo-new.zip", &[("demo.exe", b"new binary")]);
    let new_hash = scoop_core::infra::hash::sha256_file(&camino::Utf8PathBuf::from(&new_archive))
        .expect("new hash should compute");
    fixture.bucket_manifest(
        "main",
        "demo",
        &format!(
            r#"{{
                "version":"1.2.4",
                "url":"{}",
                "hash":"{}",
                "bin":"demo.exe"
            }}"#,
            escape_json_path(&new_archive),
            new_hash
        ),
    );

    let running_path = format!("{}\\apps\\demo\\current\\demo.exe", fixture.local_root());
    let output = run_binary_with_env(
        &["update", "demo"],
        &[
            ("SCOOP", fixture.local_root()),
            ("SCOOP_GLOBAL", fixture.global_root()),
            ("SCOOP_RS_ENV_STORE", &env_store),
            ("SCOOP_RS_RUNNING_PROCESS_PATHS", &running_path),
            ("XDG_CONFIG_HOME", &fixture.config_home()),
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, "");
    assert!(
        output
            .stdout
            .contains("are still running. Close them and try again.")
    );
    assert!(
        output
            .stdout
            .contains("Running process detected, skip updating.")
    );
    let current = fs::read(format!(
        "{}\\apps\\demo\\current\\demo.exe",
        fixture.local_root()
    ))
    .expect("existing version should remain installed");
    assert_eq!(current, b"old binary");
}
