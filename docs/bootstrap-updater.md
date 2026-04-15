# Bootstrap and Self-Update Updater Contract

This document defines the Phase 2 ownership boundary for binary distribution and self-update replacement.

## Core principle

`scoop-core` and `scoop-cli` should remain command-oriented and do not own runtime executable replacement.

- They own self-update for `scoop` package via the same install/activation lifecycle used for any app (install a versioned payload and switch `current` when successful).
- They do not perform in-process replacement of a running CLI binary.
- They never assume a fixed app-name for the install root layout.

## Assumed package layout

- Scoop package root: `apps/<app-name>/`.
- Versioned payload directory: `apps/<app-name>/<version>/`.
- Active checkout: `apps/<app-name>/current` as a switchable link/junction/reparse-point to the active `<version>` directory.

`<app-name>` is the package identifier resolved by package metadata (for Scoop this is currently `scoop`), not a hard-coded path.

This prevents command-layer self-update from colliding with any pre-existing PowerShell Scoop installation layout.

## Responsibility split

### Command layer (`scoop-core` / `scoop-cli`)

- Determine whether self-update is needed (`LAST_UPDATE`, `HOLD_UPDATE_UNTIL`).
- Resolve target manifest reference and install the new manifest artifact into a versioned directory.
- Switch `apps/<app-name>/current` to the new version directory when the install operation succeeds.
- Return outcome (`ScoopUpdated`, changelog/hold status, etc.) without trying to replace a locked running binary.

This includes the first-install and fresh-`scoop` update workflow for this project: after a bootstrap installer lays down an initial `<app-name>/current`, subsequent self-updates are expected to stay on the versioned install + current-switch model.

### Installer/updater (external)

- Own all executable replacement semantics when the active binary is currently running.
- Handle the Windows lock scenario where the active `current` entry is a plain directory that cannot be replaced in place.
- Treat a plain, non-switchable `apps/<app-name>/current` as a packaging contract violation for command-layer assumptions; migration and normalization must be handled here.
- Perform safe staging, atomic handoff, and rollback for running binary replacement.
- Perform migration from older non-versioned layouts where needed.
- Own first-install bootstrap and upgrade from legacy distributions.

## Transition note

- The command-layer boundary is intentionally narrow: a stable launcher/updater component can be added later without changing command behavior contracts.
- Any behavior delta caused by the lock scenario should be documented as an explicit installer/updater responsibility until the packaging layer is complete.
