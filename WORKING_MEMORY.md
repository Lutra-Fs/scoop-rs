# Working Memory

This is the canonical compact engineering memory for the current `scoop-rs` state. Read it first when resuming work so implementation does not re-open settled decisions or duplicate facts that already have canonical homes elsewhere.

## Current Architecture

- Workspace shape:
  - `crates/scoop-cli`: CLI parsing, command dispatch, stdout/stderr/exit-code shaping
  - `crates/scoop-core`: command logic and implementation details
- Internal split in `scoop-core`:
  - `app`: command-oriented use cases
  - `domain`: pure Scoop rules and typed context
  - `infra`: filesystem, git, HTTP, sqlite, Windows/platform boundaries
  - `compat`: upstream-facing behavior adapters and manifest resolution/rendering

## Locked Decisions

- `scoop` self-update is versioned binary self-update, not git-checkout self-update.
- Runtime OS queries should prefer native Rust, not `pwsh`.
- PowerShell remains only for compatibility surfaces such as hooks, installers, and uninstallers.
- `unsafe` is allowed only in narrow infra/platform boundaries behind safe typed APIs.
- Documentation boundaries are explicit; do not duplicate the same fact across plan, memory, backlog, and delta files.
- Command-layer self-update is split from executable replacement: command code handles version planning and activation only; installer/updater owns locked running-binary replacement under the contract in `docs/bootstrap-updater.md`.

## Current User-Visible State

- Implemented commands: `help`, `cat`, `info`, `install`, `list`, `prefix`, `reset`, `search`, `status`, `uninstall`, `update`, `which`.
- `install` covers dependency expansion, manifest path/URL installs, bucket git-history `app@version`, hooks, installers, shims, shortcuts, environment mutation, PowerShell modules, persist linking, failed-install purge, and extract-dir/extract-to handling.
- `update` now syncs buckets and self-updates `scoop` through the same versioned manifest install path.
- `uninstall` and `reset` are implemented and use the same lifecycle substrate.
- Canonical current behavior differences live in [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md).

## Immediate Open Gaps

- No immediate open gaps are currently tracked here; active behavior gaps remain in [BEHAVIOR_DELTAS.md](/E:/scoop-rs/BEHAVIOR_DELTAS.md) and `THINGS_TO_ADDRESS.md`.

## First Files To Read

- [`AGENTS.md`](/E:/scoop-rs/AGENTS.md)
- [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md)
- [`THINGS_TO_ADDRESS.md`](/E:/scoop-rs/THINGS_TO_ADDRESS.md)
- [`docs/install-prep.md`](/E:/scoop-rs/docs/install-prep.md)

## Working Rules

- Run before handoff:
  - `cargo fmt --all`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
- When a Scoop command changes, benchmark it against upstream with an in-repo `hyperfine` script.
- Update docs only in their canonical file:
  - current observable gaps -> `BEHAVIOR_DELTAS.md`
  - backlog / issue clusters -> `THINGS_TO_ADDRESS.md`
  - long-horizon roadmap -> `FUTURE_PLAN.md`
  - current architecture facts / locked decisions -> `WORKING_MEMORY.md`

## Do Not Re-Decide

- Installer/bootstrap owns running-exe replacement.
- Self-update must not regress back to git-checkout semantics.
- The product route is binary distribution, Windows-first.
