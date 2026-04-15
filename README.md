# scoop-rs

Rust reimplementation of Scoop with a hard target of full interoperability and materially better performance than the PowerShell implementation.

## Current scope

- Workspace split into `scoop-core` and `scoop-cli`.
- Implemented command surface: `help`, `version`, `list`, `cat`, `info`, `search`, `status`, `prefix`, `which`, and a substantial `install` path.
- Upstream-compatible SQLite-backed `search` cache with local and cross-implementation benchmarks.
- Windows-first install lifecycle support covering dependency planning, URL/path manifests, bucket git-history `app@version`, installer hooks, shims, shortcuts, environment mutation, PowerShell modules, and persist handling.
- Default install root set to `D:/Applications/Scoop`.

## Near-term priorities

1. Close the remaining install parity gaps.
2. Build the uninstall/reset/update lifecycle on top of the install substrate.
3. Keep command-level parity tests and benchmark coverage moving with each implemented command.

## Documentation map

- [`AGENTS.md`](/E:/scoop-rs/AGENTS.md): repo rules and documentation-boundary rules.
- [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md): canonical current differences from upstream.
- [`THINGS_TO_ADDRESS.md`](/E:/scoop-rs/THINGS_TO_ADDRESS.md): backlog, issue clusters, and future parity/performance targets.
- [`docs/install-prep.md`](/E:/scoop-rs/docs/install-prep.md): upstream install behavior reference.
- [`COMPARISION.md`](/E:/scoop-rs/COMPARISION.md): comparison policy for upstream, `sfsu`, and `hok`.
- [`benchmarks/README.md`](/E:/scoop-rs/benchmarks/README.md): benchmark and profiling script usage.

## Upstream reference

Use `<upstream-scoop-root>/apps/scoop/current` as the PowerShell parity reference when porting behavior. Resolve `<upstream-scoop-root>` with the same root-selection order used by the upstream installer: explicit installer path, then `$env:SCOOP`, then the default user install root `~/scoop` / `$env:USERPROFILE\scoop`.
