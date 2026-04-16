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
- scoop-rs matches upstream on input and functional contracts; human-facing CLI presentation is allowed to be opinionated when the contract is stable and documented.
- scoop-rs may ship a clearer observable contract when an upstream issue cluster reflects unresolved defects, missing maintenance, or recurring workflow hazards; each shipped delta belongs in `BEHAVIOR_DELTAS.md`.
- Phase `2` is now split into `2A` manifest/resolution parity, `2B` lifecycle/layout parity, `2C` bucket/network/CLI robustness, and `2D` installer/bootstrap/live-activation work.
- `Phase 2A` uses `<upstream-scoop-root>/buckets/main`, `<upstream-scoop-root>/buckets/nonportable`, and `<upstream-scoop-root>/buckets/extras` as the core manifest compatibility corpus.
- Core corpus compatibility includes special manifests and lifecycle coverage across `install`, `uninstall`, and `reset`.
- Command-layer self-update owns version planning, staging, and activation intent.
- The final activation mechanism for a live `scoop-rs.exe` remains an open bootstrap/updater design space documented in `docs/bootstrap-updater.md`.

## Current User-Visible State

- Implemented commands: `bucket`, `cache`, `cat`, `cleanup`, `config`, `depends`, `download`, `export`, `help`, `hold`, `import`, `info`, `install`, `list`, `prefix`, `reinstall`, `reset`, `search`, `shim`, `status`, `unhold`, `uninstall`, `update`, `virustotal`, `which`.
- `install` covers dependency expansion, manifest path/URL installs, bucket git-history `app@version`, hooks, installers, shims, shortcuts, environment mutation, PowerShell modules, persist linking, failed-install purge, and extract-dir/extract-to handling.
- Shared lifecycle substrate now includes canonical hash-only cache keys and a persistent per-manifest version index for bucket-backed `app@version` resolution.
- `update` now syncs buckets and stages `scoop` self-updates through the same versioned manifest install path.
- `uninstall` and `reset` are implemented and use the same lifecycle substrate.
- Current accepted differences are explicit deltas in [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md).
- Canonical current behavior differences live in [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md).

## Immediate Open Gaps

- Active roadmap work starts with `Phase 2A`: manifest, resolution, and input parity.
- Current compatibility pressure centers on special manifests from the core corpus and their lifecycle behavior across `install`, `uninstall`, and `reset`.
- Installer/bootstrap follow-up and self-update activation for a live `scoop-rs.exe` are tracked in [`THINGS_TO_ADDRESS.md`](/E:/scoop-rs/THINGS_TO_ADDRESS.md) and [`docs/bootstrap-updater.md`](/E:/scoop-rs/docs/bootstrap-updater.md).

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

- Self-update activation lives behind the bootstrap/updater boundary; the exact mechanism remains open.
- Self-update must not regress back to git-checkout semantics.
- The product route is binary distribution, Windows-first.
