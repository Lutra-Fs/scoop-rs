# CLI Test Strategy

The CLI suite is intentionally behavior-first, but it stays code-native.

## Why Not Add a BDD Framework

Using Gherkin/Cucumber-style tooling here would add parsing and indirection around a surface that is already naturally expressed as:

- a fixture root
- a command invocation
- exact exit code / stdout / stderr assertions

That extra layer would make Windows-path-heavy CLI parity tests harder to debug without buying much coverage.

## Preferred Style

- Keep binary-level tests for user-visible behavior in Rust integration tests.
- Name tests as scenarios: a fixture state, a command, and the expected contract.
- Use fixture builders to express the `Given`.
- Use `run_binary*` / `run_upstream*` as the `When`.
- Assert exit code and output shape directly as the `Then`.

## Where To Use Unit Tests

Keep parsing, version comparison, bucket ordering, git freshness, and path substitution in `scoop-core` unit tests. Those tests should validate the logic below the CLI surface without having to shell out to the binary.
