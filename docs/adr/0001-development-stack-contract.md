# ADR 0001: explicit development stack contract

Status: implemented on the environment feature branch; not yet integrated into main.
Date: 2026-09-17.

## Context and evidence

The app had a pnpm lockfile but ignored Cargo.lock, floating Rust/mise versions and
an ambient Node version different from the intended major. Baseline tests reproduced
a stale native library path. The [Cargo guidance](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html)
supports tracking application resolution. Native reproducibility needs more than a
package lock and is intentionally not claimed here.

## Decision

Track the existing Cargo.lock; pin observed development tools; add a JSON manifest
checked against existing config/locks/migration heads. Use Node's standard library
for read-only verification and bounded shell-free probes. Keep native/model unknowns
visible and fail on drift. Setup is explicit and separate from diagnosis.

## Alternatives and consequences

A Python tool would introduce another environment for simple repository validation;
a Rust xtask would require the toolchain it diagnoses before it could run. Node is
already required by the frontend and can report missing Rust/just. JSON avoids a new
manifest parser dependency. Minimal lock readers deliberately validate known generated
version fields, not every TOML/YAML construct. Future full graph/native artifact
verification must extend this contract with appropriate parsers or tool outputs.

Roll back by reverting the tooling commit; no profile data/migrations are involved.
CI checks repository consistency, while native SDK, models, clean setup and E2E remain
separate gates. Do not treat a green manifest as production certification.
