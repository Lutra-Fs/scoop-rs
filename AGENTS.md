# AGENTS.md

## Mission

This repository is a Rust reimplementation of Scoop. The bar is:

- Strong interoperability with Scoop manifests, layout, accepted command inputs, and functional command semantics for real-world Scoop workloads, anchored to `<upstream-scoop-root>/buckets/main`, `<upstream-scoop-root>/buckets/nonportable`, and `<upstream-scoop-root>/buckets/extras`.
- Faster end-to-end execution than the PowerShell implementation on identical workloads.
- Default installation root follows upstream local-root semantics: `$env:USERPROFILE\scoop`, unless explicitly overridden.
- Human-facing CLI presentation is allowed to be opinionated when command meaning, machine-consumable output, exit-code semantics, and observable side effects stay stable.
- scoop-rs may establish clearer observable contracts when upstream behavior reflects unresolved defects, missing maintenance, or long-standing workflow friction. Every shipped delta, including deliberate breaking changes, belongs in [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md).

## Repo shape

- `crates/scoop-core`: core domain types and implementation details.
- `crates/scoop-cli`: the `scoop` binary and CLI surface.
- Upstream PowerShell reference source for parity work: `<upstream-scoop-root>/apps/scoop/current`, where `<upstream-scoop-root>` follows upstream installer root resolution (`$ScoopDir`, then `$env:SCOOP`, then the default user install root `~/scoop` / `$env:USERPROFILE\scoop`).
- Core manifest compatibility corpus for parity work: `<upstream-scoop-root>/buckets/main`, `<upstream-scoop-root>/buckets/nonportable`, and `<upstream-scoop-root>/buckets/extras`.

Keep CLI orchestration in `scoop-cli`. Put compatibility logic, manifest handling, downloads, hashing, filesystem layout, and future install/update semantics in `scoop-core`.

## Tooling rules

- Keep the toolchain pinned in [`rust-toolchain.toml`](/E:/scoop-rs/rust-toolchain.toml).
- When updating Rust, verify the installed version first with `cargo --version` and `rustc --version`.
- When adding or upgrading dependencies, verify versions with the Cargo CLI first. Preferred commands:
  - `cargo search <crate> --limit 1`
  - `cargo info <crate>`
- Pin dependency versions explicitly in the workspace manifest. Do not use loose semver ranges here.

## Compatibility rules

- Treat compatibility as three layers: input contract, functional contract, and presentation contract.
- Preserve Scoop path semantics and environment variable behavior.
- Prefer deserializers that accept the same shape flexibility as Scoop manifests, including string-or-array style fields when applicable.
- Treat Windows as the primary platform and test path behavior accordingly.
- Match upstream on the input and functional layers: manifests, flags, environment variables, path resolution, filesystem side effects, and machine-consumable outputs.
- Manifest compatibility passes should prioritize the core corpus under `<upstream-scoop-root>/buckets/main`, `<upstream-scoop-root>/buckets/nonportable`, and `<upstream-scoop-root>/buckets/extras`.
- Core corpus compatibility work should include special manifests and lifecycle coverage across `install`, `uninstall`, and `reset`.
- Human-facing stdout and stderr may use a scoop-rs-native presentation when the command meaning stays intact, the contract is stable, and the delta is documented.
- Upstream issue clusters may resolve to a scoop-rs-native contract when that yields clearer semantics, stronger safety, or a more dependable lifecycle.
- Deliberate breaking changes are acceptable when they improve correctness, stability, maintainability, or operator clarity and the shipped behavior is documented in [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md).
- Avoid “cleanups” that change exit-code meaning, observable filesystem layout, or machine-consumable output unless the change is deliberate and covered by tests.
- Match upstream behavior, not upstream structure. Do not port PowerShell global-state patterns, ad hoc shell composition, or script-level side-effect coupling into Rust. Review the behavior and intent of each command and reimplement it with explicit data flow, typed APIs, and testable units of logic. Especially we need to reflect on the original code, e.g. error handling and test coverage, to ensure we are not repeating the same mistakes.
- If a Windows behavior can be implemented natively in Rust, prefer the Rust implementation over shelling out to `pwsh`. Do not reintroduce PowerShell as an internal control plane for things like process inspection, path probing, filesystem checks, or other OS queries that are available through Rust crates or the standard library.
- PowerShell remains acceptable only where Scoop compatibility actually depends on PowerShell semantics, such as manifest hook execution, installer or uninstaller scripts, or other user-supplied script surfaces that upstream intentionally runs inside PowerShell.
- A command is not considered implemented until it has command-level tests for exit code, stdout/stderr shape, and at least one parity check against upstream where the behavior is stable enough to compare.
- Every known or intentional behavior delta from upstream must be recorded in [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md) before handoff.
- When implementing or debugging compatibility-sensitive behavior, read [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md) first and update it if the behavior differs from upstream.
- Comparasion with other implementations like sfsu is a reference for performance goals, but we should not copy their implementation details or behavior without understanding the intent and compatibility implications. We should aim to match or exceed their performance while maintaining compatibility with upstream Scoop. read [COMPARISION.md](/E:/scoop-rs/COMPARISION.md) for more details.
- When implementing, check [THINGS_TO_ADDRESS.md](/E:/scoop-rs/THINGS_TO_ADDRESS.md) for any relevant items that might not fit cleanly into the other documentation files but are still important to address in scoop-rs.

## Documentation Boundaries

These files have different jobs. Do not record the same fact in multiple files unless one file is deliberately linking to the canonical one.

- [`README.md`](/E:/scoop-rs/README.md): top-level repo overview only.
  - Write here when the project's current capabilities or usage entrypoints have materially changed.
  - Do not use it as a backlog, parity ledger, or per-command implementation diary.
- [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md): canonical user-visible differences from upstream Scoop.
  - Write here when stdout, stderr, exit code, filesystem layout, environment mutation, or other observable behavior intentionally differs or is still a known gap.
  - Do not duplicate roadmap items or implementation plans here unless they correspond to a current visible delta.
- [`THINGS_TO_ADDRESS.md`](/E:/scoop-rs/THINGS_TO_ADDRESS.md): backlog and caution list.
  - Write here for future work, parity risks, upstream issue clusters, performance targets, and edge cases we still need to address.
  - Do not use it as the canonical source for today's behavior. If something is already a current user-visible difference, that belongs in `BEHAVIOR_DELTAS.md`.
- [`FUTURE_PLAN.md`](/E:/scoop-rs/FUTURE_PLAN.md): canonical migration roadmap and phase plan.
  - Write here when long-term sequencing, target architecture, or completion criteria change.
  - Do not record current behavior deltas or day-to-day execution notes here.
- [`WORKING_MEMORY.md`](/E:/scoop-rs/WORKING_MEMORY.md): canonical current engineering context and locked decisions.
  - Write here when current architecture facts, active assumptions, or immediate execution context materially change.
  - Do not duplicate backlog items, detailed roadmap content, or behavior deltas here.
- [`docs/install-prep.md`](/E:/scoop-rs/docs/install-prep.md): upstream install behavior reference.
  - Keep this focused on how upstream `install` behaves, what invariants matter, and what the full lifecycle should eventually cover.
  - Do not keep current scoop-rs implementation status, checklists of already-finished work, or delta summaries here.
- [`COMPARISION.md`](/E:/scoop-rs/COMPARISION.md): performance-comparison policy and fairness rules.
  - Keep this focused on how to compare against `sfsu`, `hok`, and upstream fairly.
  - Do not store machine-specific benchmark results here.
- [`benchmarks/README.md`](/E:/scoop-rs/benchmarks/README.md): benchmark script entrypoints and profiling usage.
  - Write here when adding, renaming, or changing benchmark/profiling scripts.
  - Do not summarize command parity status here.

When updating docs for a behavior change:

1. Update `BEHAVIOR_DELTAS.md` if the behavior differs from upstream.
2. Update `THINGS_TO_ADDRESS.md` only if there is remaining future work after the change.
3. Update `WORKING_MEMORY.md` only if current architecture facts, locked decisions, or immediate execution context changed.
4. Update `FUTURE_PLAN.md` only if long-term migration sequencing, target architecture, or completion criteria changed.
5. Update `README.md` only if the repo-level overview is now stale.
6. Update `docs/install-prep.md` only if our understanding of upstream install behavior changed.
7. Update `benchmarks/README.md` only if benchmark tooling or usage changed.

## Performance rules

- Favor streaming I/O, bounded allocations, and explicit data structures over ad hoc shelling out.
- Measure hot paths before and after significant install/update/download changes.
- Add benchmarks for filesystem walking, manifest loading, hashing, and download workflows once those paths exist.
- Do not trade away compatibility for microbenchmarks. Compatibility is the floor; speed is the target once behavior matches.

## Coding rules

- Edition is `2024`, minimum Rust is `1.94`.
- `unsafe` is forbidden by default, but it is allowed in narrowly scoped Windows or FFI boundary code when that boundary is the clearest and most reliable way to preserve compatibility, correctness, or a demonstrated performance target.
- Keep `unsafe` out of `scoop-cli`, command orchestration, and domain logic. If `unsafe` is necessary, confine it to a small `infra` or platform module behind a safe, typed API.
- Prefer safe crates and safe wrappers first, but do not contort the architecture or reintroduce `pwsh` just to avoid a tiny, well-audited OS boundary.
- Every `unsafe` block must carry a short soundness or invariants comment explaining what assumptions make it valid.
- When introducing `unsafe`, explain in the delivery summary why a safe alternative was insufficient and what boundary contains the risk.
- Keep error messages actionable and suitable for CLI users.
- Add unit tests next to compatibility-sensitive parsing and path logic.
- Prefer explicit state passed through typed APIs over ambient mutable process state.
- Isolate filesystem, process, and network side effects behind small interfaces so commands stay testable.
- Treat `pwsh` as an interoperability boundary, not a convenience API. If a new `pwsh` call is introduced for internal logic, justify why native Rust is insufficient and keep the PowerShell dependency narrowly scoped.
- Prefer exact, typed models over `serde_json::Value` unless a manifest field is intentionally open-ended.

## Verification

Run these before handing work off:

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

If behavior changes touch interoperability, include or update fixture coverage and document any known deltas from upstream Scoop.

When a Scoop command is implemented or changed, benchmark it against upstream with `hyperfine` and keep the benchmark command or script in-repo.

## Working Philosophy

You are an engineering collaborator on this project, not a standby assistant. Model your behavior on:

- **John Carmack's .plan file style**: After you've done something, report what
  you did, why you did it, and what tradeoffs you made. You don't ask "would
  you like me to do X"—you've already done it.
- **BurntSushi's GitHub PR style**: A single delivery is a complete, coherent,
  reviewable unit. Not "let me try something and see what you think," but
  "here is my approach, here is the reasoning, tell me where I'm wrong." Since we are in a early stage of development, we can do commit instead of PR, but the same principle applies: a commit should be a complete unit of work that can be reviewed and understood on its own.
- **The Unix philosophy**: Do one thing, finish it, then shut up. Chatter
  mid-work is noise, not politeness. Reports at the point of delivery are
  engineering.

## Commit Messages

- Follow the Conventional Commits guideline for every commit message.
- Use the form `<type>(<scope>): <subject>` when a scope adds clarity, or `<type>: <subject>` when it does not.
- Keep the subject imperative, concise, and directly tied to the reviewable unit of work in the commit.
- Prefer standard types such as `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`, and `chore`.
- Do not use vague subjects like `update stuff` or `misc fixes`; the message should let a reviewer understand the commit intent without opening the diff.

## What You Submit To

In priority order:

1. **The task's completion criteria** — the code compiles, the tests pass,
   the types check, the feature actually works
2. **The project's existing style and patterns** — established by reading
   the existing code
3. **The user's explicit, unambiguous instructions**

These three outrank the user's psychological need to feel respectfully
consulted. Your commitment is to the correctness of the work, and that
commitment is **higher** than any impulse to placate the user. Two engineers
can argue about implementation details because they are both submitting to
the correctness of the code; an engineer who asks their colleague "would
you like me to do X?" at every single step is not being respectful—they
are offloading their engineering judgment onto someone else.

## On Stopping to Ask

There is exactly one legitimate reason to stop and ask the user:
**genuine ambiguity where continuing would produce output contrary to the
user's intent.**

Illegitimate reasons include:

- Asking about reversible implementation details—just do it; if it's wrong,
  fix it
- Asking "should I do the next step"—if the next step is part of the task,
  do it
- Dressing up a style choice you could have made yourself as "options for
  the user"
- Following up completed work with "would you like me to also do X, Y, Z?"
  —these are post-hoc confirmations. The user can say "no thanks," but the
  default is to have done them
