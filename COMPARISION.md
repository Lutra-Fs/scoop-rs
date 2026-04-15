# Comparison with sfsu and hok

[`sfsu`](https://github.com/winpax/sfsu) and [`hok`](https://github.com/chawyehsu/hok) are the closest Rust reference implementations worth comparing against for performance work. Their benchmark notes are useful as baselines, but they are not compatibility targets. Our target is upstream Scoop behavior first, with equal or better throughput on the same workloads.

Scope:

- Keep this file focused on fairness rules, comparison methodology, and what should be measured.
- Do not store machine-specific benchmark results or current behavior-gap summaries here.

Use `hyperfine` scripts checked into the repository to compare scoop-rs against upstream Scoop and, where useful, against `sfsu` and `hok` on the same machine, with the same cache state, manifest set, and network assumptions.

The sfsu benchmark notes reinforce a few discipline rules that we should keep:

- Run with warmups.
- Measure cold-cache and warm-cache search separately.
- Keep the upstream branch fixed when comparing against Scoop.
- Be fair about command shape differences, especially for `info`, where `hok` does not print all of the same fields.
- Keep the bucket set and installed app set identical across runs.

## Comparison Caveats

- `sfsu` and `hok` are useful speed references, but they are not strict behavioral baselines.
- Some command surfaces are not directly comparable one-to-one. For example, `hok` does not currently expose a `status` command, and ambiguous package references in `hok cat` or `hok info` can trigger package-selection flows instead of Scoop-style resolution.
- Prefer exact package references such as `main/git` when comparing manifest-backed commands across implementations.
- Treat comparisons as meaningful only when the command shape, work performed, and output contract are close enough to be considered the same user task.

## What To Measure

- `scoop install` on a small package, a large package, and a package with many files.
- `scoop update` with a cold cache, a warm cache, and at least one simulated failure.
- `scoop bucket add`, `scoop bucket update`, and `scoop bucket list` because git and filesystem walking dominate those paths.
- `scoop search`, `scoop info`, and `scoop cat` because manifest scanning and parsing show up in everyday use.
- `scoop list`, `scoop cleanup`, and `scoop uninstall` because they exercise directory walking, shim management, and version retention.
- `scoop status` because git metadata handling tends to be both slow and failure-prone.

## What To Record

- Wall-clock time.
- Download reuse versus re-download count.
- Number of git operations and archive extraction operations.
- Error behavior on partial failures, especially whether one failed package blocks the whole run.
- Any change in stdout, stderr, or exit code relative to upstream Scoop.

## Benchmark Scenarios Worth Keeping

- Cold start install of a representative app set.
- Update of a bucket-heavy environment with several installed apps.
- Cleanup of a machine with many old versions retained.
- Search on a large manifest set with and without a local cache.
- Network-fragile runs that cover retries, proxy behavior, and GitHub API throttling.

## Why This Matters

Benchmarks should tell us two things at once:

- Whether scoop-rs is faster than upstream Scoop for the same command.
- Whether scoop-rs is faster than other Rust Scoop reimplementations without dropping compatibility.
- Whether a speedup came from a real implementation improvement or from a compatibility regression that silently skipped work.

If the latter happens, the benchmark is signaling a bug, not a win.
