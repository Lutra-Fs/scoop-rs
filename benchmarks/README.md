# Benchmarks

Use the PowerShell helpers below to compare an implemented Rust command against upstream Scoop, or against the other Rust reimplementations when you want a broader performance check:

```powershell
.\scripts\benchmark-command.ps1 -Name help -Args help -IterationsPerRun 50 -Warmup 1 -Runs 3
.\scripts\benchmark-compare.ps1 -Name help -Args help -IterationsPerRun 50 -Warmup 1 -Runs 3
.\scripts\benchmark-suite.ps1 -Scenario all
.\scripts\benchmark-search-cache.ps1 -Scenario all -Query google
.\scripts\benchmark-install-fixture.ps1 -Warmup 1 -Runs 3
.\scripts\benchmark-uninstall-fixture.ps1 -Warmup 1 -Runs 3
.\\scripts\\benchmark-update-fixture.ps1 -Warmup 1 -Runs 3
```

Scope:

- This file documents benchmark and profiling entrypoints only.
- Current compatibility gaps belong in [`BEHAVIOR_DELTAS.md`](/E:/scoop-rs/BEHAVIOR_DELTAS.md).
- Performance-comparison policy belongs in [`COMPARISION.md`](/E:/scoop-rs/COMPARISION.md).

Notes:

- `IterationsPerRun` batches repeated command invocations so Windows process-launch noise does not dominate very fast commands.
- The script benchmarks generated wrapper commands through `cmd.exe /c call ...`, which produced stable results on this machine where other `hyperfine` shell modes did not.
- JSON outputs are written to `benchmarks/*.json` and are ignored by git because they are machine-specific measurements.
- Scripts that invoke upstream Scoop resolve the root from `-UpstreamScoopRoot`, then `SCOOP`, then `$env:USERPROFILE\scoop`.
- `benchmark-search-cache.ps1` resolves `-ScoopRoot` with the same root-selection order.
- Fixture prepare scripts are generated into `benchmarks/` at runtime; repo-tracked benchmark helpers stay generic.
- For the four-way comparison, install `sfsu` and `hok` with Scoop and make sure `Get-Command sfsu` and `Get-Command hok` resolve on PATH before running `benchmark-compare.ps1`.
- The four-way compare keeps upstream Scoop as the behavior baseline, while `sfsu` and `hok` are reference points for speed.
- `benchmark-suite.ps1` runs the named scenarios from the sfsu benchmark notes: `list`, `search-cold`, `search-warm`, `info-full`, and `info-fair`.
- `benchmark-search-cache.ps1` measures `scoop-rs` SQLite-cache `search` cold-build and rebuild cost by resetting or corrupting `scoop.db` before each run and then restoring the original database afterward.
- `benchmark-install-fixture.ps1` measures a controlled install workload against upstream Scoop and `scoop-rs` with isolated fixture roots.
- `benchmark-uninstall-fixture.ps1` measures a controlled uninstall workload against upstream Scoop and `scoop-rs` using the same isolated fixture roots and an upstream-prepared install state.
- `benchmark-update-fixture.ps1` measures a controlled no-argument `update` workload against upstream Scoop and `scoop-rs`; upstream runs against a git-backed Scoop core checkout, while `scoop-rs` runs against a versioned installed `scoop` binary plus a git-backed bucket that advertises a newer `scoop` manifest.

Benchmark tips:

- Use warmups. The sfsu benchmark notes show that warm caches and repeated launches materially change the numbers, especially for search.
- Compare cold and warm cache cases separately. Search is much faster with SQLite cache enabled, so collapsing those cases hides the real shape of the work.
- Keep the Scoop branch fixed when comparing against upstream. The sfsu notes use `develop`, which is the right choice for parity comparisons against the latest upstream behavior.
- Treat `info` carefully. `hok` does not expose the `Updated at` and `Updated by` fields that Scoop and sfsu can print, so fairness requires either a reduced comparison or a separate mode.
- Use the same bucket set, manifest set, and installed apps across runs. The benchmark numbers are only useful if the inputs are stable.
- Before running the suite, make sure the machine state matches the scenario you want. For example, `search-cold` should run with `use_sqlite_cache` disabled, while `search-warm` should run with it enabled.

Profiling:

- `scoop-rs` has an internal profiler for command hot paths. Set `SCOOP_RS_PROFILE=1` to print phase timings to `stderr`.
- Example:

```powershell
$env:SCOOP_RS_PROFILE='1'
.\target\release\scoop.exe info git 1>$null
.\target\release\scoop.exe status 1>$null
```

- The profiler is intended for local investigation only. It does not affect normal CLI output unless the environment variable is set.
