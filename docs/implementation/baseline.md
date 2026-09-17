# Pre-migration baseline — 17 September 2026

Application source checkpoint: `c2841bc`, previously `feat/ui-identity`, 14 commits
ahead of local `main` (`eaf7216`). No existing tags. App, root/frontend package and
Rust packages declare 0.1.0. Preserve that version as local historical tag `v0.1.0`;
this is not a claim of stable/public release certification. Next application
development target: `0.2.0-alpha.0`. Internal library versions remain independent.

The worktree initially contained only the two requested architecture documents.
They are preserved with this baseline. Cargo.lock existed locally but was ignored;
track the existing resolution without upgrading dependencies. The large generated
lockfile is intentionally part of the reproducibility checkpoint.

## Code state

Tauri 2 / React 19 / Rust 2021 monorepo, 16 Rust workspace members, per-profile
SQLite/SQLx with 19 profile migrations plus the registry migration. Existing paths:
M3U/Xtream import and refresh, browse/details/series, favorites/watchlist/history,
mpv playback, AVFoundation AirPlay integration, XMLTV Now/Next, playhead-window
Whisper transcription, CPU CTranslate2/NLLB translation and FTS/E5 search.
Code presence does not establish fresh app/device verification. Native mpv PiP
is disabled. Full EPG, canonical catalog/compiler and complete E2E remain future.

Import orchestration lives in Tauri commands; raw row IDs and stream URLs remain
coupled to catalog/user state; frontend queries group duplicates. PlayerModal
combines substantial orchestration/presentation. Native mpv uses OpenGL rendering.
Database downgrade handling ignores missing migrations. CSP is disabled. These
are migration concerns, not changes made in this checkpoint.

## Validation

- Frontend: all 9 tests in 4 files pass; TypeScript and ESLint pass.
- Rust Clippy: workspace/all targets passes with warnings denied.
- Rust tests: full workspace and doc tests pass on the final socket-enabled rerun.
  Initial full run failed
  on a cached Homebrew mpv path (0.41.0_4 versus installed 0.41.0_7). Setting
  `PKG_CONFIG_PATH=/opt/homebrew/opt/mpv/lib/pkgconfig` invalidated the existing
  build-script cache; no source fix or dependency upgrade was required.
- Sandbox denied socket binding in two hls-proxy HTTP tests; rerun with local
  socket access distinguishes environment restrictions from regressions.
- `cargo fmt --all -- --check` finds pre-existing formatting differences in
  11 files. Do not hide this or mix wholesale formatting into environment work.
- No current app E2E runner, device certification or packaged-app validation.

Observed tools: Rust/Cargo 1.95.0, pnpm 9.15.4, just 1.51.0; selected existing
Node 22.22.3 is installed, but default PATH selected Node 26.5.0. Cargo and just
were absent from PATH; mise shim points to an unavailable mise executable.
Observed standalone media tools: mpv 0.41.0, FFmpeg 8.1.2, libplacebo 7.360.1.
These do not yet prove the app's loaded runtime dependency closure. macOS 26.6.2,
SDK 26.2 observed; app deployment target is 14.0. Model digests/linked SQLite still
need runtime inventory.

CI currently checks Rust, frontend tests/typecheck/lint on PRs and main pushes;
Rust/toolchain/Homebrew resolution floats. No E2E or distribution workflow exists.
GitHub CLI lacks authentication: remote CI and branch protection are unverified.
No push, remote protection change, publication or merge is part of this checkpoint.

## Immediate increment contract

Goal: make selected versions and environment drift inspectable. Affected subsystem:
development tooling only. Boundary: do not initialize user databases or load models.
Files: stack manifest, scripts/tests, task recipes, CI checks and environment docs.
Research: Cargo lockfile guidance and Node subprocess APIs. Tests: parser/validator
negative cases, runtime probe failures, manifest integration and existing checks.
E2E: not required for a read-only developer command; remains mandatory before
import/storage/player migrations. Migration risk: no user-data changes. Rollback:
revert tooling commit; immutable baseline tag remains. Acceptance: deterministic
validation, actionable missing-tool/drift errors, no secrets or silent installs.

Evidence: [Cargo lockfile guidance](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html)
supports tracking application resolution; [Node 22 subprocess documentation](https://nodejs.org/download/release/latest-jod/docs/api/child_process.html)
supports shell-free bounded probes. Adopt both; no new package dependency is needed.
