# Future Plan

This is the canonical long-horizon migration plan for turning `scoop-rs` into the default `scoop` on Windows. It is intentionally decision-complete: this file defines the target end state, phase order, and the exit criteria for each migration stage.

## Target End State

- `scoop-rs` is the default `scoop` users invoke.
- Distribution is binary-first and Windows-first.
- `scoop` self-update uses a versioned binary path, not a git-checkout path.
- Scoop manifests, layout, accepted command inputs, and functional command semantics are compatible with upstream.
- Human-facing CLI presentation is a stable scoop-rs contract and may diverge intentionally where clarity or ergonomics improve without changing command meaning.
- Parity-complete workloads are faster end-to-end than upstream PowerShell Scoop.

## Current Baseline

- Implemented commands: `bucket`, `cache`, `cat`, `cleanup`, `config`, `depends`, `download`, `export`, `help`, `hold`, `import`, `info`, `install`, `list`, `prefix`, `reinstall`, `reset`, `search`, `shim`, `status`, `unhold`, `uninstall`, `update`, `virustotal`, `which`.
- `scoop` self-update now follows a versioned manifest install pipeline and stages activation intent in-repo.
- Lifecycle parity work at command-layer is complete for behavior-contract intent.
- Command-layer boundary work is complete in-repo: self-update for `scoop` itself is handled in Rust through versioned install and staged activation intent; bootstrap, installer, and live-activation follow-up is tracked in [`THINGS_TO_ADDRESS.md`](/E:/scoop-rs/THINGS_TO_ADDRESS.md) and [`docs/bootstrap-updater.md`](/E:/scoop-rs/docs/bootstrap-updater.md).
- Active roadmap work now starts with Phase `2A`, while installer/bootstrap follow-up remains a dedicated Phase `2D` track and broader performance hardening remains Phase `3`.

## Migration Phases

### Phase 2A: Manifest, Resolution, and Input Parity

- Goal: close manifest-shape, variable-expansion, and command-input compatibility gaps that block real-world Scoop usage.
- Deliverables:
  - broader manifest field compatibility
  - richer variable substitution parity
  - URL, hash, redirect, and indirection parity
  - direct file, UNC, URL, and explicit-source manifest resolution closure for remaining commands
  - deterministic parser and input-validation behavior for manifest-backed commands
  - core manifest compatibility coverage anchored to `<upstream-scoop-root>/buckets/main`, `<upstream-scoop-root>/buckets/nonportable`, and `<upstream-scoop-root>/buckets/extras`
- Out of scope:
  - installer/bootstrap activation work
- Exit criteria:
  - manifest and input edge cases have direct fixture coverage
  - remaining manifest-resolution deltas are narrow, explicit, and low impact

### Phase 2B: Lifecycle, Side-Effect, and Layout Parity

- Goal: make install, download, uninstall, reset, and related side effects behave like dependable Scoop lifecycle operations in real environments.
- Deliverables:
  - unified download/cache/retry planning across lifecycle commands
  - archive extraction, persist, shim, shortcut, and environment-mutation edge-case coverage
  - uninstall/reset parity for lifecycle side effects and recovery flows
  - stable progress and summary output for lifecycle commands
- Out of scope:
  - root-entry bootstrap and live-engine activation
- Exit criteria:
  - lifecycle edge cases have fixture coverage across install, update, uninstall, and reset
  - remaining layout and side-effect deltas are intentional and documented

### Phase 2C: Bucket, Network, and CLI Robustness

- Goal: harden bucket, update, network, and CLI behavior so routine Scoop usage stays reliable under imperfect environments.
- Deliverables:
  - bucket and git-metadata resilience for `status`, `list`, and freshness checks
  - retry, proxy, rate-limit, and large-download robustness
  - deterministic CLI help, parser, and redirected-output behavior
  - policy coverage for packages that customize or depend on other packages
- Out of scope:
  - launcher and installer architecture work
- Exit criteria:
  - bucket/update/network flows fail predictably and recover cleanly
  - CLI help and parser behavior are stable enough for parity and fixture coverage

### Phase 2D: Installer, Bootstrap, and Live Self-Update Activation

- Goal: define and implement the bootstrap and activation boundary that turns `scoop-rs` into the default root entrypoint on Windows.
- Deliverables:
  - installer contract for root resolution, first payload layout, and migration from upstream bootstrap trees
  - bounded root-entry lifecycle for launcher, shim, or equivalent activation mechanism
  - live `scoop-rs.exe` self-update activation with recovery and rollback semantics
  - stable contract for direct execution of versioned engines
- Out of scope:
  - broad manifest-shape parity unrelated to activation
- Exit criteria:
  - first install, repair, and self-update share one documented activation model
  - live-engine activation gaps move out of `known-gap` status

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

### Phase 2A details

- Use [`THINGS_TO_ADDRESS.md`](/E:/scoop-rs/THINGS_TO_ADDRESS.md) as the source for issue clusters tagged to `Phase 2A`.
- Keep `Phase 2A` focused on inputs, manifests, and resolution rules before widening lifecycle or bootstrap scope.
- Treat `<upstream-scoop-root>/buckets/main`, `<upstream-scoop-root>/buckets/nonportable`, and `<upstream-scoop-root>/buckets/extras` as the default manifest corpus for compatibility passes unless a task explicitly targets a different bucket set.
- Every closed edge case must either remove a delta or add explicit coverage for why the delta remains.

### Phase 2B details

- Treat install, uninstall, reset, download, persist, shim, shortcut, and environment mutation as one lifecycle surface.
- Prefer shared typed substrates over command-specific patches whenever a lifecycle edge case appears in more than one command.

### Phase 2C details

- Group network, bucket, update, and CLI robustness work when they share the same failure surface.
- Prefer one resilient code path for retries, freshness checks, and parser behavior instead of separate ad hoc fixes.

### Phase 2D details

- Keep bootstrap and live-activation design work anchored to [`docs/bootstrap-updater.md`](/E:/scoop-rs/docs/bootstrap-updater.md).
- Treat root-entry continuity, security surface, and interruption recovery as first-class acceptance criteria.

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
- Keep machine-consumable outputs compatible; allow human-facing presentation to evolve as a documented scoop-rs contract.
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
- Day-to-day command coverage remains implemented with tests and benchmarks.
- The remaining entries in `BEHAVIOR_DELTAS.md` are narrow, intentional, and low impact.
- Benchmarks for parity-complete workloads consistently show `scoop-rs` faster than upstream.


