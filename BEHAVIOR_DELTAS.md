# Behavior Deltas

This is the canonical file for current user-visible behavior differences from upstream Scoop.

Read and update this file when implementing or debugging compatibility-sensitive behavior.
`AGENTS.md` points here on purpose so the delta log stays in one place.

Scope:

- Put current deltas here when the difference is already observable today.
- Do not put future work, plans, or general caution items here unless they describe a present user-visible gap.
- Do not duplicate backlog summaries from `THINGS_TO_ADDRESS.md`.

## Record Format

- `Command`: affected command or subsystem.
- `Status`: `intentional`, `known-gap`, or `environmental`.
- `Why`: short explanation of the difference.
- `Test handling`: how parity is asserted today.

## Current Entries

- `Command`: `list`
  `Status`: `intentional`
  `Why`: invalid regex input returns one actionable CLI error instead of upstream's repeated PowerShell error spam.
  `Test handling`: fixture test asserts the Rust-side error contract; parity tests only cover stable successful/no-match cases.

- `Command`: `cat`
  `Status`: `intentional`
  `Why`: when `cat_style` is configured but `bat` is unavailable or fails, `scoop-rs` falls back to plain pretty JSON instead of surfacing a shell-execution failure.
  `Test handling`: fixture tests verify `bat` invocation when available; plain JSON parity remains covered against upstream.

- `Command`: `search`
  `Status`: `intentional`
  `Why`: invalid regex diagnostics are normalized to a shorter Rust-side message instead of PowerShell's full regex parser text.
  `Test handling`: fixture tests assert the Rust-side contract; parity tests cover stable successful paths.

- `Command`: `search`
  `Status`: `intentional`
  `Why`: when `USE_SQLITE_CACHE` is enabled and `scoop.db` is missing, empty, or corrupt, `scoop-rs` rebuilds it on demand from local bucket manifests instead of requiring a separate cache-population step.
  `Test handling`: core cache tests cover initial build and missing-database rebuild; CLI tests cover sqlite-mode search semantics.

- `Command`: `search`
  `Status`: `intentional`
  `Why`: sqlite-cache mode uses case-insensitive literal substring matching for remote known-bucket lookup, instead of upstream's raw regex interpolation into the GitHub tree filter. Local sqlite-cache results still follow upstream-style partial matching against `name`, `binary`, and `shortcut`.
  `Test handling`: parity tests cover stable local sqlite-cache output; remote-bucket tests cover the Rust-side contract separately.

- `Command`: `status`
  `Status`: `intentional`
  `Why`: network-failure handling is intentionally compact and routed through output levels; when fetch cannot be checked, `status` reports a single warning line and continues with a success exit code.
  `Test handling`: fixture tests assert network-failure warning output; local contract tests still assert table output for `status -l`.

- `Command`: `download`
  `Status`: `known-gap`
  `Why`: `download app@version` currently reuses scoop-rs versioned-manifest resolution and git-history lookup, but does not yet synthesize arbitrary historical manifests through upstream's `autoupdate` generation path. Versions absent from bucket history may therefore fail where upstream can still generate a temporary manifest.
  `Test handling`: CLI tests cover usage, missing-manifest parity, and local fixture downloads; no parity assertion is made yet for upstream-generated historical manifests.

- `Command`: `export`
  `Status`: `known-gap`
  `Why`: exported `Updated` timestamps are semantically aligned with upstream but still normalized in parity tests because upstream's object serialization can differ in sub-second values across fixture paths.
  `Test handling`: CLI tests compare stable export fields and normalize `Updated` before semantic parity assertions.

- `Command`: `import`
  `Status`: `intentional`
  `Why`: missing-path handling is normalized to a direct scoop-rs usage error instead of upstream PowerShell parameter-binding stderr.
  `Test handling`: parity tests cover stable invalid-JSON behavior; fixture tests cover successful config/bucket/app/hold import flows.

- `Command`: `cache` / `download`
  `Status`: `known-gap`
  `Why`: cached payload filenames currently follow scoop-rs's simpler `app#version#leafname` layout instead of upstream Scoop's URL-derived `cache_path` naming. `download` reuses this shared cache layout, so cache-hit success-path parity is only asserted against the Rust-side contract today.
  `Test handling`: cache and download fixture tests assert the current layout and cache reuse behavior; parity tests are limited to stable usage and missing-manifest paths.

- `Command`: `install`
  `Status`: `intentional`
  `Why`: `install` now covers multi-app planning, manifest path/URL installs, direct manifest-source `@version` validation, bucket git-history `app@version`, dependency expansion, helper dependency planning, nightly version stamping, installer execution, hooks, shims, shortcuts, PowerShell modules, environment mutation, persist linking, failed-install purge, and extract-dir/extract-to handling.
  `Test handling`: fixture tests cover install side effects, nightly behavior, URL/path manifests, extract-dir/extract-to, dependency ordering, failed-install repair, suggestions, and shim argument substitution. Parity tests cover usage, missing manifests, and already-installed output.

- `Command`: `install`
  `Status`: `intentional`
  `Why`: output for install is now mapped to explicit levels (`WARN`, `INFO`, `VERBOSE`) and can be filtered by default `--quiet`/`--verbose` flags. Core progress and actionable lines are in `INFO`, while intermediate side effects (for example shim creation) are `VERBOSE`.
  `Test handling`: command-level tests still assert side effects and behavior on disk; CLI tests assert stable install outcome lines and known missing-manifest/already-installed cases.

- `Command`: `install`
  `Status`: `intentional`
  `Why`: when link creation for `apps/<app>/current` is unavailable, `scoop-rs` falls back to copying the version directory into `current` so installs remain usable without shelling out for junction creation.
  `Test handling`: fixture tests assert the observable `current` directory contents, not the exact reparse-point type.

- `Command`: `upstream parity harness`
  `Status`: `environmental`
  `Why`: on this machine, upstream Scoop emits unrelated config-access noise on stderr because `C:\Users\lutra\.config\scoop\config.json` is denied. That stderr is not a command contract we intend to reproduce.
  `Test handling`: parity tests normalize stderr by allowing that exact environmental failure while still comparing stable stdout behavior.

- `Command`: `uninstall`
  `Status`: `intentional`
  `Why`: success output is condensed to one summary line per app and then filtered by output levels; running-process failures stay as error-level output and still surface under `--quiet`.
  `Test handling`: CLI tests cover success, purge, running-process skip, and `--quiet` filtering behavior; core tests cover install-then-uninstall round-trip and not-installed paths.

- `Command`: `update`
  `Status`: `known-gap`
  `Why`: `update` with no arguments now syncs git buckets and self-updates `scoop-rs` through the normal manifest install pipeline: a newer `scoop` manifest is installed into a versioned directory and then activated by switching `apps/<app-name>/current`. Lifecycle output is now level-mapped (`WARN`/`INFO`/`VERBOSE`) with `--quiet`/`--verbose` support; the remaining known gap is external installer/updater ownership of running-binary replacement when the active `current` path is non-switchable.
  `Test handling`: fixture tests cover no-arg self-update, explicit `update scoop`, install-triggered self-update, already-latest, and running-process skip behavior.

- `Command`: `reset`
  `Status`: `intentional`
  `Why`: `reset` follows the same lifecycle substrate as install/uninstall and output is filtered by output levels (`--quiet` hides the INFO summary, error paths remain visible).
  `Test handling`: reset reuses shared install/uninstall primitives, and CLI tests cover shim restoration and `--quiet` behavior.

- `Command`: `reinstall`
  `Status`: `intentional`
  `Why`: upstream `reinstall` is a shim alias that loops raw arguments through `scoop uninstall` and `scoop install` separately. `scoop-rs` implements it as explicit CLI orchestration with shared option parsing and then reuses the existing uninstall/install command handlers. Stable usage and missing-app output match upstream, but odd alias edge cases from raw per-argument looping are intentionally not reproduced.
  `Test handling`: CLI tests cover install-on-missing behavior plus exact parity for usage and missing-app output.

- `Command`: `shim`
  `Status`: `intentional`
  `Why`: `shim alter` is implemented as a deterministic next-alternative switch instead of upstream's interactive choice prompt, so the command stays testable and non-interactive inside scoop-rs.
  `Test handling`: CLI tests cover exact parity for stable usage errors and fixture round-trips for add/list/info/rm; core tests cover deterministic `alter` switching.

- `Command`: `virustotal`
  `Status`: `known-gap`
  `Why`: scoop-rs currently implements argument handling, dependency expansion, API-key enforcement, manifest validation, and passthru stubs, but does not yet perform real VirusTotal hash/url API queries or submissions.
  `Test handling`: CLI tests cover exact usage parity plus the Rust-side missing-API-key contract; no parity assertion is made yet for live lookup paths.
