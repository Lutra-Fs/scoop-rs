pub struct CommandSpec {
    pub name: &'static str,
    pub summary: &'static str,
    pub usage: &'static str,
    pub help: Option<&'static str>,
    pub implemented: bool,
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "alias",
        summary: "Manage scoop aliases",
        usage: "Usage: scoop alias <subcommand> [<args>]",
        help: None,
        implemented: false,
    },
    CommandSpec {
        name: "bucket",
        summary: "Manage Scoop buckets",
        usage: "Usage: scoop bucket <subcommand> [<args>]",
        help: Some(
            "Add, list, or remove buckets.\n\nUsage:\n  scoop bucket add <name> [<repo>]\n  scoop bucket list\n  scoop bucket known\n  scoop bucket rm <name>",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "cache",
        summary: "Show or clear the download cache",
        usage: "Usage: scoop cache <subcommand> [<args>]",
        help: Some(
            "Show or clear cached downloads.\n\nUsage:\n  scoop cache show [<app>...]\n  scoop cache rm <app>...\n  scoop cache rm *",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "cat",
        summary: "Show content of specified manifest.",
        usage: "Usage: scoop cat <app>",
        help: Some(
            "Show content of specified manifest.\nIf configured, `bat` will be used to pretty-print the JSON.",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "checkup",
        summary: "Check for potential problems",
        usage: "Usage: scoop checkup",
        help: None,
        implemented: false,
    },
    CommandSpec {
        name: "cleanup",
        summary: "Cleanup apps by removing old versions",
        usage: "Usage: scoop cleanup [<app>] [<args>]",
        help: Some(
            "Remove old app versions.\n\nOptions:\n  -a, --all     Cleanup all apps\n  -g, --global  Cleanup a globally installed app\n  -k, --cache   Remove outdated download cache",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "config",
        summary: "Get or set configuration values",
        usage: "Usage: scoop config <name> [value]",
        help: Some(
            "Get, set, or remove Scoop configuration values.\n\nUsage:\n  scoop config\n  scoop config <name>\n  scoop config <name> <value>\n  scoop config rm <name>",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "create",
        summary: "Create a custom app manifest",
        usage: "Usage: scoop create <url>",
        help: None,
        implemented: false,
    },
    CommandSpec {
        name: "depends",
        summary: "List dependencies for an app, in the order they'll be installed",
        usage: "Usage: scoop depends <app>",
        help: None,
        implemented: true,
    },
    CommandSpec {
        name: "download",
        summary: "Download apps in the cache folder and verify hashes",
        usage: "Usage: scoop download <app> [<args>]",
        help: None,
        implemented: true,
    },
    CommandSpec {
        name: "export",
        summary: "Exports installed apps, buckets (and optionally configs) in JSON format",
        usage: "Usage: scoop export [<path>] [<args>]",
        help: Some(
            "Options:\n  -c, --config  Export the active Scoop config alongside buckets and apps",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "help",
        summary: "Show help for a command",
        usage: "Usage: scoop help <command>",
        help: None,
        implemented: true,
    },
    CommandSpec {
        name: "hold",
        summary: "Hold an app to disable updates",
        usage: "Usage: scoop hold <app> [<args>]",
        help: Some("Options:\n  -g, --global  Hold globally installed apps"),
        implemented: true,
    },
    CommandSpec {
        name: "home",
        summary: "Opens the app homepage",
        usage: "Usage: scoop home <app>",
        help: None,
        implemented: false,
    },
    CommandSpec {
        name: "import",
        summary: "Imports apps, buckets and configs from a Scoopfile in JSON format",
        usage: "Usage: scoop import <path> [<args>]",
        help: Some(
            "Import a Scoopfile from a local JSON path or URL.\nImported configs are applied first, then buckets, then apps.",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "info",
        summary: "Display information about an app",
        usage: "Usage: scoop info <app> [options]",
        help: Some("Options:\n  -v, --verbose   Show full paths and URLs"),
        implemented: true,
    },
    CommandSpec {
        name: "install",
        summary: "Install apps",
        usage: "Usage: scoop install <app> [<args>]",
        help: Some(
            "Options:\n  -g, --global              Install the app globally\n  -i, --independent         Don't install dependencies automatically\n  -k, --no-cache            Don't use the download cache\n  -s, --skip-hash-check     Skip hash validation\n  -q, --quiet               Hide extraneous messages\n  -v, --verbose             Show verbose output\n  -u, --no-update-scoop     Don't update Scoop before installing if it's outdated\n  -a, --arch <arch>         Use the specified architecture",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "list",
        summary: "List installed apps",
        usage: "Usage: scoop list",
        help: Some("Lists all installed apps, or the apps matching the supplied query."),
        implemented: true,
    },
    CommandSpec {
        name: "prefix",
        summary: "Returns the path to the specified app",
        usage: "Usage: scoop prefix <app>",
        help: None,
        implemented: true,
    },
    CommandSpec {
        name: "reset",
        summary: "Reset an app to resolve conflicts",
        usage: "Usage: scoop reset <app> [<args>]",
        help: Some(
            "Re-creates shims, shortcuts, environment variables and persisted data for an app.\nUseful when another app has overwritten shims or when switching between app versions.\n\nOptions:\n  -a, --all       Reset all installed apps\n  -q, --quiet     Hide extraneous messages\n  -v, --verbose   Show verbose output",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "search",
        summary: "Search available apps",
        usage: "Usage: scoop search <app>",
        help: Some("Searches for apps that are available to install."),
        implemented: true,
    },
    CommandSpec {
        name: "shim",
        summary: "Manipulate Scoop shims",
        usage: "Usage: scoop shim <subcommand> [<args>]",
        help: Some(
            "Available subcommands: add, rm, list, info, alter.\n\nUsage:\n  scoop shim add <shim_name> <command_path> [-- <args>...]\n  scoop shim rm <shim_name> [<shim_name>...]\n  scoop shim list [<regex_pattern>...]\n  scoop shim info <shim_name>\n  scoop shim alter <shim_name>",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "status",
        summary: "Show status and check for new app versions",
        usage: "Usage: scoop status",
        help: Some(
            "Options:\n  -l, --local   Checks the status for only the locally installed apps.",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "unhold",
        summary: "Unhold an app to enable updates",
        usage: "Usage: scoop unhold <app>",
        help: Some("Options:\n  -g, --global  Unhold globally installed apps"),
        implemented: true,
    },
    CommandSpec {
        name: "uninstall",
        summary: "Uninstall an app",
        usage: "Usage: scoop uninstall <app> [<args>]",
        help: Some(
            "Options:\n  -g, --global   Uninstall a globally installed app\n  -p, --purge    Remove all persistent data\n  -q, --quiet    Hide extraneous messages\n  -v, --verbose  Show verbose output",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "update",
        summary: "Update apps, or Scoop itself",
        usage: "Usage: scoop update [<app>] [<args>]",
        help: Some(
            "Options:\n  -f, --force           Force update even when update to date\n  -g, --global          Update a globally installed app\n  -i, --independent     Don't install dependencies automatically\n  -k, --no-cache        Don't use the download cache\n  -s, --skip-hash-check Skip hash validation\n  -q, --quiet           Hide extraneous messages\n  -v, --verbose         Show verbose output\n  -a, --all             Update all installed apps",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "virustotal",
        summary: "Look for app's hash or url on virustotal.com",
        usage: "Usage: scoop virustotal <app>",
        help: Some(
            "Usage:\n  scoop virustotal [* | app1 app2 ...] [options]\n\nOptions:\n  -a, --all             Check all installed apps\n  -n, --no-depends      Do not expand dependencies\n  -u, --no-update-scoop Don't self-update Scoop first\n  -p, --passthru        Emit raw JSON reports when available",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "which",
        summary: "Locate a shim/executable (similar to 'which' on Linux)",
        usage: "Usage: scoop which <command>",
        help: Some(
            "Locate the path to a shim/executable that was installed with Scoop (similar to 'which' on Linux)",
        ),
        implemented: true,
    },
    CommandSpec {
        name: "reinstall",
        summary: "Re-install software",
        usage: "Usage: scoop reinstall <app> [<args>]",
        help: None,
        implemented: true,
    },
];

pub fn find(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|command| command.name == name)
}
