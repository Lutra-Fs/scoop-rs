# Future Plan

This is the canonical long-horizon migration plan for turning `scoop-rs` into the default `scoop` on Windows. It is intentionally decision-complete: this file defines the target end state, phase order, and the exit criteria for each migration stage.

## Target End State

- `scoop-rs` is the default `scoop` users invoke.
- Distribution is binary-first and Windows-first.
- `scoop` self-update uses a versioned binary path, not a git-checkout path.
- Scoop manifests, layout, and user-visible CLI behavior are compatible with upstream.
- Parity-complete workloads are faster end-to-end than upstream PowerShell Scoop.

## Current Baseline

- Implemented commands: `help`, `cat`, `info`, `install`, `list`, `prefix`, `reset`, `search`, `status`, `uninstall`, `update`, `which`.
- `scoop` self-update now follows a versioned manifest install pipeline and activates via `current` by command-layer logic.
- Lifecycle parity work at command-layer is complete for behavior-contract intent.
- Command-layer boundary work is complete in-repo: self-update for `scoop` itself is handled in Rust through versioned install + pointer switch; runtime binary replacement for locked or non-switchable active directories is handled separately via the external installer/updater contract in [`docs/bootstrap-updater.md`](/E:/scoop-rs/docs/bootstrap-updater.md).
- Remaining gaps are mainly installer-side bootstrap responsibilities and intentionally scoped lifecycle output behavior differences.

## Migration Phases

### Phase 1: Missing Core Commands

- Goal: close the biggest command-surface gaps after core lifecycle and bootstrap work.
- Deliverables, in priority order:
  - `bucket`
  - `config`
  - `cleanup`
  - `cache`
  - `download`
  - `hold` / `unhold`
  - then lower-priority commands such as `depends`, `export`, `import`, `shim`, `virustotal`, `reinstall`
- Out of scope:
  - deep performance tuning not required to make a command usable
  - experimental features not present in upstream Scoop
- Exit criteria:
  - each new command has command-level tests, stable parity coverage where practical, and a benchmark entrypoint
  - behavior deltas are documented immediately when parity is intentionally incomplete

### Phase 2: Manifest and Edge-Case Parity Sweep

- Goal: close the remaining manifest-shape and edge-case compatibility gaps that only appear in real-world Scoop usage.
- Deliverables:
  - broader manifest field compatibility
  - richer variable substitution parity
  - archive, persist, shim, and redirect edge-case handling
  - direct file / UNC / URL manifest behavior closure for remaining commands
  - targeted fixes for upstream issue clusters already tracked in [`THINGS_TO_ADDRESS.md`](/E:/scoop-rs/THINGS_TO_ADDRESS.md)
- Out of scope:
  - non-Windows-first abstractions that do not help Scoop parity
- Exit criteria:
  - the remaining deltas are narrow, explicitly intentional, and low user impact
  - manifest and filesystem edge cases have direct fixture coverage

### Phase 3: Performance Hardening and Regression Harness

- Goal: make speed claims durable after parity is substantially complete.
- Deliverables:
  - parity-complete benchmark set
  - partial-parity benchmark set kept separate
  - hot-path profiling for install/update/search/status/download flows
  - regression thresholds for key command classes
- Out of scope:
  - trading away compatibility for raw microbenchmark wins
- Exit criteria:
  - parity-complete commands are benchmarked against upstream consistently
  - performance regressions are easy to detect and reproduce
  - documentation distinguishes parity-complete and partial-parity comparisons

## Phase Details

### Phase 1 details

- Implement commands in the prioritized order above unless a dependency forces a small reorder.
- For each command:
  - finish parser behavior
  - implement the command with typed APIs in `scoop-core`
  - add fixture tests and stable parity tests
  - add or update benchmark coverage
- Do not mark a command complete until tests and benchmark entrypoints exist.

### Phase 2 details

- Use [`THINGS_TO_ADDRESS.md`](/E:/scoop-rs/THINGS_TO_ADDRESS.md) as the source for issue clusters; do not mirror that backlog here.
- Prefer grouped compatibility passes:
  - manifest variables
  - archive/extraction
  - persist/layout
  - URL/hash/redirect handling
- Every closed edge case must either remove a delta or add explicit coverage for why the delta remains.

### Phase 3 details

- Keep parity-complete benchmarking separate from partial-parity benchmarking.
- Reuse in-repo benchmark scripts; add new scripts only when a workload cannot be expressed fairly with current ones.
- Benchmark categories to maintain:
  - lifecycle
  - search/list/info/status
  - download/extraction
  - self-update
- Performance sign-off requires both:
  - no material parity regressions
  - measured improvement on the same workload shape

## Cross-Cutting Rules

- Binary self-update remains versioned, not git-checkout based.
- Prefer native Rust for OS behavior unless Scoop compatibility explicitly depends on PowerShell semantics.
- Do not change observable behavior without fixture or parity coverage.
- Benchmark every command after meaningful changes.
- Keep documentation boundaries strict:
  - present deltas in `BEHAVIOR_DELTAS.md`
  - future work in `THINGS_TO_ADDRESS.md`
  - current architecture facts in `WORKING_MEMORY.md`
  - long-horizon sequencing here

## Completion Bar

The migration is complete only when all of the following are true:

- `scoop-rs` can serve as the default `scoop` binary product on Windows.
- Self-update, install, update, uninstall, and reset are stable and parity-complete enough for normal user workflows.
- Remaining commands needed for day-to-day Scoop use are implemented with tests and benchmarks.
- The remaining entries in `BEHAVIOR_DELTAS.md` are narrow, intentional, and low impact.
- Benchmarks for parity-complete workloads consistently show `scoop-rs` faster than upstream.


