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

Next implementation slice started below: SourceSnapshot completeness/lifecycle and
reproducible fingerprints, then fixture infrastructure. Do not connect the module to actual
import/publication or migrate favorites/history until the required fixture, storage
and app E2E gates exist. The full Catalog Compiler is not implemented yet.

## Completed local increment: snapshot receipts and fingerprints

Goal: bounded endpoint capture, immutable completion receipts and reproducible
fingerprints in core/catalog submodules. No network, persistence, publication or
existing importer changes. Research: pinned sha2 streaming API and Rust's warning
that DefaultHasher is not stable across releases. Tests: fixed hash vectors,
chunk/order invariance, field-boundary separation, explicit partial/empty scopes,
terminal lifecycle errors, foreign records and quota failures. E2E: not required
until wired into app paths. No DB migration; rollback removes unconsumed APIs.
Acceptance: focused tests, full regression, Clippy, formatting and manifest checks.

Implemented on `feat/catalog-snapshots`: complete/partial/failed/cancelled capture,
immutable receipt, safe scope fallback, bounded streaming payload digest, framed v1
metadata and locator fingerprints. Thirteen new regression tests pass, including
independent fixed encoding vectors. Full workspace: 174 Rust tests pass; workspace
Clippy with warnings denied and rustfmt pass. Frontend: 9 tests, typecheck and lint
pass. Tooling: 7 tests and manifest verification pass. Existing sha2 0.10.9 is now a
direct core dependency; Cargo.lock changes only that edge, with no package upgrades.

Research and the capture-versus-compiler lifecycle decision are in ADR 0003. No
durable snapshot files, encryption/key storage, compilation, migrations, provider
requests or publication are implemented by this slice. No user-data changes. No
performance improvement is claimed; the helper retains only fixed hash state and
counters, and actual import performance awaits the adapter/staging benchmark.

Next: deterministic sanitized provider fixtures and adapter contract tests, followed
by protected snapshot persistence with crash/replay tests. Keep existing import
behavior until the storage and actual-app E2E gates are satisfied.

## Completed local increment: deterministic provider fixture contracts

Goal: exercise actual existing clients against an isolated synthetic HTTP provider.
Boundary: tools/iptv-fixture plus fixtures/tests, with one isolated M3U correction
after the fixture reproduced a Unicode slicing panic. See [regression evidence](m3u-utf8-regression.md).
Research: pinned Axum graceful-shutdown contract; reviewed current M3U/Xtream/EPG
APIs and import-validation skill. Required tests: normal and partial/fault endpoints,
raw names, duplicate IDs, episode order, XMLTV offsets, readiness and shutdown.
E2E: not claimed; this is a prerequisite fixture service, not a Tauri test. Migration
risk: no profile access or data changes. Rollback removes the dev-only tool/member.
Acceptance: fixture tests, full workspace regression, Clippy/format/manifest checks;
prove desktop dependency graph does not include the fixture crate.

Implemented on `feat/iptv-fixture-contracts`: versioned synthetic M3U/Xtream/XMLTV
corpus, loopback-only provider with ephemeral ports and bounded shutdown, plus nine
integration tests using the actual parser clients. Fault scenarios distinguish
HTTP failure, malformed payload and valid empty scope. The fixture reproduced an
M3U Unicode panic; an isolated regression test failed before the narrow correction.

Validation: all 184 Rust tests pass, including nine fixture tests and the new M3U
regression. Workspace Clippy with warnings denied, rustfmt, frontend typecheck/lint,
nine frontend tests, seven environment tests and manifest verification pass.
The desktop normal dependency graph excludes nova-iptv-fixture. Cargo.lock adds
only the local tool package; no external packages were upgraded. No Python venv or
Docker service is introduced. No user profile or provider account was accessed.

This checkpoint does not certify production imports: the real-provider probe and
running-app check required by import-validation remain outstanding. No remote CI,
native E2E, playable-media or catalog publication claim is made. Next: protected
snapshot persistence/replay, followed by lossless adapters and app-level validation.

## Completed local increment: protected snapshot files

Goal: retain raw evidence and capture receipt metadata under authenticated
encryption with bounded replay and immutable commits. Scope is an unconsumed
Unix storage module; no SQLite migrations, importer edits or user-profile access.
Research and format choices are in [ADR 0004](../adr/0004-protected-snapshot-files.md).
Required validation: failed-first contract tests, tamper/ownership/quota checks,
concurrent commits, restart after abrupt exits at commit boundaries, full Rust
regression, Clippy/rustfmt and manifest checks. App E2E is an integration gate.

Implemented on `feat/protected-snapshot-store`: caller-supplied profile key,
AES-256-GCM with fresh per-object HKDF subkey, encrypted metadata/raw bytes,
private staging and no-clobber commit, authenticated bounded replay. Eight
integration tests and two abrupt-exit subprocess tests pass in the storage crate.
Full workspace: 194 Rust tests pass; workspace Clippy with warnings denied,
rustfmt, seven environment tests and manifest verification pass. Frontend code
is unchanged; its nine tests/typecheck/lint passed at the preceding checkpoint
and were not rerun for this Rust-only slice. The subprocess helper is intentionally
ignored by direct test discovery and invoked by the boundary tests.

No external package upgrades or Python environment. Rollback removes the new
module and dependency edges without changing databases. Keychain key lifecycle,
streaming beyond 16 MiB, aggregate quotas, cleanup leases, normalized records and
app import wiring remain separate required work. No production security or E2E
certification is claimed for this prototype.

## Completed local contract increment: profile snapshot-key lifecycle

Goal: separate key creation from loading, prevent replacement or silent recovery,
and implement the native macOS boundary. Scope: an unconsumed storage module and
already-locked native dependencies; no profile/schema/import changes. Research,
failure behavior and recovery limits: [ADR 0005](../adr/0005-profile-snapshot-keys.md).

Implemented on `feat/profile-snapshot-keys`: random 32-byte key provisioning via
insert-only backend, read-only loading with distinct missing/invalid/access/signing
errors, and native data-protection Keychain operations on blocking workers. Six
backend tests plus one OSStatus classification test pass after a failed-first test
compile. The native roundtrip was actually attempted: the absent-item read passed
and provisioning failed with MissingEntitlement. Native validation is still blocked;
the regular suite intentionally does not execute that environment-dependent test.
Do not change signing identities or relax storage protection as a test workaround.

Regular workspace regression: 201 Rust tests pass; Clippy with warnings denied,
rustfmt, seven environment tests and manifest verification pass. Two entries are
ignored by ordinary discovery: the crash subprocess helper (executed by its parent
tests) and native Keychain probe (explicitly attempted and failed as recorded above).
Frontend code is unchanged; prior frontend checks were not rerun in this Rust slice.
Rollback removes the unconsumed module and
dependency edges; no production key or user data changed. Next independent work:
streaming and aggregate retention. Signed-app validation and durable profile key
provisioning/recovery remain required before import wiring; rotation/deletion are
not implemented by this increment.

## Completed local increment: streaming snapshot quotas

Goal: stream encrypted evidence beyond the v1 memory ceiling, enforce conservative
storage admission, and preserve existing snapshots through failures. Scope is an
unconsumed v2 storage module; no importer/schema/user-data changes. Research and
format: [ADR 0006](../adr/0006-streaming-snapshot-quotas.md). V1 remains readable.

Implemented on `feat/streaming-snapshot-quotas`: fixed 64 KiB authenticated frames,
mandatory final receipt, one-slot worker channels, OS lock coordination and logical
byte/file quotas that charge interrupted staging. Quota exhaustion rejects writes;
there is no automatic eviction before catalog references/leases exist. Replay output
is provisional until final verification and must be staged by consumers.

Nine integration tests and two new subprocess interruption cases pass in storage,
including a 17.0625 MiB stream, tamper/truncation/order checks, cross-profile/key/ID
isolation, cancellation, quota races, orphans and commit visibility. Full workspace:
212 Rust tests pass; Clippy with warnings denied, rustfmt, seven environment tests
and manifest verification pass. Ordinary discovery excludes the same subprocess
helper (run by parent tests) and native Keychain test. Frontend is unchanged and
its preceding checks were not rerun in this Rust-only slice.
No dependency upgrades or Python environment. Signed-app
Keychain validation is still blocked by the previously observed MissingEntitlement;
the native probe was not rerun without a signing change.

Rollback removes the unconsumed v2 module/API additions without migrating/deleting
v1 evidence. Next: protected record staging and lossless adapters; reference-aware
retention; signed-app key lifecycle and isolated-profile app E2E. No automatic
deletion, physical power-loss, production security or RSS benchmark claim is made.

## Completed local increment: protected record staging

Goal: preserve raw object evidence with bounded normalized records and persist an
immutable derived archive. Scope: new unconnected nova-catalog-ingest crate plus an
explicit v2 writer-abort acknowledgement. Existing provider imports stay unchanged.
Research/design: [ADR 0007](../adr/0007-protected-record-staging.md).

Implemented on `feat/protected-record-staging`: explicit Xtream live/VOD/series
object scopes, exact raw JSON retention, bounded/ambiguous-input checks, opaque
caller-owned record/locator IDs, encrypted archives bound to raw snapshot receipts,
duplicate occurrence rejection and validated replay framing. Duplicate upstream IDs
remain separate evidence. Archive completeness is distinct from provider completeness;
callbacks remain provisional until final replay succeeds.

Seven integration tests plus two framing tests pass after the contract tests first
failed against the missing API. Full workspace: 221 Rust tests pass; workspace
Clippy with warnings denied, rustfmt, seven environment tests and manifest
verification pass. The desktop normal dependency graph excludes nova-catalog-ingest.
The same two discovery exclusions remain: the subprocess helper (run by parent
tests) and the native Keychain probe (previously failed signing validation).
Frontend is unchanged; prior frontend checks were not rerun for this Rust-only slice.
Cargo.lock adds only the local crate; no
external package upgrade or production importer/profile/schema change. No Python
environment, real provider or credentials used. No app E2E is claimed.

Next: bounded array framing with offsets and raw/staged evidence coordination,
followed by episode/M3U adapters and isolated-profile app validation. Keychain signing,
reference-aware cleanup and publication remain gates. Rollback removes the unconsumed
crate and abort addition without a data migration.

## Completed local increment: catalog intelligence evidence boundary

Revised requirement: catalog understanding is a required subsystem, with model
selection benchmark-driven. The updated blueprint replaces its previous optional
LLM positioning. [Baseline](intelligence-baseline.md) and
[ADR 0008](../adr/0008-catalog-intelligence-evidence.md) record scope and decisions.

Implemented on `feat/catalog-intelligence-evidence`: unconsumed strict proposal
validation in nova-core, typed taxonomy/quality/entity fields, caller-owned code
registries and evidence, per-fact expiry, deterministic authority/conflict decisions,
provisional inference, record/snapshot binding and bounded responses. Unknown or
fabricated citations, suppressed conflicting assertions and model-supplied authority
are rejected. No new title-cleaning rules or model/runtime dependencies were added.

Validation: all 230 Rust tests pass, including nine new intelligence tests. Workspace
Clippy with warnings denied, rustfmt, seven tooling tests, manifest verification and
diff whitespace checks pass. Two ordinary-discovery exclusions remain: the crash
helper invoked by parent tests and the native Keychain probe previously failing
MissingEntitlement. No frontend changes or fresh frontend checks; no app E2E,
model-quality benchmark, real-provider validation or production readiness claim.

Next: reviewed multilingual evaluation corpus, replaceable reasoner/matcher and
safe contextual input, cache-first orchestration and persistent knowledge/research
cache. Raw framing, remaining adapters, signed key lifecycle, canonical identity
and atomic publication remain prerequisites to app rollout. No active catalog LLM
has been chosen or installed; the serving import pipeline still uses legacy logic.
Rollback removes the unconsumed module without migrations or user-data changes.

## Completed local increment: model evaluation harness and first real inference

The user permits any suitable open model size subject to app constraints. The
[model plan](catalog-model-evaluation.md) compares Qwen3.5 4B/9B and Ministral 3
8B, with separate embedding/reranking comparisons. This increment introduces a
development-only local runtime adapter and 12 synthetic multilingual cases; it
does not connect model proposals to production imports or trust them as evidence.

On `feat/catalog-model-evaluation`, added bounded loopback inference, artifact
hash verification, strict output-shape/candidate checks, failure/abstention/latency
metrics, request/fixture hashes, exclusive reports and seven regression tests.
CI runs these without model downloads. Native llama.cpp and the 4B quantization
were downloaded under ignored models/, verified and actually executed. No Python
environment or production dependency changes.

First real Qwen3.5-4B Q4_K_M run: 4/12 fully correct, zero schema failures, three
unsafe identity proposals, p50 1222 ms/p95 1618 ms. Its exact configuration fails
the smoke gate. [Report](../evaluations/catalog-smoke-v1.md) preserves the failure.
The 4B runtime was stopped before preparing the pinned 9B comparison. No production
default selected, no inference results written to user data.

Checks: seven new harness tests, seven tooling tests, nine frontend tests,
typecheck, lint, manifest and formatting/whitespace checks pass. Rust code is
unchanged; the preceding 230-test Rust baseline was not rerun. App E2E,
concurrent playback/captions, memory/energy and held-out quality remain untested.

## Completed local comparison: Qwen 4B/9B and prompt diagnosis

After the harness checkpoint, ran the unchanged v1 prompt on the verified 9B
quantization: 2/12 correct with three unsafe links. Inspected the runtime-rendered
synthetic prompt, then introduced explicit prompt versioning without replacing v1.
The diagnostic v2 prompt scored 11/12 with one unsafe link on 9B, and 10/12 with
zero unsafe links but two missed identities on 4B. Four raw reports, exact hashes
and the development-tuning caveat are retained in the
[comparison report](../evaluations/catalog-smoke-v1.md).

Eight harness tests pass after adding version-preservation coverage; formatting
and whitespace checks pass. The prior tooling/frontend checks still apply; no
Rust/frontend implementation changed in this comparison. Both native runtimes
were stopped. No model is production-selected; Ministral, held-out corpus review,
resource/playback coexistence and the Rust inference bridge remain next.

## Completed local increment: Ministral and frozen transfer comparison

On `feat/catalog-ministral-comparison`, pinned and verified Ministral 3 8B
Q4_K_M, ran both preserved prompts and a second synthetic corpus. The 12-case
transfer corpus was committed at `2ed003c` before inference; its title families
are disjoint from the development smoke set, but independent review is still
pending. Both Qwen models also ran the frozen transfer set with unchanged v2.

Combined v2 results across 24 cases: Qwen 4B 21 correct/zero unsafe links/three
missed identities; Qwen 9B 22 correct/two unsafe links; Ministral 17 correct/four
unsafe links/two missed identities/one candidate-name contract failure. These are
development observations, not statistically representative accuracy estimates.
Raw reports preserve all outputs and hashes. Ministral's transfer failures include
following an instruction embedded in provider metadata.

Selected Qwen3.5-4B Q4_K_M / prompt v2 as the **development reference** for the
next Rust reasoner and provisional-evidence bridge. Added `eval:catalog:reference`
as an explicit development command. No serving-app model, production default,
identity merge or profile data change. All started runtimes were stopped; model
artifacts remain ignored on disk. No Python environment or dependency upgrades.

Validation: nine harness tests, seven tooling tests, formatting, manifest checks,
diff whitespace and corpus/prompt/schema hash consistency across all nine reports
pass. Rust/frontend implementation is unchanged and those suites were not rerun;
the prior 230 Rust/nine frontend baselines remain historical evidence. Next:
bounded Rust adapter, independent corpus review, realistic input volume, cancellation,
memory and playback/caption coexistence, then cache/source integration.

## Remaining integration gates (unchanged)

- Remote CI/protection/merge unavailable without GitHub authentication.
- No deterministic native E2E harness yet.
- Native dependency paths depend on Homebrew; stale build-script cache reproduced.
- Named Rust toolchain installation and clean bootstrap not exercised in this session.
- Models/native SDK hashes, linked SQLite status and license gates remain open.

## Rollback

Revert the environment commit to restore pre-migration tooling; no user data is
modified. v0.1.0 remains immutable. No schemas or compatibility adapters introduced.
