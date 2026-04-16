# Behavior Deltas

This is the canonical file for current user-visible behavior differences from upstream Scoop.

Read and update this file when implementing or debugging compatibility-sensitive behavior.
`AGENTS.md` points here on purpose so the delta log stays in one place.

Scope:

- Put current deltas here when the difference is already observable today.
- Do not put future work, plans, or general caution items here unless they describe a present user-visible gap.
- Do not duplicate backlog summaries from `THINGS_TO_ADDRESS.md`.

## Current Deltas

| Command | Behavior | Upstream Scoop | scoop-rs | Status | Test handling / rationale |
| --- | --- | --- | --- | --- | --- |
| `list` | Invalid regex diagnostics | Repeats PowerShell regex/parser noise | Returns one actionable CLI error | `intentional` | Fixture test asserts the Rust-side contract; parity tests cover stable success and no-match paths |
| `cat` | `bat` fallback | Surfaces shell-execution failure when configured `bat` is unavailable | Falls back to plain pretty JSON | `intentional` | Fixture tests verify `bat` invocation when available; JSON parity remains covered |
| `search` | Invalid regex diagnostics | Emits full PowerShell regex parser text | Emits a shorter normalized error | `intentional` | Fixture tests assert the Rust-side contract; parity tests cover stable successful paths |
| `search` | SQLite cache bootstrap | Requires separate cache population | Rebuilds `scoop.db` on demand when missing, empty, or corrupt | `intentional` | Core cache tests cover initial build and rebuild; CLI tests cover sqlite-mode search semantics |
| `search` | Remote known-bucket matching in SQLite mode | Interpolates raw regex into the upstream GitHub tree filter | Uses case-insensitive literal substring matching for remote bucket names; local sqlite results still follow upstream-style partial matching | `intentional` | Parity tests cover stable local sqlite output; remote-bucket tests cover the Rust-side contract |
| `status` | Network-failure reporting | Can emit broader script-shaped fetch noise | Emits one compact warning and keeps a success exit code | `intentional` | Fixture tests assert warning output; `status -l` table behavior still has parity coverage |
| `import` | Missing-path handling | Surfaces PowerShell parameter-binding noise | Returns a direct scoop-rs usage error | `intentional` | Parity tests cover stable invalid-JSON behavior; fixture tests cover successful import flows |
| `cache` / `download` / `cleanup --cache` | Cache filename layout | Reads legacy `app#version#underscored-url`, otherwise writes/uses `app#version#sha7.ext` | Reads and writes only canonical `app#version#sha7.ext` | `intentional` | Core tests cover canonical naming and cache parsing; CLI tests cover cache reuse and cleanup. Legacy cache reuse is deliberately dropped instead of carrying upstream migration debt |
| `CLI` | Color rendering and control | PowerShell `Write-Host` colors depend on host semantics and do not expose a unified `--color` switch | Uses theme-safe ANSI semantic colors with `--color auto|always|never`; `auto` respects TTY detection and `NO_COLOR` | `intentional` | CLI tests cover forced color, forced plain output, and strip-ANSI regressions; parity tests continue comparing normalized plain text |
| `export` | JSON rendering contract | PowerShell object serialization decides key order and timestamp formatting | Preserves the same top-level shape and field meanings (`apps`, `buckets`, optional `config`) but uses Rust JSON serialization and format-compatible comparisons | `intentional` | CLI parity tests canonicalize object order and normalize `Updated`; the accepted bar is format compatibility and better performance, not byte-for-byte serializer parity |
| `install` | Output shaping | Script output is not level-structured | Maps output to explicit `WARN` / `INFO` / `VERBOSE` levels and filters with `--quiet` / `--verbose` | `intentional` | Fixture tests assert side effects and stable outcome lines; parity tests cover usage, missing manifests, and already-installed output |
| `install` | `apps/<app>/current` activation fallback | Relies on link creation semantics | Falls back to copying the version directory when link creation is unavailable | `intentional` | Fixture tests assert observable `current` contents, not the exact reparse-point type |
| `uninstall` | Success output | More verbose script-shaped success output | Emits one condensed summary line per app; error-level output still survives `--quiet` | `intentional` | CLI tests cover success, purge, running-process skip, and quiet-mode behavior |
| `reset` | Output shaping | Script output is not level-structured | Reuses the lifecycle substrate and filters output through explicit levels | `intentional` | CLI tests cover shim restoration and `--quiet` behavior |
| `reinstall` | Command implementation and missing-app exit behavior | Alias-style loop over raw arguments; missing-app behavior depends on upstream alias/environment interaction and commonly exits non-zero | Explicit CLI orchestration over shared uninstall/install handlers; the documented missing-app contract is stable stdout with exit code `0` | `intentional` | CLI tests cover usage parity, install-on-missing behavior, and the documented missing-app orchestration contract |
| `shim` | `shim alter` interaction model | Interactive choice prompt | Deterministic next-alternative switch | `intentional` | CLI tests cover stable usage errors and round-trips for add/list/info/rm; core tests cover deterministic `alter` switching |
| `update` | `scoop` live self-update activation | PowerShell script files can refresh in place during `update_scoop` because the running process is `pwsh.exe` | `scoop-rs` stages versioned payloads through Rust install/update flows; final activation of a live `scoop-rs.exe` remains a future bootstrap/updater mechanism documented in [`docs/bootstrap-updater.md`](/E:/scoop-rs/docs/bootstrap-updater.md) | `known-gap` | Fixture tests cover no-arg self-update, explicit `update scoop`, already-latest, and running-process skip behavior; the remaining gap is the process-external activation path |
| Upstream parity harness | Local machine stderr noise | On this machine, upstream emits unrelated config-access noise for `C:\Users\lutra\.config\scoop\config.json` | scoop-rs does not reproduce it | `environmental` | Parity tests normalize that exact upstream stderr so stdout contracts remain comparable |
