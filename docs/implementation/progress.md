# Production migration progress

## Baseline

- Source: c2841bc, preserved in local baseline commit 679c251 and immutable v0.1.0 tag.
- 150 Rust tests pass with localhost access; 9 frontend tests, typecheck and Clippy pass.
- Pre-existing formatting differences in 10 files; no real-app E2E/device certification.
- Remote protection/CI cannot be verified: GitHub CLI has no authentication.
- Main remains untouched. No push/publication or claim of remote CI success.

## Completed local increment: environment contract

Goal and boundaries are recorded in `baseline.md`. Development application version
is 0.2.0-alpha.0; internal library versions remain 0.1.0. Implemented: tracked Cargo
resolution, exact tool pins, stack manifest, read-only stack/doctor/verify commands,
negative validation tests, frozen bootstrap, locked Cargo recipes and CI manifest checks.

No runtime/catalog/database behavior changes and no new package dependencies.
E2E is not required for this tooling increment; it is required before connecting
canonical types to import, storage and frontend. Full native reproducibility is not
complete. Existing formatter debt was fixed in its own mechanical checkpoint.

Validation: 7 tooling tests pass on Node 22.22.3, including a reproduced/fixed Node
version-output parsing regression; selected-toolchain doctor passes and prints all
remaining native/model gaps. Frontend 9 tests/typecheck/lint and locked workspace
check pass after the app version change. Bootstrap shell syntax and Justfile parse
pass; clean setup and remote CI are not claimed.

Environment checkpoint: `90a4536` on `feat/baseline-environment`.

Mechanical formatting checkpoint: `f540c5a` on `feat/rust-format-baseline`, using
the selected rustfmt on the 10 pre-existing files; no intentional semantic changes.
Formatting and workspace Clippy pass. This does not change the historical baseline's debt.

## Completed local increment: catalog source contracts

Goal: add provider-independent source evidence types in `nova-core::catalog`.
Boundary: pure domain only; no Tauri, network, model or persistence dependency.
Files: core module/tests, catalog documentation and ADR. Research: existing parser
types and blueprint's conservative identity/SourceRecord contracts; no resolver or
identity algorithm implemented yet. Required tests: opaque ID validation/serde,
unaltered multilingual source text, bounded fields/attributes, debug redaction,
unknown kinds/episode numbers and separate entity/variant identity. E2E: no, this
module has no runtime consumers; yes before adapters publish its output. Migration
risk: none to user data. Rollback: remove the unconsumed module. Acceptance: focused
tests and neighboring workspace checks pass without changing current imports.

Implemented in `d93d82e` on `feat/catalog-source-contract`: bounded immutable SourceRecord,
source/snapshot/entity/edition/variant/binding IDs, protected-locator references,
explicit kind/numbering unknowns and a separate StreamVariant view. Ten integration
tests and one compile-fail doctest were added after first confirming the missing
contract failed compilation. Final full workspace run: 161 Rust tests pass; Clippy
and rustfmt pass. Seven environment tests and manifest verification pass. The nine
frontend tests/typecheck/lint passed at the unchanged frontend development checkpoint.
No dependencies, migrations, provider requests or user databases changed in this slice.

Local branches are stacked deliberately because main/remote CI integration is not
authorized by passing local tests alone. No branch is represented as merged. Review
environment → mechanical format → source contract in that order once remote CI is
available. No temporary compatibility layer or permanent feature flag was added.

Next implementation slice: SourceSnapshot completeness/lifecycle and reproducible
fingerprints, then fixture infrastructure. Do not connect the module to actual
import/publication or migrate favorites/history until the required fixture, storage
and app E2E gates exist. The full Catalog Compiler is not implemented yet.

## Outstanding integration gates

- Remote CI/protection/merge unavailable without GitHub authentication.
- No deterministic native E2E harness yet.
- Native dependency paths depend on Homebrew; stale build-script cache reproduced.
- Named Rust toolchain installation and clean bootstrap not exercised in this session.
- Models/native SDK hashes, linked SQLite status and license gates remain open.

## Rollback

Revert the environment commit to restore pre-migration tooling; no user data is
modified. v0.1.0 remains immutable. No schemas or compatibility adapters introduced.
