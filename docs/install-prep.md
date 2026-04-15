# Install Prep

This note captures the upstream Scoop `install` behavior we need to preserve when implementing `scoop install` in Rust.

Scope:

- Keep this file focused on upstream `install` semantics, lifecycle shape, and important invariants.
- Do not use it as the canonical status page for current scoop-rs implementation progress.
- Current scoop-rs behavior differences belong in [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md).
- Future install backlog items belong in [`THINGS_TO_ADDRESS.md`](/E:/scoop-rs/THINGS_TO_ADDRESS.md).

Primary upstream references under `<upstream-scoop-root>/apps/scoop/current`, with `<upstream-scoop-root>` resolved by the upstream installer from the explicit installer path, then `$env:SCOOP`, then the default user install root `~/scoop` / `$env:USERPROFILE\scoop`:

- `<upstream-scoop-root>/apps/scoop/current/libexec/scoop-install.ps1`
- `<upstream-scoop-root>/apps/scoop/current/lib/install.ps1`
- `<upstream-scoop-root>/apps/scoop/current/lib/manifest.ps1`
- `<upstream-scoop-root>/apps/scoop/current/lib/download.ps1`
- `<upstream-scoop-root>/apps/scoop/current/lib/decompress.ps1`
- `<upstream-scoop-root>/apps/scoop/current/lib/depends.ps1`
- `<upstream-scoop-root>/apps/scoop/current/lib/shortcuts.ps1`
- `<upstream-scoop-root>/apps/scoop/current/lib/psmodules.ps1`

## CLI Contract

Supported input forms:

- `scoop install git`
- `scoop install bucket/app`
- `scoop install app@version`
- `scoop install https://.../manifest.json`
- `scoop install https://.../manifest.json@version`
- `scoop install C:\path\to\manifest.json`
- `scoop install \\server\share\manifest.json`

Supported options:

- `-g`, `--global`
- `-i`, `--independent`
- `-k`, `--no-cache`
- `-s`, `--skip-hash-check`
- `-u`, `--no-update-scoop`
- `-a`, `--arch <32bit|64bit|arm64>`

Observable gate checks:

- Global install aborts without admin rights.
- If Scoop core is outdated and `--no-update-scoop` is not passed, upstream runs `scoop update` before install.
- If one explicit app is already installed and no version override was requested, upstream warns and exits `0`.
- Failed previous installs are repaired or purged before new work starts.

## Resolution Behavior

Manifest resolution is broader than bucket lookup:

- Local buckets, including explicit `bucket/app`
- Already installed app metadata via `install.json`
- Direct URL manifest
- Direct local or UNC path manifest
- Versioned request via `generate_user_manifest`

Important upstream quirks:

- `Get-Manifest` prefers installed metadata when an app is already installed.
- If multiple buckets contain the same app, upstream chooses one and warns.
- Versioned installs rely on manifest autoupdate generation unless sqlite cache already has the exact historical manifest.
- `nightly` manifests are rewritten to `nightly-YYYYMMDD` and skip hash validation.

## Dependency Resolution

Upstream expands both:

- manifest `depends`
- implicit installer helpers from `Get-InstallationHelper`

Helpers are install-time dependencies such as:

- `7zip`
- `lessmsi`
- `innounp`
- `dark`

Dependency behavior that matters:

- `--independent` disables automatic dependency installation
- circular dependencies abort
- dependency output order is topological, then explicit app last
- duplicate inputs are removed after dependency expansion

## Install Pipeline

For each app, upstream effectively does:

1. Resolve manifest and architecture.
2. Create target version directory.
3. Download artifacts into cache or directly into the version directory.
4. Validate hashes unless disabled or nightly.
5. Extract archives or installers when applicable.
6. Run `pre_install`.
7. Run installer payload if `installer.file` / `installer.args` exist.
8. Remove installer-added PATH entries under the app dir.
9. Link `apps/<app>/current` unless `NO_JUNCTION`.
10. Create shims.
11. Create Start Menu shortcuts.
12. Install PowerShell module links.
13. Apply `env_add_path`.
14. Apply `env_set`.
15. Link or move `persist` data.
16. Apply global persist ACL adjustment when needed.
17. Run `post_install`.
18. Save `manifest.json` and `install.json`.
19. Print suggestions and notes.

## Filesystem And Environment Side Effects

Install is not just file extraction. It mutates:

- `apps/<app>/<version>`
- `apps/<app>/current`
- `cache`
- `persist/<app>`
- `shims`
- Start Menu shortcut directory
- `modules`
- user or machine registry environment variables
- current process environment variables

State files written by upstream:

- `apps/<app>/<version>/manifest.json`
- `apps/<app>/<version>/install.json`

`install.json` is important because later commands recover:

- selected architecture
- source bucket
- original manifest URL/path

## Failure And Recovery Semantics

Upstream currently has several recovery behaviors we should model explicitly:

- previous failed install is reset or uninstalled before retry
- hash failure removes cached file
- extraction failure points to a log file
- installer failure leaves a partially created app tree and suggests uninstall first
- broken `current` state falls back to installed version detection by timestamps

We should implement these as typed failure states, not ad hoc shell cleanup.

## Recommended Rust Implementation Phases

Phase 1: resolution and planning

- parse install targets and options
- resolve manifest source
- resolve version override and generated manifest path
- resolve dependency graph including helpers
- detect skip/install/repair plan before side effects

Phase 2: fetch and unpack

- cache path computation compatible with Scoop
- downloader with hash validation and `--no-cache`
- extraction engine with archive-type dispatch

Phase 3: activation

- installer hooks
- `current` linking
- shim creation
- shortcut creation
- PowerShell module linking
- environment mutation
- persist linking

Phase 4: durability and recovery

- durable write of `manifest.json` and `install.json`
- structured rollback or repair on partial failure
- uninstall/reset compatibility for failed installs

## High-Value Install Test Matrix

- bucket app install
- local manifest path install
- URL manifest install
- explicit version install
- dependency install ordering
- helper dependency install
- `--independent`
- `--no-cache`
- `--skip-hash-check`
- `--global` admin gate
- unsupported architecture
- nightly install hash bypass
- persist file and directory cases
- shim overwrite and alternate shim preservation
- installer added PATH cleanup
- failed install repair on retry
