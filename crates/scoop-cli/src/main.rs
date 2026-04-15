mod registry;

use anyhow::Context;
use registry::{COMMANDS, find};
use scoop_core::InstalledApp;
use scoop_core::app::bucket::{
    BucketAddOutcome, BucketListEntry, add_bucket, known_bucket_names, list_buckets, remove_bucket,
};
use scoop_core::app::cache::{CacheEntry, remove_cache, show_cache};
use scoop_core::app::cat::render_manifest_for_app;
use scoop_core::app::cleanup::{CleanupOptions, CleanupOutcome, cleanup_apps};
use scoop_core::app::config::{
    current_config, get_config_value, remove_config_value, set_config_value,
};
use scoop_core::app::depends::{DependencyRow, list_dependencies};
use scoop_core::app::download::{DownloadOptions, DownloadOutcome, DownloadReport, download_apps};
use scoop_core::app::export::{export_state, render_export_json};
use scoop_core::app::hold::{HoldOutcome, hold_apps, unhold_apps};
use scoop_core::app::import::{ImportLoadError, build_import_plan, load_import_scoopfile};
use scoop_core::app::info::{describe_app, render_info};
use scoop_core::app::install::{InstallOptions, InstallOutcome};
use scoop_core::app::list::list_installed;
use scoop_core::app::reset::ResetOutcome;
use scoop_core::app::search::{
    RemoteSearchResult, SearchMode, SearchResult, compile_search_query, search_cached_buckets,
    search_local_buckets, search_remote_buckets, search_remote_buckets_partial,
};
use scoop_core::app::shim::{
    AlterShimOutcome, ShimInfo, ShimLookup, add_shim, alter_shim, list_shims, remove_shims,
    shim_info,
};
use scoop_core::app::status::{StatusRow, collect_status};
use scoop_core::app::uninstall::{UninstallOptions, UninstallOutcome};
use scoop_core::app::update::{UpdateOptions, UpdateOutcome};
use scoop_core::app::virustotal::{
    EXIT_NO_API_KEY, VirusTotalOptions, check_apps as check_virustotal_apps,
};
use scoop_core::infra::http::build_blocking_http_client;
use scoop_core::infra::installed::format_updated_time;
use scoop_core::{resolve_prefix, resolve_which};
use serde_json::{Map, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const HELP_ALIASES: &[&str] = &["-h", "--help", "/?"];
const VERSION_ALIASES: &[&str] = &["-v", "--version"];
const EOL: &str = "\r\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum OutputLevel {
    Error,
    Warn,
    Info,
    Verbose,
}

fn emit(output: &mut String, current_level: OutputLevel, message_level: OutputLevel, text: &str) {
    if current_level >= message_level {
        output.push_str(text);
    }
}

struct Response {
    code: i32,
    output: String,
}

fn main() -> anyhow::Result<()> {
    init_tracing()?;

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let response = run(&args);

    if !response.output.is_empty() {
        std::io::stdout()
            .write_all(response.output.as_bytes())
            .context("failed to write CLI output")?;
    }

    std::process::exit(response.code);
}

fn init_tracing() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .try_init()
        .context("failed to initialize tracing")
}

fn run(args: &[String]) -> Response {
    match args.split_first() {
        None => Response {
            code: 0,
            output: render_help_overview(),
        },
        Some((first, _)) if HELP_ALIASES.contains(&first.as_str()) => Response {
            code: 0,
            output: render_help_overview(),
        },
        Some((first, _)) if VERSION_ALIASES.contains(&first.as_str()) => Response {
            code: 0,
            output: render_version(),
        },
        Some((first, remaining)) if first == "bucket" => handle_bucket(remaining),
        Some((first, remaining)) if first == "help" => handle_help(remaining),
        Some((first, remaining)) if first == "cache" => handle_cache(remaining),
        Some((first, remaining))
            if HELP_ALIASES
                .contains(&remaining.first().map(String::as_str).unwrap_or_default()) =>
        {
            handle_help(std::slice::from_ref(first))
        }
        Some((first, remaining)) if first == "config" => handle_config(remaining),
        Some((first, remaining)) if first == "cleanup" => handle_cleanup(remaining),
        Some((first, remaining)) if first == "depends" => handle_depends(remaining),
        Some((first, remaining)) if first == "download" => handle_download(remaining),
        Some((first, remaining)) if first == "export" => handle_export(remaining),
        Some((first, remaining)) if first == "hold" => handle_hold(remaining),
        Some((first, remaining)) if first == "import" => handle_import(remaining),
        Some((first, remaining)) if first == "list" => handle_list(remaining),
        Some((first, remaining)) if first == "cat" => handle_cat(remaining),
        Some((first, remaining)) if first == "info" => handle_info(remaining),
        Some((first, remaining)) if first == "install" => handle_install(remaining),
        Some((first, remaining)) if first == "search" => handle_search(remaining),
        Some((first, remaining)) if first == "shim" => handle_shim(remaining),
        Some((first, remaining)) if first == "status" => handle_status(remaining),
        Some((first, remaining)) if first == "unhold" => handle_unhold(remaining),
        Some((first, remaining)) if first == "uninstall" => handle_uninstall(remaining),
        Some((first, remaining)) if first == "update" => handle_update(remaining),
        Some((first, remaining)) if first == "virustotal" => handle_virustotal(remaining),
        Some((first, remaining)) if first == "reset" => handle_reset(remaining),
        Some((first, remaining)) if first == "reinstall" => handle_reinstall(remaining),
        Some((first, remaining)) if first == "prefix" => handle_prefix(remaining),
        Some((first, remaining)) if first == "which" => handle_which(remaining),
        Some((first, _)) => match find(first) {
            Some(spec) if spec.implemented => Response {
                code: 1,
                output: format!("ERROR scoop: internal dispatch failed for command '{first}'{EOL}"),
            },
            Some(_) => Response {
                code: 1,
                output: format!(
                    "ERROR scoop: command '{first}' is not implemented yet in scoop-rs{EOL}"
                ),
            },
            None => Response {
                code: 1,
                output: format!(
                    "WARN  scoop: '{first}' isn't a scoop command. See 'scoop help'.{EOL}"
                ),
            },
        },
    }
}

fn handle_help(args: &[String]) -> Response {
    match args.first() {
        None => Response {
            code: 0,
            output: render_help_overview(),
        },
        Some(command) => match find(command) {
            Some(spec) => Response {
                code: 0,
                output: render_command_help(spec.usage, spec.help),
            },
            None => Response {
                code: 0,
                output: format!("ERROR scoop help: no such command '{command}'{EOL}"),
            },
        },
    }
}

fn handle_bucket(args: &[String]) -> Response {
    match run_bucket(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_cache(args: &[String]) -> Response {
    match run_cache(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_config(args: &[String]) -> Response {
    match run_config(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_cleanup(args: &[String]) -> Response {
    match run_cleanup(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_depends(args: &[String]) -> Response {
    match run_depends(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 0,
            output: format!("{error}{EOL}"),
        },
    }
}

fn handle_download(args: &[String]) -> Response {
    match run_download(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_export(args: &[String]) -> Response {
    match run_export(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_hold(args: &[String]) -> Response {
    match run_hold(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_import(args: &[String]) -> Response {
    match run_import(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_shim(args: &[String]) -> Response {
    match run_shim(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_reinstall(args: &[String]) -> Response {
    match run_reinstall(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_virustotal(args: &[String]) -> Response {
    match run_virustotal(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn render_version() -> String {
    format!(
        "Current Scoop version:{EOL}scoop-rs {}{EOL}",
        env!("CARGO_PKG_VERSION")
    )
}

fn handle_list(args: &[String]) -> Response {
    match run_list(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_cat(args: &[String]) -> Response {
    match run_cat(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_info(args: &[String]) -> Response {
    match run_info(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_install(args: &[String]) -> Response {
    match run_install(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_uninstall(args: &[String]) -> Response {
    match run_uninstall(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_update(args: &[String]) -> Response {
    match run_update(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_reset(args: &[String]) -> Response {
    match run_reset(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_search(args: &[String]) -> Response {
    match run_search(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_status(args: &[String]) -> Response {
    match run_status(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_unhold(args: &[String]) -> Response {
    match run_unhold(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_prefix(args: &[String]) -> Response {
    match run_prefix(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn handle_which(args: &[String]) -> Response {
    match run_which(args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    }
}

fn render_help_overview() -> String {
    let mut output = [
        "Usage: scoop <command> [<args>]",
        "",
        "Available commands are listed below.",
        "",
        "Type 'scoop help <command>' to get more help for a specific command.",
        "",
    ]
    .join(EOL);
    output.push_str(EOL);
    output.push_str(&format!("{:<11} {}{EOL}", "Command", "Summary"));
    output.push_str(&format!("{:<11} {}{EOL}", "-------", "-------"));
    for command in COMMANDS {
        output.push_str(&format!("{:<11} {}{EOL}", command.name, command.summary));
    }
    output
}

fn render_command_help(usage: &str, help: Option<&str>) -> String {
    match help {
        Some(help_text) if !help_text.is_empty() => format!("{usage}{EOL}{EOL}{help_text}{EOL}"),
        _ => format!("{usage}{EOL}{EOL}"),
    }
}

fn run_bucket(args: &[String]) -> anyhow::Result<Response> {
    let config = scoop_core::RuntimeConfig::detect(None);
    match args {
        [cmd, name] if cmd == "rm" => {
            let removed = remove_bucket(&config, name)?;
            Ok(Response {
                code: if removed { 0 } else { 1 },
                output: if removed {
                    format!("The {name} bucket was removed successfully.{EOL}")
                } else {
                    format!("ERROR '{name}' bucket not found.{EOL}")
                },
            })
        }
        [cmd] if cmd == "rm" => Ok(Response {
            code: 0,
            output: String::from("ERROR <name> missing\r\nusage: scoop bucket rm <name>\r\n"),
        }),
        [cmd, name, repo] if cmd == "add" => {
            render_bucket_add(add_bucket(&config, name, Some(repo))?)
        }
        [cmd, name] if cmd == "add" => match add_bucket(&config, name, None) {
            Ok(outcome) => render_bucket_add(outcome),
            Err(error) if error.to_string().starts_with("Unknown bucket ") => Ok(Response {
                code: 1,
                output: format!("ERROR {error}{EOL}usage: scoop bucket add <name> [<repo>]{EOL}"),
            }),
            Err(error) => Err(error),
        },
        [cmd] if cmd == "add" => Ok(Response {
            code: 0,
            output: String::from(
                "ERROR <name> missing\r\nusage: scoop bucket add <name> [<repo>]\r\n",
            ),
        }),
        [cmd] if cmd == "known" => Ok(Response {
            code: 0,
            output: render_bucket_known(&known_bucket_names(&config)?),
        }),
        [cmd] if cmd == "list" => {
            let rows = list_buckets(&config)?;
            if rows.is_empty() {
                return Ok(Response {
                    code: 2,
                    output: String::from(
                        "WARN  No bucket found. Please run 'scoop bucket add main' to add the default 'main' bucket.\r\n",
                    ),
                });
            }
            Ok(Response {
                code: 0,
                output: render_bucket_list(&rows)?,
            })
        }
        [cmd, ..] => Ok(Response {
            code: 1,
            output: format!("ERROR scoop bucket: cmd '{cmd}' not supported{EOL}"),
        }),
        [] => Ok(Response {
            code: 1,
            output: String::from("ERROR scoop bucket: cmd '' not supported\r\n"),
        }),
    }
}

fn render_bucket_add(outcome: BucketAddOutcome) -> anyhow::Result<Response> {
    Ok(match outcome {
        BucketAddOutcome::Added { name } => Response {
            code: 0,
            output: format!("The {name} bucket was added successfully.{EOL}"),
        },
        BucketAddOutcome::AlreadyExists { name } => Response {
            code: 2,
            output: format!(
                "WARN  The '{name}' bucket already exists. To add this bucket again, first remove it by running 'scoop bucket rm {name}'.{EOL}"
            ),
        },
        BucketAddOutcome::DuplicateRemote {
            existing_bucket,
            name: _,
        } => Response {
            code: 2,
            output: format!(
                "WARN  Bucket {existing_bucket} already exists for that repository{EOL}"
            ),
        },
    })
}

fn run_cache(args: &[String]) -> anyhow::Result<Response> {
    let mut subcommand = "show";
    let mut remaining = args;
    if let Some(first) = args.first().map(String::as_str)
        && matches!(first, "show" | "rm")
    {
        subcommand = first;
        remaining = &args[1..];
    }

    let config = scoop_core::RuntimeConfig::detect(None);
    match subcommand {
        "show" => {
            let filters = remaining.to_vec();
            let report = show_cache(&config, &filters)?;
            Ok(Response {
                code: 0,
                output: render_cache_show(&report.entries, report.total_length),
            })
        }
        "rm" => {
            if remaining.is_empty() {
                return Ok(Response {
                    code: 1,
                    output: format!(
                        "ERROR <app(s)> missing{EOL}Usage: scoop cache rm <app(s)>{EOL}"
                    ),
                });
            }
            let all = remaining
                .iter()
                .any(|arg| matches!(arg.as_str(), "*" | "-a" | "--all"));
            let filters = remaining
                .iter()
                .filter(|arg| !matches!(arg.as_str(), "*" | "-a" | "--all"))
                .cloned()
                .collect::<Vec<_>>();
            let report = remove_cache(&config, &filters, all)?;
            Ok(Response {
                code: 0,
                output: render_cache_remove(&report.entries, report.total_length),
            })
        }
        _ => unreachable!(),
    }
}

fn run_config(args: &[String]) -> anyhow::Result<Response> {
    let config = scoop_core::RuntimeConfig::detect(None);
    match args {
        [] => Ok(Response {
            code: 0,
            output: render_config_map(&current_config(&config)),
        }),
        [name] => match get_config_value(&config, name) {
            Some(value) => Ok(Response {
                code: 0,
                output: format!("{}{EOL}", render_config_value(&value)),
            }),
            None => Ok(Response {
                code: 0,
                output: format!("'{}' is not set{EOL}", name.to_ascii_lowercase()),
            }),
        },
        [rm, name] if rm == "rm" => {
            remove_config_value(&config, name)?;
            Ok(Response {
                code: 0,
                output: format!("'{}' has been removed{EOL}", name.to_ascii_lowercase()),
            })
        }
        [name, value] => {
            let result = set_config_value(&config, name, value)?;
            let mut output = String::new();
            if result.initialized_sqlite_cache {
                output.push_str(
                    "INFO  Initializing SQLite cache in progress... This may take a while, please wait.\r\n",
                );
            }
            output.push_str(&format!(
                "'{}' has been set to '{}'{EOL}",
                result.name,
                render_config_value_inline(&result.value)
            ));
            Ok(Response { code: 0, output })
        }
        _ => Ok(Response {
            code: 1,
            output: String::from("Usage: scoop config [rm] name [value]\r\n"),
        }),
    }
}

fn run_cleanup(args: &[String]) -> anyhow::Result<Response> {
    let mut options = CleanupOptions::default();
    let mut apps = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-a" | "--all" | "*" => options.all = true,
            "-g" | "--global" => options.global = true,
            "-k" | "--cache" => options.cache = true,
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 1,
                    output: format!("ERROR scoop cleanup: Option {value} not recognized.{EOL}"),
                });
            }
            value => apps.push(value.to_owned()),
        }
    }
    if apps.is_empty() && !options.all {
        return Ok(Response {
            code: 0,
            output: String::from(
                "ERROR <app> missing\r\nUsage: scoop cleanup <app> [options]\r\r\n",
            ),
        });
    }
    let config = scoop_core::RuntimeConfig::detect(None);
    let outcomes = cleanup_apps(&config, &apps, &options)?;
    Ok(Response {
        code: 0,
        output: render_cleanup_outcomes(&outcomes, options.all),
    })
}

fn run_hold(args: &[String]) -> anyhow::Result<Response> {
    let mut global = false;
    let mut apps = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-g" | "--global" => global = true,
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 1,
                    output: format!("ERROR scoop hold: Option {value} not recognized.{EOL}"),
                });
            }
            value => apps.push(value.to_owned()),
        }
    }

    if apps.is_empty() {
        return Ok(Response {
            code: 0,
            output: String::from("Usage: scoop hold <apps>\r\r\n"),
        });
    }

    let config = scoop_core::RuntimeConfig::detect(None);
    let outcomes = hold_apps(&config, &apps, global)?;
    Ok(Response {
        code: 0,
        output: render_hold_outcomes(&outcomes, true),
    })
}

fn run_unhold(args: &[String]) -> anyhow::Result<Response> {
    let mut global = false;
    let mut apps = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-g" | "--global" => global = true,
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 1,
                    output: format!("ERROR scoop unhold: Option {value} not recognized.{EOL}"),
                });
            }
            value => apps.push(value.to_owned()),
        }
    }

    if apps.is_empty() {
        return Ok(Response {
            code: 0,
            output: String::from("Usage: scoop unhold <app>\r\r\n"),
        });
    }

    let config = scoop_core::RuntimeConfig::detect(None);
    let outcomes = unhold_apps(&config, &apps, global)?;
    Ok(Response {
        code: 0,
        output: render_hold_outcomes(&outcomes, false),
    })
}

fn run_list(args: &[String]) -> anyhow::Result<Response> {
    let config = scoop_core::RuntimeConfig::detect(None);
    let report = list_installed(&config, args.first().map(String::as_str))?;
    if report.apps.is_empty() && report.query.is_none() {
        return Ok(Response {
            code: 1,
            output: format!("WARN  There aren't any apps installed.{EOL}"),
        });
    }

    let mut output = String::new();
    match report.query.as_deref() {
        Some(query) => output.push_str(&format!("Installed apps matching '{query}':{EOL}")),
        None => output.push_str(&format!("Installed apps:{EOL}")),
    }

    if report.apps.is_empty() {
        return Ok(Response { code: 0, output });
    }

    output.push_str(EOL);
    output.push_str(&render_list_table(&report.apps)?);

    Ok(Response { code: 0, output })
}

fn run_cat(args: &[String]) -> anyhow::Result<Response> {
    let Some(app) = args.first() else {
        return Ok(Response {
            code: 0,
            output: String::from("ERROR <app> missing\r\nUsage: scoop cat <app>\r\r\n"),
        });
    };

    let config = scoop_core::RuntimeConfig::detect(None);
    match render_manifest_for_app(&config, app)? {
        Some(rendered) => {
            let output = match config.settings().cat_style.as_deref() {
                Some(style) if !style.is_empty() => render_with_bat(&rendered, style),
                _ => rendered,
            };
            Ok(Response {
                code: 0,
                output: format!("{output}{EOL}"),
            })
        }
        None => Ok(Response {
            code: 0,
            output: format!("Couldn't find manifest for '{app}'.{EOL}"),
        }),
    }
}

fn run_info(args: &[String]) -> anyhow::Result<Response> {
    let mut verbose = false;
    let mut app = None;
    for arg in args {
        match arg.as_str() {
            "-v" | "--verbose" => verbose = true,
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 1,
                    output: format!("ERROR scoop info: unknown option '{value}'{EOL}"),
                });
            }
            value if app.is_none() => app = Some(value),
            value => {
                return Ok(Response {
                    code: 1,
                    output: format!("ERROR scoop info: unexpected argument '{value}'{EOL}"),
                });
            }
        }
    }
    let Some(app) = app else {
        return Ok(Response {
            code: 1,
            output: String::from("Usage: scoop info <app>\r\n"),
        });
    };

    let config = scoop_core::RuntimeConfig::detect(None);
    match describe_app(&config, app, verbose)? {
        Some(info) => Ok(Response {
            code: 0,
            output: render_info(&info)?,
        }),
        None => Ok(Response {
            code: 1,
            output: format!("Could not find manifest for '{app}' in local buckets.{EOL}"),
        }),
    }
}

fn run_depends(args: &[String]) -> anyhow::Result<Response> {
    let mut architecture = None;
    let mut app = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-a" | "--arch" => {
                let Some(value) = args.get(index + 1) else {
                    return Ok(Response {
                        code: 0,
                        output: String::from(
                            "ERROR scoop depends: Option --arch requires an argument.\r\n",
                        ),
                    });
                };
                architecture = Some(value.clone());
                index += 1;
            }
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 0,
                    output: format!("ERROR scoop depends: Option {value} not recognized.{EOL}"),
                });
            }
            value if app.is_none() => app = Some(value),
            value => {
                return Ok(Response {
                    code: 0,
                    output: format!("ERROR scoop depends: unexpected argument '{value}'{EOL}"),
                });
            }
        }
        index += 1;
    }

    let Some(app) = app else {
        return Ok(Response {
            code: 0,
            output: String::from("ERROR <app> missing\r\nUsage: scoop depends <app>\r\r\n"),
        });
    };

    let config = scoop_core::RuntimeConfig::detect(None);
    let rows = list_dependencies(&config, app, architecture.as_deref())?;
    Ok(Response {
        code: 0,
        output: render_dependency_table(&rows),
    })
}

fn run_download(args: &[String]) -> anyhow::Result<Response> {
    let mut options = DownloadOptions::default();
    let mut no_update_scoop = false;
    let mut apps = Vec::<String>::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-f" | "--force" => options.use_cache = false,
            "-s" | "--skip-hash-check" => options.check_hash = false,
            "-u" | "--no-update-scoop" => no_update_scoop = true,
            "-a" | "--arch" => {
                let Some(value) = args.get(index + 1) else {
                    return Ok(Response {
                        code: 0,
                        output: String::from(
                            "ERROR scoop download: Option --arch requires an argument.\r\n",
                        ),
                    });
                };
                options.architecture = Some(value.clone());
                index += 1;
            }
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 0,
                    output: format!("ERROR scoop download: Option {value} not recognized.{EOL}"),
                });
            }
            value => apps.push(value.to_owned()),
        }
        index += 1;
    }

    if apps.is_empty() {
        return Ok(Response {
            code: 0,
            output: String::from(
                "ERROR <app> missing\r\nUsage: scoop download <app> [options]\r\r\n",
            ),
        });
    }

    let mut deduped_apps = Vec::new();
    for app in apps {
        if !deduped_apps.contains(&app) {
            deduped_apps.push(app);
        }
    }

    let config = scoop_core::RuntimeConfig::detect(None);
    let mut output = String::new();
    if scoop_core::app::update::is_scoop_outdated(&config)? {
        if no_update_scoop {
            output.push_str("WARN  Scoop is out of date.\r\n");
        } else {
            output.push_str(&render_update_outcome(
                &scoop_core::app::update::update_scoop(&config)?,
                OutputLevel::Info,
            ));
        }
    }
    if !options.use_cache {
        output.push_str("WARN  Cache is being ignored.\r\n");
    }

    for report in download_apps(&config, &deduped_apps, &options)? {
        output.push_str(&render_download_report(&report));
    }

    Ok(Response { code: 0, output })
}

fn run_export(args: &[String]) -> anyhow::Result<Response> {
    let include_config = args
        .first()
        .is_some_and(|arg| matches!(arg.as_str(), "-c" | "--config"));
    let config = scoop_core::RuntimeConfig::detect(None);
    let rendered = render_export_json(&export_state(&config, include_config)?)?;
    Ok(Response {
        code: 0,
        output: format!("{rendered}{EOL}"),
    })
}

fn run_import(args: &[String]) -> anyhow::Result<Response> {
    let Some(source) = args.first() else {
        return Ok(Response {
            code: 0,
            output: String::from("ERROR <path> missing\r\nUsage: scoop import <path>\r\r\n"),
        });
    };

    let config = scoop_core::RuntimeConfig::detect(None);
    let scoopfile = match load_import_scoopfile(&config, source) {
        Ok(scoopfile) => scoopfile,
        Err(ImportLoadError::LocalJsonParse { path }) => {
            return Ok(Response {
                code: 0,
                output: format!(
                    "WARN  Error parsing JSON at '{path}'.{EOL}Input file not a valid JSON.{EOL}"
                ),
            });
        }
        Err(ImportLoadError::InvalidJson) => {
            return Ok(Response {
                code: 0,
                output: format!("Input file not a valid JSON.{EOL}"),
            });
        }
    };

    let mut output = String::new();
    for (name, value) in &scoopfile.config {
        let result = scoop_core::app::config::set_config_json_value(&config, name, value.clone())?;
        output.push_str(&format!(
            "'{}' has been set to '{}'{EOL}",
            result.name,
            render_config_value_inline(&result.value)
        ));
    }
    for bucket in &scoopfile.buckets {
        let _ = add_bucket(&config, &bucket.name, Some(&bucket.source))?;
    }
    for plan in build_import_plan(&scoopfile) {
        let mut install_args = Vec::new();
        if plan.global {
            install_args.push(String::from("--global"));
        }
        if let Some(architecture) = &plan.architecture {
            install_args.push(String::from("--arch"));
            install_args.push(architecture.clone());
        }
        install_args.push(plan.app_reference);
        output.push_str(&match run_install(&install_args) {
            Ok(response) => response.output,
            Err(error) => format!("ERROR {error}{EOL}"),
        });

        if plan.hold {
            let mut hold_args = Vec::new();
            if plan.global {
                hold_args.push(String::from("--global"));
            }
            hold_args.push(plan.name);
            output.push_str(&match run_hold(&hold_args) {
                Ok(response) => response.output,
                Err(error) => format!("ERROR {error}{EOL}"),
            });
        }
    }

    Ok(Response { code: 0, output })
}

fn run_shim(args: &[String]) -> anyhow::Result<Response> {
    const USAGE: &str =
        "Usage: scoop shim <subcommand> [<shim_name>...] [options] [other_args]\r\r\n";
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Ok(Response {
            code: 0,
            output: format!("ERROR <subcommand> missing{EOL}{USAGE}"),
        });
    };
    if !matches!(subcommand, "add" | "rm" | "list" | "info" | "alter") {
        return Ok(Response {
            code: 1,
            output: format!(
                "ERROR '{subcommand}' is not one of available subcommands: add, rm, list, info, alter{EOL}{USAGE}"
            ),
        });
    }

    let mut global = false;
    let mut other = Vec::<String>::new();
    for arg in &args[1..] {
        match arg.as_str() {
            "-g" | "--global" => global = true,
            value => other.push(value.to_owned()),
        }
    }
    if subcommand != "list" && other.is_empty() {
        return Ok(Response {
            code: 0,
            output: format!(
                "ERROR <shim_name> must be specified for subcommand '{subcommand}'{EOL}{USAGE}"
            ),
        });
    }

    let config = scoop_core::RuntimeConfig::detect(None);
    match subcommand {
        "add" => {
            if other.len() < 2 || other[1].is_empty() {
                return Ok(Response {
                    code: 0,
                    output: format!(
                        "ERROR <command_path> must be specified for subcommand 'add'{EOL}{USAGE}"
                    ),
                });
            }
            let shim_name = other.remove(0);
            let command_path = other.remove(0);
            let mut shim_args = other;
            if matches!(
                shim_args.first().map(String::as_str),
                Some("--") | Some("--%")
            ) {
                shim_args.remove(0);
            }
            add_shim(&config, &shim_name, &command_path, &shim_args, global)?;
            Ok(Response {
                code: 0,
                output: format!(
                    "Adding {} shim {shim_name}...{EOL}",
                    if global { "global" } else { "local" }
                ),
            })
        }
        "rm" => {
            let missing = remove_shims(&config, &other, global)?;
            let mut output = String::new();
            for name in missing {
                output.push_str(&format!(
                    "ERROR {} shim not found: {name}{EOL}",
                    if global { "Global" } else { "Local" }
                ));
            }
            Ok(Response {
                code: if output.is_empty() { 0 } else { 3 },
                output,
            })
        }
        "list" => Ok(Response {
            code: 0,
            output: render_shim_list(&list_shims(&config, &other, global)?)?,
        }),
        "info" => match shim_info(&config, &other[0], global)? {
            ShimLookup::Found(info) => Ok(Response {
                code: 0,
                output: render_shim_info(&info),
            }),
            ShimLookup::Missing { other_scope_exists } => {
                let mut output = format!(
                    "ERROR {} shim not found: {}{EOL}",
                    if global { "Global" } else { "Local" },
                    other[0]
                );
                if other_scope_exists {
                    output.push_str(&format!(
                        "But a {} shim exists, run 'scoop shim info {}{}' to show its info.{EOL}",
                        if global { "local" } else { "global" },
                        other[0],
                        if global { "" } else { " --global" }
                    ));
                }
                Ok(Response {
                    code: if other_scope_exists { 2 } else { 3 },
                    output,
                })
            }
        },
        "alter" => match alter_shim(&config, &other[0], global)? {
            AlterShimOutcome::Altered { name, from, to } => Ok(Response {
                code: 0,
                output: format!("{name} is now using {to} instead of {from}.{EOL}"),
            }),
            AlterShimOutcome::NoAlternatives { name } => Ok(Response {
                code: 2,
                output: format!("ERROR No alternatives of {name} found.{EOL}"),
            }),
            AlterShimOutcome::Missing { other_scope_exists } => {
                let mut output = format!(
                    "ERROR {} shim not found: {}{EOL}",
                    if global { "Global" } else { "Local" },
                    other[0]
                );
                if other_scope_exists {
                    output.push_str(&format!(
                        "But a {} shim exists, run 'scoop shim alter {}{}' to alternate its source.{EOL}",
                        if global { "local" } else { "global" },
                        other[0],
                        if global { "" } else { " --global" }
                    ));
                }
                Ok(Response {
                    code: if other_scope_exists { 2 } else { 3 },
                    output,
                })
            }
        },
        _ => unreachable!(),
    }
}

fn run_virustotal(args: &[String]) -> anyhow::Result<Response> {
    let mut all = false;
    let mut no_depends = false;
    let mut no_update_scoop = false;
    let mut passthru = false;
    let mut scan = false;
    let mut apps = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-a" | "--all" | "*" => all = true,
            "-n" | "--no-depends" => no_depends = true,
            "-u" | "--no-update-scoop" => no_update_scoop = true,
            "-p" | "--passthru" => passthru = true,
            "-s" | "--scan" => scan = true,
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 1,
                    output: format!("ERROR scoop virustotal: Option {value} not recognized.{EOL}"),
                });
            }
            value => apps.push(value.to_owned()),
        }
    }
    if apps.is_empty() && !all {
        return Ok(Response {
            code: 0,
            output: String::from("Usage: scoop virustotal [* | app1 app2 ...] [options]\r\r\n"),
        });
    }

    let config = scoop_core::RuntimeConfig::detect(None);
    let mut output = String::new();
    if scoop_core::app::update::is_scoop_outdated(&config)? {
        if no_update_scoop {
            output.push_str("WARN  Scoop is out of date.\r\n");
        } else {
            output.push_str(&render_update_outcome(
                &scoop_core::app::update::update_scoop(&config)?,
                OutputLevel::Info,
            ));
        }
    }
    if all {
        apps = list_installed(&config, None)?
            .apps
            .into_iter()
            .map(|app| app.name)
            .collect();
    }
    if !no_depends {
        let mut expanded = Vec::new();
        for app in &apps {
            if !expanded.contains(app) {
                expanded.push(app.clone());
            }
            if let Ok(rows) = list_dependencies(&config, app, None) {
                for row in rows {
                    if !expanded.contains(&row.name) {
                        expanded.push(row.name);
                    }
                }
            }
        }
        apps = expanded;
    }

    let api_key = get_config_value(&config, "virustotal_api_key")
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|value| !value.is_empty());
    if api_key.is_none() {
        return Ok(Response {
            code: EXIT_NO_API_KEY,
            output: String::from(
                "VirusTotal API key is not configured\r\n  You could get one from https://www.virustotal.com/gui/my-apikey and set with\r\n  scoop config virustotal_api_key <API key>\r\n",
            ),
        });
    }

    let run = check_virustotal_apps(
        &config,
        &apps,
        api_key.as_deref().expect("api key already validated"),
        &VirusTotalOptions {
            scan,
            base_url: std::env::var("SCOOP_RS_VIRUSTOTAL_BASE_URL").ok(),
        },
    )?;
    for line in &run.lines {
        output.push_str(line);
        output.push_str(EOL);
    }
    if passthru && !run.reports.is_empty() {
        output.push_str(&format!(
            "{}{EOL}",
            serde_json::to_string_pretty(&run.reports)
                .context("failed to serialize virustotal reports")?
                .replace('\n', "\r\n")
        ));
    }
    Ok(Response {
        code: run.exit_code,
        output,
    })
}

fn run_reinstall(args: &[String]) -> anyhow::Result<Response> {
    let mut install_options = InstallOptions::default();
    let mut uninstall_options = UninstallOptions::default();
    let mut output_level = OutputLevel::Info;
    let mut apps = Vec::<String>::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-g" | "--global" => {
                install_options.global = true;
                uninstall_options.global = true;
            }
            "-p" | "--purge" => uninstall_options.purge = true,
            "-i" | "--independent" => install_options.independent = true,
            "-k" | "--no-cache" => install_options.use_cache = false,
            "-s" | "--skip-hash-check" => install_options.check_hash = false,
            "-u" | "--no-update-scoop" => install_options.no_update_scoop = true,
            "-q" | "--quiet" => output_level = OutputLevel::Warn,
            "-v" | "--verbose" => output_level = OutputLevel::Verbose,
            "-a" | "--arch" => {
                let Some(value) = args.get(index + 1) else {
                    return Ok(Response {
                        code: 0,
                        output: String::from(
                            "ERROR scoop reinstall: Option --arch requires an argument.\r\n",
                        ),
                    });
                };
                install_options.architecture = Some(value.clone());
                index += 1;
            }
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 0,
                    output: format!("ERROR scoop reinstall: Option {value} not recognized.{EOL}"),
                });
            }
            value => apps.push(value.to_owned()),
        }
        index += 1;
    }

    if apps.is_empty() {
        let mut output = run_uninstall(&[]).map(|response| response.output)?;
        output.push_str(&run_install(&[]).map(|response| response.output)?);
        return Ok(Response { code: 0, output });
    }

    let mut uninstall_args = Vec::new();
    if uninstall_options.global {
        uninstall_args.push(String::from("--global"));
    }
    if uninstall_options.purge {
        uninstall_args.push(String::from("--purge"));
    }
    if output_level == OutputLevel::Warn {
        uninstall_args.push(String::from("--quiet"));
    } else if output_level == OutputLevel::Verbose {
        uninstall_args.push(String::from("--verbose"));
    }
    uninstall_args.extend(apps.clone());

    let mut install_args = Vec::new();
    if install_options.global {
        install_args.push(String::from("--global"));
    }
    if install_options.independent {
        install_args.push(String::from("--independent"));
    }
    if !install_options.use_cache {
        install_args.push(String::from("--no-cache"));
    }
    if !install_options.check_hash {
        install_args.push(String::from("--skip-hash-check"));
    }
    if install_options.no_update_scoop {
        install_args.push(String::from("--no-update-scoop"));
    }
    if output_level == OutputLevel::Warn {
        install_args.push(String::from("--quiet"));
    } else if output_level == OutputLevel::Verbose {
        install_args.push(String::from("--verbose"));
    }
    if let Some(architecture) = install_options.architecture {
        install_args.push(String::from("--arch"));
        install_args.push(architecture);
    }
    install_args.extend(apps);

    let uninstall_response = match run_uninstall(&uninstall_args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    };
    let install_response = match run_install(&install_args) {
        Ok(response) => response,
        Err(error) => Response {
            code: 1,
            output: format!("ERROR {error}{EOL}"),
        },
    };
    let mut output = uninstall_response.output;
    output.push_str(&install_response.output);
    Ok(Response {
        code: install_response.code,
        output,
    })
}

fn run_install(args: &[String]) -> anyhow::Result<Response> {
    let mut options = InstallOptions::default();
    let mut output_level = OutputLevel::Info;
    let mut apps = Vec::<String>::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-g" | "--global" => options.global = true,
            "-i" | "--independent" => options.independent = true,
            "-k" | "--no-cache" => options.use_cache = false,
            "-s" | "--skip-hash-check" => options.check_hash = false,
            "-u" | "--no-update-scoop" => options.no_update_scoop = true,
            "-q" | "--quiet" => output_level = OutputLevel::Warn,
            "-v" | "--verbose" => output_level = OutputLevel::Verbose,
            "-a" | "--arch" => {
                let Some(value) = args.get(index + 1) else {
                    return Ok(Response {
                        code: 0,
                        output: String::from(
                            "ERROR scoop install: Option --arch requires an argument.\r\n",
                        ),
                    });
                };
                options.architecture = Some(value.clone());
                index += 1;
            }
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 0,
                    output: format!("ERROR scoop install: Option {value} not recognized.{EOL}"),
                });
            }
            value => apps.push(value.to_owned()),
        }
        index += 1;
    }

    if apps.is_empty() {
        return Ok(Response {
            code: 0,
            output: String::from(
                "ERROR <app> missing\r\nUsage: scoop install <app> [options]\r\r\n",
            ),
        });
    }

    let config = scoop_core::RuntimeConfig::detect(None);
    let mut output = String::new();
    if scoop_core::app::update::is_scoop_outdated(&config)? {
        if options.no_update_scoop {
            emit(
                &mut output,
                output_level,
                OutputLevel::Warn,
                "WARN  Scoop is out of date.\r\n",
            );
        } else {
            output.push_str(&render_update_outcome(
                &scoop_core::app::update::update_scoop(&config)?,
                output_level,
            ));
        }
    }
    let outcomes = scoop_core::app::install::install_apps(&config, &apps, &options)?;
    for outcome in &outcomes {
        output.push_str(&render_install_outcome(outcome, &options, output_level));
    }
    output.push_str(&render_install_suggestions(
        &config,
        &outcomes,
        output_level,
    ));
    Ok(Response { code: 0, output })
}

fn run_uninstall(args: &[String]) -> anyhow::Result<Response> {
    let mut options = UninstallOptions::default();
    let mut apps = Vec::<String>::new();
    let mut output_level = OutputLevel::Info;
    for arg in args {
        match arg.as_str() {
            "-g" | "--global" => options.global = true,
            "-p" | "--purge" => options.purge = true,
            "-q" | "--quiet" => output_level = OutputLevel::Warn,
            "-v" | "--verbose" => output_level = OutputLevel::Verbose,
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 1,
                    output: format!("ERROR scoop uninstall: Option {value} not recognized.{EOL}"),
                });
            }
            value => apps.push(value.to_owned()),
        }
    }
    if apps.is_empty() {
        return Ok(Response {
            code: 1,
            output: format!("ERROR <app> missing{EOL}Usage: scoop uninstall <app> [options]{EOL}"),
        });
    }

    let config = scoop_core::RuntimeConfig::detect(None);
    let mut output = String::new();
    for outcome in scoop_core::app::uninstall::uninstall_apps(&config, &apps, &options)? {
        output.push_str(&render_uninstall_outcome(&outcome, output_level));
    }
    Ok(Response { code: 0, output })
}

fn render_uninstall_outcome(outcome: &UninstallOutcome, output_level: OutputLevel) -> String {
    match outcome {
        UninstallOutcome::Uninstalled { app, version } => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Info,
                &format!("'{app}' ({version}) was uninstalled.{EOL}"),
            );
            output
        }
        UninstallOutcome::NotInstalled { app, .. } => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Warn,
                &format!("'{app}' isn't installed.{EOL}"),
            );
            output
        }
        UninstallOutcome::RunningProcess { app, processes } => {
            let mut output = format!(
                "ERROR The following instances of '{app}' are still running. Close them and try again.{EOL}"
            );
            for process in processes {
                emit(&mut output, output_level, OutputLevel::Error, process);
                output.push_str(EOL);
            }
            emit(
                &mut output,
                output_level,
                OutputLevel::Error,
                "Running process detected, skip uninstalling.\r\n",
            );
            output
        }
    }
}

fn run_update(args: &[String]) -> anyhow::Result<Response> {
    let mut options = UpdateOptions::default();
    let mut output_level = OutputLevel::Info;
    let mut apps = Vec::<String>::new();
    for arg in args {
        match arg.as_str() {
            "-f" | "--force" => options.force = true,
            "-g" | "--global" => options.global = true,
            "-i" | "--independent" => options.independent = true,
            "-k" | "--no-cache" => options.use_cache = false,
            "-s" | "--skip-hash-check" => options.check_hash = false,
            "-q" | "--quiet" => {
                options.quiet = true;
                output_level = OutputLevel::Warn;
            }
            "-v" | "--verbose" => output_level = OutputLevel::Verbose,
            "-a" | "--all" | "*" => options.all = true,
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 1,
                    output: format!("ERROR scoop update: Option {value} not recognized.{EOL}"),
                });
            }
            value => apps.push(value.to_owned()),
        }
    }

    let config = scoop_core::RuntimeConfig::detect(None);
    if apps.is_empty() && !options.all {
        // No args: sync Scoop/buckets
        let outcome = scoop_core::app::update::update_scoop(&config)?;
        return Ok(Response {
            code: 0,
            output: render_update_outcome(&outcome, output_level),
        });
    }

    let mut output = String::new();
    for outcome in scoop_core::app::update::update_apps(&config, &apps, &options)? {
        output.push_str(&render_update_outcome(&outcome, output_level));
    }
    Ok(Response { code: 0, output })
}

fn render_update_outcome(outcome: &UpdateOutcome, output_level: OutputLevel) -> String {
    match outcome {
        UpdateOutcome::ScoopUpdateHeld { hold_until } => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Warn,
                &format!(
                    "WARN  Skipping self-update of Scoop Core until {hold_until}.{EOL}\
WARN  If you want to update Scoop Core immediately, use 'scoop unhold scoop; scoop update'.{EOL}"
                ),
            );
            output
        }
        UpdateOutcome::ScoopUpdated { changelog } => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Info,
                &format!("Scoop was updated successfully!{EOL}"),
            );
            emit(
                &mut output,
                output_level,
                OutputLevel::Info,
                &format_changelog(changelog),
            );
            output
        }
        UpdateOutcome::AppUpdated {
            app,
            old_version,
            new_version,
            changelog,
        } => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Info,
                &format!("'{app}' was updated from {old_version} to {new_version}.{EOL}"),
            );
            emit(
                &mut output,
                output_level,
                OutputLevel::Info,
                &format_changelog(changelog),
            );
            output
        }
        UpdateOutcome::AlreadyLatest { app, version } => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Info,
                &format!("'{app}' ({version}) is already up to date.{EOL}"),
            );
            output
        }
        UpdateOutcome::NoManifest { app } => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Warn,
                &format!("Couldn't find manifest for '{app}'.{EOL}"),
            );
            output
        }
        UpdateOutcome::Held { app, version } => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Warn,
                &format!("'{app}' ({version}) is held. Use --force to update anyway.{EOL}"),
            );
            output
        }
        UpdateOutcome::RunningProcess { app, processes } => {
            let mut output = format!(
                "ERROR The following instances of '{app}' are still running. Close them and try again.{EOL}"
            );
            for process in processes {
                emit(
                    &mut output,
                    output_level,
                    OutputLevel::Error,
                    &format!("{process}{EOL}"),
                );
            }
            emit(
                &mut output,
                output_level,
                OutputLevel::Error,
                "Running process detected, skip updating.\r\n",
            );
            output
        }
    }
}

fn format_changelog(changelog: &Option<String>) -> String {
    changelog
        .as_deref()
        .map(|changelog| format!("CHANGELOG: {changelog}{EOL}"))
        .unwrap_or_default()
}

fn run_reset(args: &[String]) -> anyhow::Result<Response> {
    let mut apps = Vec::<String>::new();
    let mut all = false;
    let mut output_level = OutputLevel::Info;
    for arg in args {
        match arg.as_str() {
            "-a" | "--all" | "*" => all = true,
            "-q" | "--quiet" => output_level = OutputLevel::Warn,
            "-v" | "--verbose" => output_level = OutputLevel::Verbose,
            value if value.starts_with('-') => {
                return Ok(Response {
                    code: 1,
                    output: format!("ERROR scoop reset: Option {value} not recognized.{EOL}"),
                });
            }
            value => apps.push(value.to_owned()),
        }
    }

    if apps.is_empty() && !all {
        return Ok(Response {
            code: 1,
            output: format!("ERROR <app> missing{EOL}Usage: scoop reset <app> [options]{EOL}"),
        });
    }

    let config = scoop_core::RuntimeConfig::detect(None);
    if all {
        let apps_dir = config.paths().apps();
        if apps_dir.exists() {
            for entry in std::fs::read_dir(&apps_dir)?.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name != "scoop" && entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                    apps.push(name);
                }
            }
        }
        apps.sort();
    }
    let mut output = String::new();
    for outcome in scoop_core::app::reset::reset_apps(&config, &apps)? {
        output.push_str(&render_reset_outcome(&outcome, output_level));
    }
    Ok(Response { code: 0, output })
}

fn render_reset_outcome(outcome: &ResetOutcome, output_level: OutputLevel) -> String {
    let mut output = String::new();
    emit(
        &mut output,
        output_level,
        OutputLevel::Info,
        &format!("'{}' ({}) was reset.{EOL}", outcome.app, outcome.version),
    );
    output
}

fn render_install_outcome(
    outcome: &InstallOutcome,
    options: &InstallOptions,
    output_level: OutputLevel,
) -> String {
    match outcome {
        InstallOutcome::Installed(installed) => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Info,
                &format!(
                    "Installing '{}' ({}) [{}]{}{}",
                    installed.app,
                    installed.version,
                    installed.architecture,
                    installed
                        .bucket
                        .as_deref()
                        .map(|bucket| format!(" from '{bucket}' bucket"))
                        .unwrap_or_default(),
                    EOL,
                ),
            );
            for shim in &installed.shim_names {
                emit(
                    &mut output,
                    output_level,
                    OutputLevel::Verbose,
                    &format!("Creating shim for '{shim}'.{EOL}"),
                );
            }
            emit(
                &mut output,
                output_level,
                OutputLevel::Info,
                &format!(
                    "'{}' ({}) was installed successfully!{EOL}",
                    installed.app, installed.version
                ),
            );
            if !installed.notes.is_empty() {
                emit(
                    &mut output,
                    output_level,
                    OutputLevel::Info,
                    "Notes\r\n-----\r\n",
                );
                for note in &installed.notes {
                    emit(
                        &mut output,
                        output_level,
                        OutputLevel::Info,
                        &format!("{note}{EOL}"),
                    );
                }
                emit(&mut output, output_level, OutputLevel::Info, "-----\r\n");
            }
            output
        }
        InstallOutcome::AlreadyInstalled { app, version } => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Warn,
                &format!(
                    "WARN  '{app}' ({version}) is already installed.\nUse 'scoop update {app}{}' to install a new version.{EOL}",
                    if options.global { " --global" } else { "" }
                ),
            );
            output
        }
        InstallOutcome::MissingManifest { app, bucket } => {
            let mut output = String::new();
            emit(
                &mut output,
                output_level,
                OutputLevel::Warn,
                &format!(
                    "Couldn't find manifest for '{app}'{}.{EOL}",
                    bucket
                        .as_deref()
                        .map(|bucket| format!(" from '{bucket}' bucket"))
                        .unwrap_or_default()
                ),
            );
            output
        }
    }
}

fn render_install_suggestions(
    config: &scoop_core::RuntimeConfig,
    outcomes: &[InstallOutcome],
    output_level: OutputLevel,
) -> String {
    let installed = installed_app_names(config);
    let mut suggestions = std::collections::BTreeMap::<String, Vec<String>>::new();

    for outcome in outcomes {
        let InstallOutcome::Installed(installed_outcome) = outcome else {
            continue;
        };
        let mut missing = installed_outcome
            .suggestions
            .iter()
            .filter(|suggestion| {
                !installed
                    .iter()
                    .any(|installed_app| installed_app.eq_ignore_ascii_case(suggestion))
            })
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            continue;
        }
        missing.sort();
        missing.dedup();
        suggestions.insert(installed_outcome.app.clone(), missing);
    }

    let mut output = String::new();
    for (app, missing) in suggestions {
        for suggestion in missing {
            emit(
                &mut output,
                output_level,
                OutputLevel::Info,
                &format!("'{app}' suggests installing '{suggestion}'.{EOL}"),
            );
        }
    }
    output
}

fn installed_app_names(config: &scoop_core::RuntimeConfig) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(report) = list_installed(config, None) {
        names.extend(report.apps.into_iter().map(|app| app.name));
    }
    names
}

fn run_search(args: &[String]) -> anyhow::Result<Response> {
    let query = args.first().map(String::as_str);
    let config = scoop_core::RuntimeConfig::detect(None);
    let mode = if config.settings().use_sqlite_cache.unwrap_or(false) {
        SearchMode::SqliteCache
    } else {
        SearchMode::Regex
    };
    let pattern = match (&mode, query) {
        (SearchMode::Regex, Some(query)) => match compile_search_query(query) {
            Ok(pattern) => Some(pattern),
            Err(_) => {
                return Ok(Response {
                    code: 0,
                    output: format!("Invalid regular expression: invalid pattern '{query}'.{EOL}"),
                });
            }
        },
        _ => None,
    };
    let results = match mode {
        SearchMode::Regex => search_local_buckets(&config, pattern.as_ref())?,
        SearchMode::SqliteCache => search_cached_buckets(&config, query)?,
    };

    if !results.is_empty() {
        let mut output = String::from("Results from local buckets...\r\n\r\n");
        output.push_str(&render_search_table(&results));
        return Ok(Response { code: 0, output });
    }

    match mode {
        SearchMode::Regex => {
            if let Some(pattern) = pattern.as_ref() {
                let remote_results =
                    search_remote_buckets(&config, &build_blocking_http_client()?, pattern)?;
                if !remote_results.is_empty() {
                    let mut output = String::from(
                        "Results from other known buckets...\r\n(add them using 'scoop bucket add <bucket name>')\r\n\r\n",
                    );
                    output.push_str(&render_remote_search_table(&remote_results));
                    return Ok(Response { code: 0, output });
                }
            }
        }
        SearchMode::SqliteCache => {
            let remote_results = search_remote_buckets_partial(
                &config,
                &build_blocking_http_client()?,
                query.unwrap_or_default(),
            )?;
            if !remote_results.is_empty() {
                let mut output = String::from(
                    "Results from other known buckets...\r\n(add them using 'scoop bucket add <bucket name>')\r\n\r\n",
                );
                output.push_str(&render_remote_search_table(&remote_results));
                return Ok(Response { code: 0, output });
            }
        }
    }

    Ok(Response {
        code: 1,
        output: format!("WARN  No matches found.{EOL}"),
    })
}

fn run_status(args: &[String]) -> anyhow::Result<Response> {
    let local_only = args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-l" | "--local"));
    let report = collect_status(&scoop_core::RuntimeConfig::detect(None), local_only)?;

    let mut output = String::new();
    if report.scoop_out_of_date {
        output
            .push_str("WARN  Scoop out of date. Run 'scoop update' to get the latest changes.\r\n");
    } else if report.bucket_out_of_date {
        output.push_str(
            "WARN  Scoop bucket(s) out of date. Run 'scoop update' to get the latest changes.\r\n",
        );
    } else if report.network_failure {
        output.push_str("WARN  Could not check for Scoop updates due to network failures.\r\n");
    } else if !report.network_failure && !local_only {
        output.push_str("Scoop is up to date.\r\n");
    }

    if report.rows.is_empty() {
        if !report.scoop_out_of_date && !report.bucket_out_of_date && !report.network_failure {
            output.push_str("Everything is ok!\r\n");
        }
        return Ok(Response { code: 0, output });
    }

    output.push_str(EOL);
    output.push_str(&render_status_table(&report.rows));
    Ok(Response { code: 0, output })
}

fn run_prefix(args: &[String]) -> anyhow::Result<Response> {
    let Some(app) = args.first() else {
        return Ok(Response {
            code: 0,
            output: String::from("Usage: scoop prefix <app>\r\n"),
        });
    };

    let config = scoop_core::RuntimeConfig::detect(None);
    match resolve_prefix(&config, app)? {
        Some(path) => Ok(Response {
            code: 0,
            output: format!("{}{EOL}", path),
        }),
        None => Ok(Response {
            code: 0,
            output: format!("Could not find app path for '{app}'.{EOL}"),
        }),
    }
}

fn run_which(args: &[String]) -> anyhow::Result<Response> {
    let Some(command) = args.first() else {
        return Ok(Response {
            code: 0,
            output: String::from("ERROR <command> missing\r\nUsage: scoop which <command>\r\n"),
        });
    };

    let config = scoop_core::RuntimeConfig::detect(None);
    match resolve_which(&config, command)? {
        Some(path) => Ok(Response {
            code: 0,
            output: format!("{}{EOL}", path),
        }),
        None => Ok(Response {
            code: 0,
            output: format!(
                "WARN  '{command}' not found, not a scoop shim, or a broken shim.{EOL}"
            ),
        }),
    }
}

fn render_list_table(apps: &[InstalledApp]) -> anyhow::Result<String> {
    let rows = apps
        .iter()
        .map(|app| {
            Ok(ListRow {
                name: app.name.clone(),
                version: app.version.clone(),
                source: app.source.clone().unwrap_or_default(),
                updated: format_updated_time(app.updated)?,
                info: app.info.join(", "),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut widths = (
        "Name".len(),
        "Version".len(),
        "Source".len(),
        "Updated".len(),
        "Info".len(),
    );
    for row in &rows {
        widths.0 = widths.0.max(row.name.len());
        widths.1 = widths.1.max(row.version.len());
        widths.2 = widths.2.max(row.source.len());
        widths.3 = widths.3.max(row.updated.len());
        widths.4 = widths.4.max(row.info.len());
    }

    let mut output = String::new();
    output.push_str(&render_table_row(
        ("Name", "Version", "Source", "Updated", "Info"),
        widths,
    ));
    output.push_str(EOL);
    output.push_str(&render_table_row(
        ("----", "-------", "------", "-------", "----"),
        widths,
    ));
    output.push_str(EOL);
    for row in rows {
        output.push_str(&render_table_row(
            (
                &row.name,
                &row.version,
                &row.source,
                &row.updated,
                &row.info,
            ),
            widths,
        ));
        output.push_str(EOL);
    }
    Ok(output)
}

fn render_table_row(
    columns: (&str, &str, &str, &str, &str),
    widths: (usize, usize, usize, usize, usize),
) -> String {
    format!(
        "{:<name_width$} {:<version_width$} {:<source_width$} {:<updated_width$} {}",
        columns.0,
        columns.1,
        columns.2,
        columns.3,
        columns.4,
        name_width = widths.0,
        version_width = widths.1,
        source_width = widths.2,
        updated_width = widths.3,
    )
}

fn render_search_table(results: &[SearchResult]) -> String {
    let mut widths = (
        "Name".len(),
        "Version".len(),
        "Source".len(),
        "Binaries".len(),
    );
    for row in results {
        widths.0 = widths.0.max(row.name.len());
        widths.1 = widths.1.max(row.version.len());
        widths.2 = widths.2.max(row.source.len());
        widths.3 = widths.3.max(row.binaries.join(" | ").len());
    }

    let mut output = String::new();
    output.push_str(&format!(
        "{:<name_width$} {:<version_width$} {:<source_width$} {}\r\n",
        "Name",
        "Version",
        "Source",
        "Binaries",
        name_width = widths.0,
        version_width = widths.1,
        source_width = widths.2,
    ));
    output.push_str(&format!(
        "{:<name_width$} {:<version_width$} {:<source_width$} {}\r\n",
        "----",
        "-------",
        "------",
        "--------",
        name_width = widths.0,
        version_width = widths.1,
        source_width = widths.2,
    ));
    for row in results {
        output.push_str(&format!(
            "{:<name_width$} {:<version_width$} {:<source_width$} {}\r\n",
            row.name,
            row.version,
            row.source,
            row.binaries.join(" | "),
            name_width = widths.0,
            version_width = widths.1,
            source_width = widths.2,
        ));
    }
    output
}

fn render_remote_search_table(results: &[RemoteSearchResult]) -> String {
    let mut widths = ("Name".len(), "Source".len());
    for row in results {
        widths.0 = widths.0.max(row.name.len());
        widths.1 = widths.1.max(row.source.len());
    }

    let mut output = String::new();
    output.push_str(&format!(
        "{:<name_width$} {}\r\n",
        "Name",
        "Source",
        name_width = widths.0,
    ));
    output.push_str(&format!(
        "{:<name_width$} {}\r\n",
        "----",
        "------",
        name_width = widths.0,
    ));
    for row in results {
        output.push_str(&format!(
            "{:<name_width$} {}\r\n",
            row.name,
            row.source,
            name_width = widths.0,
        ));
    }
    output
}

fn render_status_table(rows: &[StatusRow]) -> String {
    let mut widths = (
        "Name".len(),
        "Installed Version".len(),
        "Latest Version".len(),
        "Missing Dependencies".len(),
        "Info".len(),
    );
    for row in rows {
        widths.0 = widths.0.max(row.name.len());
        widths.1 = widths.1.max(row.installed_version.len());
        widths.2 = widths.2.max(row.latest_version.len());
        widths.3 = widths.3.max(row.missing_dependencies.join(" | ").len());
        widths.4 = widths.4.max(row.info.join(", ").len());
    }

    let mut output = String::new();
    output.push_str(&format!(
        "{:<name_width$} {:<installed_width$} {:<latest_width$} {:<missing_width$} {}\r\n",
        "Name",
        "Installed Version",
        "Latest Version",
        "Missing Dependencies",
        "Info",
        name_width = widths.0,
        installed_width = widths.1,
        latest_width = widths.2,
        missing_width = widths.3,
    ));
    output.push_str(&format!(
        "{:<name_width$} {:<installed_width$} {:<latest_width$} {:<missing_width$} {}\r\n",
        "----",
        "-----------------",
        "--------------",
        "--------------------",
        "----",
        name_width = widths.0,
        installed_width = widths.1,
        latest_width = widths.2,
        missing_width = widths.3,
    ));
    for row in rows {
        output.push_str(&format!(
            "{:<name_width$} {:<installed_width$} {:<latest_width$} {:<missing_width$} {}\r\n",
            row.name,
            row.installed_version,
            row.latest_version,
            row.missing_dependencies.join(" | "),
            row.info.join(", "),
            name_width = widths.0,
            installed_width = widths.1,
            latest_width = widths.2,
            missing_width = widths.3,
        ));
    }
    output
}

fn render_cache_show(entries: &[CacheEntry], total_length: u64) -> String {
    let mut output = format!(
        "Total: {} {}, {}{EOL}",
        entries.len(),
        pluralize(entries.len(), "file", "files"),
        format_file_size(total_length),
    );
    if !entries.is_empty() {
        let mut widths = ("Name".len(), "Version".len(), "Length".len());
        for entry in entries {
            widths.0 = widths.0.max(entry.name.len());
            widths.1 = widths.1.max(entry.version.len());
            widths.2 = widths.2.max(entry.length.to_string().len());
        }
        output.push_str(EOL);
        output.push_str(&format!(
            "{:<name_width$} {:<version_width$} {:>length_width$}{EOL}",
            "Name",
            "Version",
            "Length",
            name_width = widths.0,
            version_width = widths.1,
            length_width = widths.2,
        ));
        output.push_str(&format!(
            "{:<name_width$} {:<version_width$} {:>length_width$}{EOL}",
            "----",
            "-------",
            "------",
            name_width = widths.0,
            version_width = widths.1,
            length_width = widths.2,
        ));
        for entry in entries {
            output.push_str(&format!(
                "{:<name_width$} {:<version_width$} {:>length_width$}{EOL}",
                entry.name,
                entry.version,
                entry.length,
                name_width = widths.0,
                version_width = widths.1,
                length_width = widths.2,
            ));
        }
    }
    output
}

fn render_bucket_known(names: &[String]) -> String {
    let mut output = String::new();
    for name in names {
        output.push_str(name);
        output.push_str(EOL);
    }
    output
}

fn render_cleanup_outcomes(outcomes: &[CleanupOutcome], all: bool) -> String {
    let mut output = String::new();
    for outcome in outcomes {
        match outcome {
            CleanupOutcome::Cleaned {
                app,
                removed_versions,
                ..
            } => {
                output.push_str(&format!("Removing {app}:"));
                for version in removed_versions {
                    output.push(' ');
                    output.push_str(version);
                }
                output.push_str(EOL);
            }
            CleanupOutcome::AlreadyClean { app } if !all => {
                output.push_str(&format!("{app} is already clean{EOL}"));
            }
            CleanupOutcome::NotInstalled { app } if !all => {
                output.push_str(&format!("ERROR '{app}' isn't installed.{EOL}"));
            }
            _ => {}
        }
    }
    if all {
        output.push_str(&format!("Everything is shiny now!{EOL}"));
    }
    output
}

fn render_dependency_table(rows: &[DependencyRow]) -> String {
    let mut widths = ("Source".len(), "Name".len());
    for row in rows {
        widths.0 = widths.0.max(row.source.len());
        widths.1 = widths.1.max(row.name.len());
    }

    let mut output = String::from(EOL);
    output.push_str(&format!(
        "{:<source_width$} {}\r\n",
        "Source",
        "Name",
        source_width = widths.0,
    ));
    output.push_str(&format!(
        "{:<source_width$} {}\r\n",
        "------",
        "----",
        source_width = widths.0,
    ));
    for row in rows {
        output.push_str(&format!(
            "{:<source_width$} {}\r\n",
            row.source,
            row.name,
            source_width = widths.0,
        ));
    }
    output
}

fn render_download_report(report: &DownloadReport) -> String {
    let mut output = format!(
        "INFO  Downloading '{}'{} [{}]{}{}",
        report.app,
        report
            .requested_version
            .as_deref()
            .map(|version| format!(" ({version})"))
            .unwrap_or_default(),
        report.architecture,
        report
            .bucket
            .as_deref()
            .map(|bucket| format!(" from {bucket} bucket"))
            .unwrap_or_default(),
        EOL
    );

    match &report.outcome {
        DownloadOutcome::Downloaded {
            version,
            files,
            skipped_hash_verification,
        } => {
            for file in files {
                if file.loaded_from_cache {
                    output.push_str(&format!("Loading {} from cache.{EOL}", file.file_name));
                }
                if file.verified_hash {
                    output.push_str(&format!("Checking hash of {}... OK.{EOL}", file.file_name));
                }
            }
            if *skipped_hash_verification {
                output.push_str("INFO  Skipping hash verification.\r\n");
            }
            output.push_str(&format!(
                "'{}' ({version}) was downloaded successfully!{EOL}",
                report.app
            ));
        }
        DownloadOutcome::Failed { message } => {
            output.push_str(&format!("ERROR {message}{EOL}"));
        }
    }

    output
}

fn render_shim_list(rows: &[ShimInfo]) -> anyhow::Result<String> {
    if rows.is_empty() {
        return Ok(String::new());
    }
    let mut widths = (
        "Name".len(),
        "Source".len(),
        "Alternatives".len(),
        "IsGlobal".len(),
        "IsHidden".len(),
    );
    for row in rows {
        widths.0 = widths.0.max(row.name.len());
        widths.1 = widths.1.max(row.source.len());
        widths.2 = widths.2.max(row.alternatives.join(" ").len());
        widths.3 = widths.3.max(row.is_global.to_string().len());
        widths.4 = widths.4.max(row.is_hidden.to_string().len());
    }

    let mut output = String::new();
    output.push_str(&format!(
        "{:<name_width$} {:<source_width$} {:<alternatives_width$} {:<global_width$} {:<hidden_width$}{EOL}",
        "Name",
        "Source",
        "Alternatives",
        "IsGlobal",
        "IsHidden",
        name_width = widths.0,
        source_width = widths.1,
        alternatives_width = widths.2,
        global_width = widths.3,
        hidden_width = widths.4,
    ));
    output.push_str(&format!(
        "{:<name_width$} {:<source_width$} {:<alternatives_width$} {:<global_width$} {:<hidden_width$}{EOL}",
        "----",
        "------",
        "------------",
        "--------",
        "--------",
        name_width = widths.0,
        source_width = widths.1,
        alternatives_width = widths.2,
        global_width = widths.3,
        hidden_width = widths.4,
    ));
    for row in rows {
        output.push_str(&format!(
            "{:<name_width$} {:<source_width$} {:<alternatives_width$} {:<global_width$} {:<hidden_width$}{EOL}",
            row.name,
            row.source,
            row.alternatives.join(" "),
            row.is_global,
            row.is_hidden,
            name_width = widths.0,
            source_width = widths.1,
            alternatives_width = widths.2,
            global_width = widths.3,
            hidden_width = widths.4,
        ));
    }
    Ok(output)
}

fn render_shim_info(info: &ShimInfo) -> String {
    let alternatives = if info.alternatives.is_empty() {
        String::new()
    } else {
        info.alternatives.join(" ")
    };
    [
        format!("Name         : {}", info.name),
        format!("Path         : {}", info.path),
        format!("Source       : {}", info.source),
        format!("Type         : {}", info.kind),
        format!("Alternatives : {alternatives}"),
        format!("IsGlobal     : {}", info.is_global),
        format!("IsHidden     : {}", info.is_hidden),
        String::new(),
    ]
    .join(EOL)
}

fn render_bucket_list(rows: &[BucketListEntry]) -> anyhow::Result<String> {
    let rendered_rows = rows
        .iter()
        .map(|row| {
            Ok((
                row.name.clone(),
                row.source.clone(),
                format_bucket_updated(row.updated)?,
                row.manifests.to_string(),
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut widths = (
        "Name".len(),
        "Source".len(),
        "Updated".len(),
        "Manifests".len(),
    );
    for row in &rendered_rows {
        widths.0 = widths.0.max(row.0.len());
        widths.1 = widths.1.max(row.1.len());
        widths.2 = widths.2.max(row.2.len());
        widths.3 = widths.3.max(row.3.len());
    }

    let mut output = String::new();
    output.push_str(&format!(
        "{:<name_width$} {:<source_width$} {:<updated_width$} {:>manifest_width$}{EOL}",
        "Name",
        "Source",
        "Updated",
        "Manifests",
        name_width = widths.0,
        source_width = widths.1,
        updated_width = widths.2,
        manifest_width = widths.3,
    ));
    output.push_str(&format!(
        "{:<name_width$} {:<source_width$} {:<updated_width$} {:>manifest_width$}{EOL}",
        "----",
        "------",
        "-------",
        "---------",
        name_width = widths.0,
        source_width = widths.1,
        updated_width = widths.2,
        manifest_width = widths.3,
    ));
    for row in rendered_rows {
        output.push_str(&format!(
            "{:<name_width$} {:<source_width$} {:<updated_width$} {:>manifest_width$}{EOL}",
            row.0,
            row.1,
            row.2,
            row.3,
            name_width = widths.0,
            source_width = widths.1,
            updated_width = widths.2,
            manifest_width = widths.3,
        ));
    }
    Ok(output)
}

fn render_cache_remove(entries: &[CacheEntry], total_length: u64) -> String {
    let mut output = String::new();
    for entry in entries {
        output.push_str(&format!("Removing {}...{EOL}", entry.file_name));
    }
    output.push_str(&format!(
        "Deleted: {} {}, {}{EOL}",
        entries.len(),
        pluralize(entries.len(), "file", "files"),
        format_file_size(total_length),
    ));
    output
}

fn render_config_map(entries: &Map<String, Value>) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let columns = entries
        .iter()
        .map(|(name, value)| (name.clone(), render_config_value_inline(value)))
        .collect::<Vec<_>>();
    let widths = columns
        .iter()
        .map(|(name, value)| name.len().max(value.len()))
        .collect::<Vec<_>>();

    let mut output = String::new();
    for ((name, _), width) in columns.iter().zip(&widths) {
        output.push_str(&format!("{:<width$} ", name, width = width));
    }
    output.push_str(EOL);
    for ((_, _), width) in columns.iter().zip(&widths) {
        output.push_str(&format!("{:<width$} ", "-".repeat(*width), width = width));
    }
    output.push_str(EOL);
    for ((_, value), width) in columns.iter().zip(&widths) {
        output.push_str(&format!("{:<width$} ", value, width = width));
    }
    output.push_str(EOL);
    output
}

fn render_config_value(value: &Value) -> String {
    match value {
        Value::Bool(true) => String::from("True"),
        Value::Bool(false) => String::from("False"),
        Value::String(value) => value.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

fn render_config_value_inline(value: &Value) -> String {
    render_config_value(value)
}

fn render_hold_outcomes(outcomes: &[HoldOutcome], holding: bool) -> String {
    let mut output = String::new();
    for outcome in outcomes {
        match outcome {
            HoldOutcome::ScoopHeld { hold_until } => {
                output.push_str(&format!(
                    "scoop is now held and might not be updated until {}.{EOL}",
                    format_hold_until(hold_until)
                ));
            }
            HoldOutcome::ScoopUnheld => {
                output.push_str(&format!(
                    "scoop is no longer held and can be updated again.{EOL}"
                ));
            }
            HoldOutcome::Held { app } => {
                output.push_str(&format!(
                    "{app} is now held and can not be updated anymore.{EOL}"
                ));
            }
            HoldOutcome::AlreadyHeld { app } => {
                output.push_str(&format!("INFO  '{app}' is already held.{EOL}"));
            }
            HoldOutcome::Unheld { app } => {
                output.push_str(&format!(
                    "{app} is no longer held and can be updated again.{EOL}"
                ));
            }
            HoldOutcome::NotHeld { app } => {
                output.push_str(&format!("INFO  '{app}' is not held.{EOL}"));
            }
            HoldOutcome::NotInstalled { app, global } => {
                output.push_str(&format!(
                    "ERROR '{app}' is not installed{}.{EOL}",
                    if *global { " globally" } else { "" }
                ));
            }
        }
    }
    if holding || !outcomes.is_empty() {
        return output;
    }
    String::new()
}

fn format_hold_until(value: &str) -> String {
    value
        .parse::<jiff::Timestamp>()
        .map(|timestamp| {
            timestamp
                .to_zoned(jiff::tz::TimeZone::system())
                .strftime("%m/%d/%Y %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_owned())
}

fn format_bucket_updated(updated: std::time::SystemTime) -> anyhow::Result<String> {
    let timestamp = jiff::Timestamp::try_from(updated).context("failed to convert system time")?;
    Ok(timestamp
        .to_zoned(jiff::tz::TimeZone::system())
        .strftime("%-d/%m/%Y %-I:%M:%S %p")
        .to_string())
}

fn format_file_size(length: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    if (length as f64) > GB {
        format!("{:.1} GB", (length as f64) / GB)
    } else if (length as f64) > MB {
        format!("{:.1} MB", (length as f64) / MB)
    } else if (length as f64) > KB {
        format!("{:.1} KB", (length as f64) / KB)
    } else {
        format!("{length} B")
    }
}

fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

struct ListRow {
    name: String,
    version: String,
    source: String,
    updated: String,
    info: String,
}

fn render_with_bat(rendered: &str, style: &str) -> String {
    let Some(mut child) = spawn_bat(style) else {
        return rendered.to_owned();
    };
    if let Some(stdin) = child.stdin.as_mut()
        && stdin.write_all(rendered.as_bytes()).is_err()
    {
        return rendered.to_owned();
    }

    match child.wait_with_output() {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned(),
        _ => rendered.to_owned(),
    }
}

fn spawn_bat(style: &str) -> Option<std::process::Child> {
    for program in ["bat", "bat.exe", "bat.cmd", "bat.bat"] {
        let child = Command::new(program)
            .args(["--no-paging", "--style", style, "--language", "json"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(child) = child {
            return Some(child);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::run;

    fn response(args: &[&str]) -> (i32, String) {
        let owned = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
        let response = run(&owned);
        (response.code, response.output)
    }

    #[test]
    fn default_invocation_routes_to_help() {
        let (code, output) = response(&[]);
        assert_eq!(code, 0);
        assert!(output.starts_with("Usage: scoop <command> [<args>]"));
        assert!(output.contains("help        Show help for a command"));
    }

    #[test]
    fn help_for_known_command_returns_usage() {
        let (code, output) = response(&["help", "help"]);
        assert_eq!(code, 0);
        assert_eq!(output, "Usage: scoop help <command>\r\n\r\n");
    }

    #[test]
    fn help_for_unknown_command_returns_error() {
        let (code, output) = response(&["help", "missing"]);
        assert_eq!(code, 0);
        assert_eq!(output, "ERROR scoop help: no such command 'missing'\r\n");
    }

    #[test]
    fn unknown_command_matches_scoop_warning_shape() {
        let (code, output) = response(&["missing"]);
        assert_eq!(code, 1);
        assert_eq!(
            output,
            "WARN  scoop: 'missing' isn't a scoop command. See 'scoop help'.\r\n"
        );
    }
}
