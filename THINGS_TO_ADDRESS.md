# Things to Address

This file is a catch-all for upstream Scoop issues that do not fit cleanly into the behavior delta log, but still matter for scoop-rs parity or for known gaps we should close in the reimplementation.

Scope rule: track feature requests, performance improvements, and bug reports only when the report describes a user-visible contract we need to match or a compatibility edge case we need to support. Pure upstream implementation bugs that we do not intend to reproduce do not belong here.

Track the issue number, the expected scoop-rs outcome, and whether the item is a command parity gap, a manifest-compatibility gap, or a performance target.

Scope:

- Put future work, unresolved issue clusters, performance targets, and edge cases here.
- Do not use this file as the canonical source for current behavior. If a difference is already observable in scoop-rs today, record it in `BEHAVIOR_DELTAS.md`.
- Avoid restating repo overview or benchmark-tool usage here.

## Network and download robustness

- [#5472](https://github.com/ScoopInstaller/Scoop/issues/5472) - Retries, resume support, and proxy coverage are all part of the same underlying problem: Scoop assumes the network is reliable enough to restart from scratch when it is not.
- [#6182](https://github.com/ScoopInstaller/Scoop/issues/6182) and [#6183](https://github.com/ScoopInstaller/Scoop/issues/6183) - TLS and transport failures should surface as actionable errors instead of leaving users with vague retry loops.
- [#6549](https://github.com/ScoopInstaller/Scoop/issues/6549) - `no_proxy` needs to be honored explicitly.
- [#6608](https://github.com/ScoopInstaller/Scoop/issues/6608) and [#6609](https://github.com/ScoopInstaller/Scoop/issues/6609) - GitHub API rate limiting should be handled with caching, token-aware requests, and fallbacks that do not break install or update flows.
- [#6567](https://github.com/ScoopInstaller/Scoop/issues/6567), [#6588](https://github.com/ScoopInstaller/Scoop/issues/6588), and [#6515](https://github.com/ScoopInstaller/Scoop/issues/6515) - Download progress, large archive handling, and partial-extraction edge cases should be tested together because they fail in the same pipeline.
- [#6500](https://github.com/ScoopInstaller/Scoop/issues/6500) - `get.scoop.sh` bootstrap failures need more deterministic handling than the current shell-driven path.

## Bucket, git, and update flow

- [#3894](https://github.com/ScoopInstaller/Scoop/issues/3894) - Previous-version install should have a git-history-backed fallback, not just ad hoc manifest generation.
- [#6587](https://github.com/ScoopInstaller/Scoop/issues/6587) - Buckets should work on non-main branches when the remote layout says they should.
- [#6594](https://github.com/ScoopInstaller/Scoop/issues/6594), [#6296](https://github.com/ScoopInstaller/Scoop/issues/6296), and [#6510](https://github.com/ScoopInstaller/Scoop/issues/6510) - `status` and bucket freshness checks need to survive missing or restricted git metadata without turning stable commands into noisy failures.
- [#6573](https://github.com/ScoopInstaller/Scoop/issues/6573) - `scoop list` should not become slow just because a deprecated bucket directory does not exist.
- [#6568](https://github.com/ScoopInstaller/Scoop/issues/6568) - Installing a specific app version from a custom bucket needs to work as a first-class path.
- [#6628](https://github.com/ScoopInstaller/Scoop/issues/6628) - Packages that customize other packages need a clear policy for preserving their changes across updates.
- Self-update now follows a versioned binary path instead of a git checkout path; command-layer `scoop` behavior is complete for this cycle (`HOLD_UPDATE_UNTIL`, changelog rendering, and versioned install + `current` activation). Installer/updater external bootstrap scripts remain responsible for first-install layout setup and locked/plain `apps/<app-name>/current` running-binary replacement per [`docs/bootstrap-updater.md`](/E:/scoop-rs/docs/bootstrap-updater.md).

## Install and lifecycle flow

- [#6568](https://github.com/ScoopInstaller/Scoop/issues/6568) and [#3894](https://github.com/ScoopInstaller/Scoop/issues/3894) - `install` now has a typed version-resolution path for bucket-backed `app@version` via git history; the remaining gap is direct manifest-source `@version` installs and upstream-style autoupdate manifest generation when git history is unavailable.
- [#6413](https://github.com/ScoopInstaller/Scoop/issues/6413), [#6338](https://github.com/ScoopInstaller/Scoop/issues/6338), and [#5472](https://github.com/ScoopInstaller/Scoop/issues/5472) - `install` download planning should unify cache reuse, retries, and duplicate-download avoidance because these fail together in the real workflow.
- [#6611](https://github.com/ScoopInstaller/Scoop/issues/6611), [#6248](https://github.com/ScoopInstaller/Scoop/issues/6248), and [#6179](https://github.com/ScoopInstaller/Scoop/issues/6179) - install-time extraction and persist linking now have fixture coverage for extract-dir/extract-to plus file and directory persist cases, but archive edge cases with symlinks and more exotic formats still need explicit tests.
- [#6632](https://github.com/ScoopInstaller/Scoop/issues/6632), [#6243](https://github.com/ScoopInstaller/Scoop/issues/6243), and [#6529](https://github.com/ScoopInstaller/Scoop/issues/6529) - install activation side effects now have command-level coverage for shims, shortcuts, environment mutation, PowerShell modules, and failed-install repair; remaining work is broader parity for uninstall/reset interactions and exact progress output.
## Shim, path, and filesystem layout

- [#6611](https://github.com/ScoopInstaller/Scoop/issues/6611) - Archives that contain symlinks or junctions should unpack in a way that preserves the intended filesystem shape.
- [#6612](https://github.com/ScoopInstaller/Scoop/issues/6612), [#6619](https://github.com/ScoopInstaller/Scoop/issues/6619), and [#6592](https://github.com/ScoopInstaller/Scoop/issues/6592) - Shim lookup and wildcard handling still have sharp edges that need explicit tests.
- [#6529](https://github.com/ScoopInstaller/Scoop/issues/6529) and [#6519](https://github.com/ScoopInstaller/Scoop/issues/6519) - Shim removal and junction creation both have failure modes that are easy to regress if they remain shell-script shaped.
- [#6316](https://github.com/ScoopInstaller/Scoop/issues/6316), [#6215](https://github.com/ScoopInstaller/Scoop/issues/6215), and [#6209](https://github.com/ScoopInstaller/Scoop/issues/6209) - Reset, `START /WAIT`, and external shim behavior should be treated as compatibility points, not incidental side effects.
- [#6243](https://github.com/ScoopInstaller/Scoop/issues/6243) - Shortcut targets should not be built by blindly appending to `$dir`.
- [#6248](https://github.com/ScoopInstaller/Scoop/issues/6248), [#6179](https://github.com/ScoopInstaller/Scoop/issues/6179), and [#3582](https://github.com/ScoopInstaller/Scoop/issues/3582) - `persist` layout is still too rigid for real-world drive and junction setups.

## Manifest and variable compatibility

- [#6615](https://github.com/ScoopInstaller/Scoop/issues/6615), [#6605](https://github.com/ScoopInstaller/Scoop/issues/6605), and [#6495](https://github.com/ScoopInstaller/Scoop/issues/6495) - Manifest fields that accept variables need a typed implementation, not one-off string expansion.
- [#6528](https://github.com/ScoopInstaller/Scoop/issues/6528), [#6526](https://github.com/ScoopInstaller/Scoop/issues/6526), and [#6271](https://github.com/ScoopInstaller/Scoop/issues/6271) - URL indirection, redirect-aware manifests, and alternate hash encodings are all manifest-shape concerns that should be handled together.
- [#6523](https://github.com/ScoopInstaller/Scoop/issues/6523) - Error reporting should include inner exceptions by default when it materially improves the CLI experience.
- [#6179](https://github.com/ScoopInstaller/Scoop/issues/6179) and [#6248](https://github.com/ScoopInstaller/Scoop/issues/6248) - `persist` needs richer manifest support than the current minimal path handling.
- [#6059](https://github.com/ScoopInstaller/Scoop/issues/6059) and [#5296](https://github.com/ScoopInstaller/Scoop/issues/5296) - Hash and URL model flexibility needs to stay aligned with what manifests already express in the wild.

## CLI errors and parser behavior

- [#6635](https://github.com/ScoopInstaller/Scoop/issues/6635), [#6634](https://github.com/ScoopInstaller/Scoop/issues/6634), and [#6633](https://github.com/ScoopInstaller/Scoop/issues/6633) - Small parser and help-text bugs can still break user trust because they show up in common commands.
- [#6251](https://github.com/ScoopInstaller/Scoop/issues/6251), [#6149](https://github.com/ScoopInstaller/Scoop/issues/6149), and [#6239](https://github.com/ScoopInstaller/Scoop/issues/6239) - Input validation and error shaping need to be deterministic, especially for `search` and manifest-backed commands.
- [#6441](https://github.com/ScoopInstaller/Scoop/issues/6441) and [#6270](https://github.com/ScoopInstaller/Scoop/issues/6270) - Help output and redirected output should behave like CLI users expect, not like PowerShell scripts accidentally inheriting console behavior.

## Performance and regression targets

- [#6632](https://github.com/ScoopInstaller/Scoop/issues/6632) - Architecture-specific shim selection is a correctness issue, but it also has a performance angle because the wrong shim path creates unnecessary retries.
- [#6413](https://github.com/ScoopInstaller/Scoop/issues/6413) and [#6338](https://github.com/ScoopInstaller/Scoop/issues/6338) - Duplicate downloads and false-success download prompts are regression traps that should be covered by fixture tests.
- [#6498](https://github.com/ScoopInstaller/Scoop/issues/6498) - Bucket tests can become pathological when a commit touches many files, so any file-walking rewrite should be benchmarked.
- [#6566](https://github.com/ScoopInstaller/Scoop/issues/6566) - Old-version retention and compression is a useful cleanup benchmark because it exercises both filesystem throughput and archive handling.
- `export -c` is currently a performance regression on this workstation (latest benchmark: upstream about 3x faster than scoop-rs). The command currently pays too much for installed-app and bucket metadata collection and needs profiling before Phase 1 can be called performance-clean.

## Working note

Anything here that changes observable stdout, stderr, exit codes, or filesystem layout should be promoted into `BEHAVIOR_DELTAS.md` once it is implemented or intentionally declined.

## Not Tracking

Pure upstream bugs that do not change the scoop-rs contract are not backlog items for this file. They can still be useful as cautionary examples during implementation, but they should not drive work unless they expose a parity requirement, a manifest edge case, or a performance regression worth fixing in Rust.
