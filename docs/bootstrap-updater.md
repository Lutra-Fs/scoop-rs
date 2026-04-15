# Bootstrap and Self-Update Contract

This document defines the ownership boundary and design space for `scoop-rs` first-install, self-update, and activation of the `scoop` package itself.

The current goal is a clear contract. The implementation choice stays open until we validate the tradeoffs against security, operability, and long-term maintenance.

## Current facts

### Upstream Scoop bootstrap

The standalone upstream installer script bootstraps Scoop into `apps/scoop/current` and `buckets/main`.

- It prefers `git clone` for both repositories.
- It falls back to zip download and extraction when `git` is unavailable.
- It writes the `scoop` shim and default config values during bootstrap.

That bootstrap path creates a live working tree directly under `apps/scoop/current`.

### Upstream Scoop self-update

Upstream `scoop update scoop` updates a PowerShell script distribution.

- `pwsh.exe` is the running process.
- `scoop.ps1` and its library files are data files from the Windows loader point of view.
- `update_scoop` can therefore refresh the checkout in place through git-based flows.

### `scoop-rs` runtime constraint

`scoop-rs` ships a Windows executable. `scoop-rs.exe` is held by the Windows image loader while the process is alive.

This creates a different activation model:

- a running `apps/scoop/current/scoop-rs.exe` path needs a process-external handoff before the active binary can change safely
- `no_junction=true` changes the activation shape because the active path is version-specific instead of `current`
- direct invocation of a versioned engine path and invocation through a stable root entrypoint need the same staged-update contract

## Contract boundary

### Command layer ownership

`scoop-core` and `scoop-cli` own the package-management semantics of self-update.

- Resolve the target manifest and version.
- Download, verify, and extract the target version into `apps/scoop/<version>/`.
- Prepare activation metadata for the next handoff step.
- Report update results through the CLI contract.

This keeps `scoop` self-update on the same typed install substrate as ordinary packages.

### Bootstrap / updater ownership

Bootstrap and updater code own activation of the `scoop` package when the currently active executable is live.

- Normalize first-install layout from bootstrap into the long-term `scoop-rs` layout.
- Complete activation when the running path is currently active.
- Recover from interrupted activation and preserve a usable root entrypoint.
- Refresh any stable root entrypoint used to launch `scoop-rs`.

The exact mechanism stays open. The boundary is stable: command code stages versions; bootstrap/updater code finalizes activation.

## Layout assumptions

The long-term package layout for `scoop-rs` self-update is:

- package root: `apps/scoop/`
- version payload: `apps/scoop/<version>/`
- activation state:
  - junction mode: `apps/scoop/current -> apps/scoop/<version>/`
  - `no_junction=true`: `apps/scoop/<version>/` plus an activation record such as `active-version.txt`
- stable root entrypoint, if present: `shims/scoop.exe` or an equivalent fixed launcher path

Bootstrap code owns migration from layouts that begin as a plain `apps/scoop/current` directory.

## Security goals

Any self-update design for `scoop-rs` should satisfy these constraints:

1. Keep the trusted code surface small and auditable.
2. Limit file operations to the Scoop root and planned version directories.
3. Validate staged payload hash and resolved paths before activation.
4. Preserve side-by-side versions until the new version is known-good.
5. Keep privilege level aligned with the invoking user and the selected Scoop root.
6. Support deterministic recovery after interruption, crash, or partial activation.

## Evaluated approaches

### 1. Stable launcher

A small stable launcher at a fixed path starts the active engine, watches for staged self-update outcomes, and completes activation after the child process exits.

| Aspect | Evaluation |
| --- | --- |
| Fit | Strong fit for long-term architecture |
| Operational model | `scoop.exe` stays stable; `apps/scoop/<version>/scoop-rs.exe` changes |
| Security profile | Small trusted surface; narrow file-operation set; clear audit target |
| Strengths | Handles running-binary handoff cleanly, supports `current` and `no_junction`, gives one root trust anchor |
| Risks | Root entrypoint needs its own update policy; launcher protocol becomes a compatibility surface |

### 2. Script stub transition layer

A thin `pwsh` or `cmd` stub owns entrypoint and activation while Rust owns command logic.

| Aspect | Evaluation |
| --- | --- |
| Fit | Medium fit as a transitional path |
| Operational model | Shell stub resolves active version and forwards into Rust |
| Security profile | Entry logic inherits shell semantics and environment surface |
| Strengths | Fastest migration path; close to upstream entry behavior; easy bootstrap story |
| Risks | Shell becomes a long-lived control plane; quoting and environment behavior widen the trust surface |

### 3. Dedicated helper binary

The main engine stages the target version and launches a separate helper to finalize activation after exit.

| Aspect | Evaluation |
| --- | --- |
| Fit | Medium fit |
| Operational model | Helper consumes an activation plan and switches active version |
| Security profile | Small binary is possible; launch surface and plan validation become the key controls |
| Strengths | Straightforward to implement; supports immediate activation |
| Risks | Adds another executable and another trust boundary; helper update policy needs its own lifecycle |

### 4. Deferred next-run activation

`scoop-rs` stages the new version and writes activation intent. The next trusted entry through the root command completes activation.

| Aspect | Evaluation |
| --- | --- |
| Fit | Strong fit as a fallback or paired strategy |
| Operational model | Current run prepares; next invocation activates |
| Security profile | Small surface and simple control flow |
| Strengths | Clean around Windows loader locks; easy recovery model; low complexity |
| Risks | Activation becomes eventually consistent; explicit user messaging matters |

### 5. Windows service

A local service owns all activation and replacement work.

| Aspect | Evaluation |
| --- | --- |
| Fit | Weak fit for Scoop scale |
| Operational model | Service performs version switching and recovery |
| Security profile | Highest privilege and lifetime complexity in this set |
| Strengths | Strong central control over activation |
| Risks | Expands operational and security scope dramatically; install and recovery complexity rise with it |

## Security discussion for new files and new binaries

The main security concern is the introduction of an always-trusted root file such as a launcher or helper.

That concern is real, and it shapes the design bar:

- every extra executable becomes part of the trusted computing base for updates
- every file format used for handoff becomes a protocol that needs validation rules
- every root-level file replacement path becomes part of the recovery and tamper story

The strongest mitigations are structural:

1. Keep any root executable tiny and purpose-built.
2. Keep its accepted inputs typed, versioned, and path-bounded.
3. Require activation plans to reference already-verified staged versions.
4. Keep activation code side-effect-focused and free of general command logic.
5. Prefer a stable root entrypoint with rare updates over a frequently changing helper fleet.

This is why a stable launcher and deferred next-run activation currently look stronger than a general helper model.

## Preferred direction

The current design preference is:

1. Stable root launcher as the long-term entrypoint and activation trust anchor.
2. Deferred next-run activation as the default fallback path.
3. Dedicated helper only if supervised activation needs a tighter immediate handoff than the launcher can provide.

This preference gives `scoop-rs` a small trusted surface, a clear audit target, and one coherent model for first install, repair, `current` activation, and `no_junction` activation.

## Open design questions

These questions remain active:

- root entrypoint format: stable launcher binary, shell stub, or staged transition from one to the other
- launcher self-update policy: installer or repair refresh, side-by-side A/B update, or another bounded root update path
- activation record format for `no_junction=true`
- interrupted activation recovery record and rollback policy
- direct execution policy for `apps/scoop/<version>/scoop-rs.exe`

## Implementation boundary for later phases

The implementation work can happen later. The boundary is already defined.

- `scoop-core` / `scoop-cli` continue to own version planning, download, verification, extraction, and activation intent.
- bootstrap / updater code owns first-install normalization, active-engine handoff, activation completion, and root entrypoint continuity.
- current docs and behavior deltas should describe live self-update of `scoop-rs` as a staged-update path with activation finalized by the future bootstrap/updater layer.
