# scoop-rs

Rust reimplementation of Scoop with a hard target of upstream-compatible inputs and command outcomes across real-world Scoop workloads, anchored to the core upstream buckets, plus materially better performance than the PowerShell implementation.

## Current scope

- Workspace split into `scoop-core` and `scoop-cli`.
- Implemented command surface: `help`, `version`, `list`, `cat`, `info`, `search`, `status`, `prefix`, `which`, and a substantial `install` path.
- Upstream-compatible SQLite-backed `search` cache with local and cross-implementation benchmarks.
- Windows-first install lifecycle support covering dependency planning, URL/path manifests, bucket git-history `app@version`, installer hooks, shims, shortcuts, environment mutation, PowerShell modules, and persist handling.
- Theme-safe semantic ANSI colors for CLI status output, with `--color auto|always|never`.
- Human-facing CLI presentation follows a stable scoop-rs contract and can be more opinionated than upstream where clarity improves.
- Default install root follows upstream local-root semantics: `$env:USERPROFILE\scoop`.
- Core compatibility work stays anchored to `<upstream-scoop-root>/buckets/main`, `<upstream-scoop-root>/buckets/nonportable`, and `<upstream-scoop-root>/buckets/extras`, including special-manifest lifecycle flows across `install`, `uninstall`, and `reset`.

## Near-term priorities

1. Close `Phase 2A` manifest, resolution, and input-parity gaps.
2. Advance `Phase 2B` and `Phase 2C` lifecycle, network, bucket, and CLI robustness work.
3. Keep `Phase 2D` installer/bootstrap activation design moving while benchmarks stay current for implemented commands.

## Documentation map

- [`AGENTS.md`](/E:/scoop-rs/AGENTS.md): repo rules and documentation-boundary rules.
- [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md): canonical current differences from upstream.
- [`THINGS_TO_ADDRESS.md`](/E:/scoop-rs/THINGS_TO_ADDRESS.md): backlog, issue clusters, and future parity/performance targets.
- [`docs/install-prep.md`](/E:/scoop-rs/docs/install-prep.md): upstream install behavior reference.
- [`COMPARISION.md`](/E:/scoop-rs/COMPARISION.md): comparison policy for upstream, `sfsu`, and `hok`.
- [`benchmarks/README.md`](/E:/scoop-rs/benchmarks/README.md): benchmark and profiling script usage.

## Upstream reference

Use `<upstream-scoop-root>/apps/scoop/current` as the PowerShell parity reference when porting behavior. Resolve `<upstream-scoop-root>` with the same root-selection order used by the upstream installer: explicit installer path, then `$env:SCOOP`, then the default user install root `~/scoop` / `$env:USERPROFILE\scoop`.

Use `<upstream-scoop-root>/buckets/main`, `<upstream-scoop-root>/buckets/nonportable`, and `<upstream-scoop-root>/buckets/extras` as the core manifest compatibility corpus for `Phase 2A` work.
