# Production migration progress

## Baseline

- Source: c2841bc, preserved in local baseline commit 679c251 and immutable v0.1.0 tag.
- 150 Rust tests pass with localhost access; 9 frontend tests, typecheck and Clippy pass.
- Pre-existing formatting differences in 11 files; no real-app E2E/device certification.
- Remote protection/CI cannot be verified: GitHub CLI has no authentication.
- Main remains untouched. No push/publication or claim of remote CI success.

## Active increment: environment contract

Goal and boundaries are recorded in `baseline.md`. Development application version
is 0.2.0-alpha.0; internal library versions remain 0.1.0. Implemented: tracked Cargo
resolution, exact tool pins, stack manifest, read-only stack/doctor/verify commands,
negative validation tests, frozen bootstrap, locked Cargo recipes and CI manifest checks.

No runtime/catalog/database behavior changes and no new package dependencies.
E2E is not required for this tooling increment; it is required before connecting
canonical types to import, storage and frontend. Full native reproducibility is not
complete. Existing formatter debt will be fixed in its own mechanical checkpoint.

Validation: 7 tooling tests pass on Node 22.22.3, including a reproduced/fixed Node
version-output parsing regression; selected-toolchain doctor passes and prints all
remaining native/model gaps. Frontend 9 tests/typecheck/lint and locked workspace
check pass after the app version change. Bootstrap shell syntax and Justfile parse
pass; clean setup and remote CI are not claimed.

Next: commit; introduce a small pure
catalog domain module with opaque typed IDs, preserved source evidence and explicit
unknowns. Do not connect it to real imports or migrate user state before fixture/E2E
coverage. Every compatibility layer will require a removal condition.

## Blockers and gaps

- Remote CI/protection/merge unavailable without GitHub authentication.
- No deterministic native E2E harness yet.
- Native dependency paths depend on Homebrew; stale build-script cache reproduced.
- Named Rust toolchain installation and clean bootstrap not exercised in this session.
- Models/native SDK hashes, linked SQLite status and license gates remain open.

## Rollback

Revert the environment commit to restore pre-migration tooling; no user data is
modified. v0.1.0 remains immutable. No schemas or compatibility adapters introduced.
