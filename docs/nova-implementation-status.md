# Nova implementation status — P00 stabilization and M1 architecture audit

Updated: 2026-09-25. P00 stabilized the existing application; M1 below audits its
source against the target plan. No architecture implementation, UI redesign,
dependency upgrade, or model change was made during the audit. P00 started from a
clean worktree; this audit preserves its uncommitted changes and updates the
user-provided `master-plan.md` only for evidenced dependency/order adjustments.

## Validation map and baseline

The pnpm/Cargo workspace is driven by `just`. The shipping application is React 19 /
TypeScript in `apps/desktop/src` and Tauri in `apps/desktop/src-tauri`. Rust libraries
cover provider parsing, SQLite repositories, jobs, search/enrichment, media processes,
mpv, captions, and translation. CI uses SQLx offline metadata, macOS Rust checks,
and separate frontend/tooling checks.

Reviewed the task runner, manifests/locks, CI, Tauri initialization/IPC/state,
profile migrations and repositories, provider import/refresh transaction, catalog
and transcript search, job claim/drain, native player lifecycle, HLS/media process
ownership, caption/translation managers/hooks, settings, and browse/query tests.
Existing snapshot/reasoner prototypes were exercised by their existing tests;
P00 does not advance or integrate them into the application.

Baseline before fixes:

- `just check`, `just lint`, `just typecheck`, and `cargo fmt --all -- --check`: pass.
- Frontend: 14 tests pass; `pnpm -C apps/desktop build` passes, with an existing
  Vite warning about the >500 kB JavaScript chunk.
- `just test`: 249 Rust tests/doctests pass, two existing ignored tests, and 14
  frontend tests pass when synthetic localhost servers can bind.
- Node environment/catalog-eval/catalog-scale tests: 19 pass with localhost access.
- Initial sandboxed Rust/Node integration runs failed with `EPERM` binding
  `127.0.0.1`. These were environment restrictions, not product failures; reruns
  with loopback permission passed without disabling tests.
- `pnpm verify-environment --manifest-only`: consistent. This checks committed
  versions and migration heads, not installed AI models or signed distribution.

## Confirmed bugs fixed

| Area | Defect and narrow correction | Regression evidence |
| --- | --- | --- |
| Import refresh | A successful subset plus a provider error/timeout committed replacement rows and discarded the old catalog. Replacement imports now fail and roll back on any producer error. Initial-import partial-result behavior remains unchanged. | `failed_refresh_preserves_catalog_flags_and_resume` failed before the guard; now checks terminal failure, unchanged catalog/favorite/resume, and successful subsequent refresh. |
| Catalog/user state | Deleted source/media rows left embeddings and watch history attached to reusable numeric IDs. Migration 0020 cleans identifiable orphan history, invalidates ambiguous movie/live vectors, resets enrichment eligibility, and adds deletion triggers. | Storage integration tests failed with orphan vectors before migration; now cover source cascades and reused IDs. |
| Transcript identity | Refresh could reuse an ID belonging to another cached transcript. Deletion triggers detach numeric transcript references while retaining content hashes, segments, windows and translations; stable-key playback can reattach them. | `reused_catalog_id_does_not_relabel_an_old_transcript` checks distinct new content and preservation of the old cache. |
| Migration builds | Adding a migration was not noticed by an incremental Cargo build: the first new migration tests still ran the old embedded directory. Storage now emits `rerun-if-changed=migrations` from `build.rs`. | Tests pass after a real rebuild and cover upgrades from migration 9 through 19 to 20, foreign-key integrity, FTS backfill, and reopening. |
| Semantic search | The in-memory vector cache was keyed only by source ID/kind and invalidated only by count. Equal IDs across profiles and equal-count refreshes reused unrelated vectors. The unsafe cache is removed; each search loads its caller's persisted vectors. | The cross-profile/equal-count regression failed against the original implementation and now passes. |
| Jobs | A kick arriving after the drain's final empty claim but before unlock was dropped. Kicks now wait for the drain lock and check again. | A deterministic lock-held regression reproduced a stranded queued job and now drains it. |
| Settings/caption style | Closing a panel discarded the debounced write; a late initial read overwrote edits; changing profiles could mix values; concurrent panels could overwrite unrelated settings. Writes capture their original profile, flush on cleanup, serialize, and merge only pending edits into the latest persisted value. | Seven hook tests cover close, profile change, slow read, and two mounted settings panels. Lost edits and stale-panel overwrites were reproduced before their fixes. |
| mpv lifecycle | Late opens could keep playback alive after close or overtake newer media requests. Native open/close operations are ordered and cancelled opens close their returned token. Old state is ignored during a new open. A rejected native load retains its old valid token for cleanup. | Two frontend lifecycle tests cover late open/close/switch order, cancelled queued opens, and recovery after failure. The isolated macOS smoke test also covers native rendering, pause, seek, close/reopen and resume. |
| AirPlay lifecycle | A handoff resolving after close leaked the newly created proxy session. Ordered handoff/teardown plus a generation guard now release it. Profile changes and disabled state also tear down. | Deferred-handoff hook test failed before the change and now verifies release without engaging the closed player. |
| Caption/translation lifecycle | Model downloads could restart disabled sessions; old-track/target events shared the same media IDs; cached cues could arrive before listeners. Starts/stops are ordered, late callbacks are guarded, listeners are awaited, and cues/status echo a per-activation session ID. | Four hook tests cover late downloads and stale session cues for both engines. Existing Rust caption stitching/language/cache tests remain enabled. |
| FFmpeg/ffprobe | Timeout dropped the probe future without killing the child. Remux stderr was piped but never drained, so enough diagnostics blocked ffmpeg. Probes now kill on drop; unread remux diagnostics go to the null sink. | Real subprocess tests reproduced a surviving timed-out child and a 1 MiB stderr stall; both now pass. |
| Provider requests | Reserved credential characters were interpolated directly into query values/path segments. Components are now percent-encoded and parsed origins preserve valid host/port structure. Trailing garbage after a JSON array is rejected. | Xtream tests cover component round-tripping, unchanged plain credential requests, malformed/trailing JSON, and existing HTTP fixture contracts. |
| Privacy/proxy | Provider URLs/raw error bodies were emitted by M3U/EPG/HLS/media diagnostics; serde could quote invalid provider values. Diagnostics now omit these sensitive values while retaining operation/status/location. Loopback checks now cover IPv6, 127/8, mapped addresses and localhost suffixes; redirects apply the same guard. | Expanded URL-guard tests and JSON error-content tests, plus provider fixture tests. This is not a comprehensive DNS-rebinding audit. |
| UI cleanup | A resize listener that resolved after modal cleanup was never removed. Its promise now checks cancellation and immediately unsubscribes. | Typecheck/lint plus surrounding player tests; no visual redesign. |

## Migration and behavior notes

Migration 0020 is forward-only; no previous migration was edited. It resets only
rebuildable movie/live embedding data and enrichment eligibility. Semantic search
uses lexical results until enrichment repopulates vectors. Valid favorites,
watchlist and watch history are retained. Existing history already incorrectly
attached to a reused ID cannot be retrospectively identified from this schema.
Legacy transcripts without content hashes retain their existing lookup behavior;
deleted rows are detached, but historical identity ambiguity is not reconstructible.

Loading persisted catalog vectors on each semantic query trades the old cache's
speed for correct profile/content isolation. No replacement indexing architecture
was introduced. Existing legacy episode-refresh resume limitations remain; redesigning
that identity model is outside P00.

## Validation commands and results

- `just test` (with loopback permission): **259 Rust tests/doctests pass, 2 existing
  ignores; 28 frontend tests pass**. The ignored native Keychain test requires a
  signed entitlement context; the ignored crash-test helper is invoked by its
  parent tests. No tests were disabled.
- `just check`: pass, including Tauri. `just lint`: Clippy with `-D warnings` and
  ESLint pass. `just typecheck`, `cargo fmt --all -- --check`, `git diff --check`: pass.
- `node --test scripts/stack.test.mjs tools/catalog-eval/evaluate.test.mjs tools/catalog-scale/provider.test.mjs`:
  **19 pass**, including synthetic HTTP/model-adapter contracts and 100k scale fixtures.
- `pnpm verify-environment --manifest-only`: pass with migration head updated to 0020.
- `pnpm -C apps/desktop build`: pass; existing large-chunk warning remains.
- `just build`: release Rust compile and `.app` bundling succeeded. Initial DMG
  creation failed in the sandbox (`hdiutil: create failed - Device not configured`).
  `pnpm -C apps/desktop exec tauri bundle --bundles dmg --verbose` with filesystem/device
  permission subsequently produced the local DMG successfully.
- `pnpm -C apps/desktop exec tauri build --debug --no-bundle`: pass.
- Focused red/green commands included `cargo test -p nova-jobs --locked kick_during`,
  `cargo test -p nova-desktop --locked catalog_vectors_do_not`,
  `cargo test -p nova-desktop --locked stabilization_tests`,
  `cargo test -p nova-storage --test stabilization --locked`, and media process tests.
  Cargo commands used `SQLX_OFFLINE=true` (provided by `just` for its recipes).
- An intermediate concurrent Tauri-build/test run failed in rustdoc with E0463
  missing Tauri crates. A sequential full rerun passed, including doctests. Keep
  Tauri builds and workspace tests sequential when sharing the target directory.

### Native smoke and final artifacts

The final `just build` completed successfully after all code fixes, producing
`target/release/bundle/macos/Nova.app` and
`target/release/bundle/dmg/Nova_0.2.0-alpha.0_aarch64.dmg`.
This local build is not a signed/notarized release certification.

A debug bundle named `Nova P00 Smoke` used a unique bundle identifier and a generated
Info.plist `LSEnvironment` entry pointing `NOVA_VALIDATION_DIR` at
`/tmp/nova-p00-smoke-20260925`. The empty isolated database was verified before
creating a synthetic profile; existing user profiles were not modified.

Native UI and isolated SQLite checks passed:

- Create/select a profile; import a localhost M3U containing two synthetic movies;
  all import stages complete and both titles appear in Movies.
- Render the generated silent 60-second H.264/AAC fixture through native mpv;
  clock advances, Pause retains a frame, and forward seek moves from 0:06 to 0:16.
- Close/reopen: playback resumes at approximately 0:17. Close again and return to
  the library; Continue Watching and the persisted 23-second resume value agree.
- Mark a favorite and refresh the source; the two titles, favorite, and saved
  progress remain in the isolated database.
- Change autoplay and caption text size, close settings, and reopen; both persist.
- Quit the isolated app cleanly after testing.

Fixture setup used `ffmpeg` testsrc2/anullsrc and
`python3 -m http.server 18973 --bind 127.0.0.1 --directory /tmp/nova-p00-media`.
The first fixture had its MP4 metadata at the end; Python's simple HTTP server
has no byte-range support, yielding duration but no playable frames. Remuxing the
fixture with `ffmpeg -i fixture.mp4 -c copy -movflags +faststart faststart.mp4`
resolved this test-fixture limitation without a product-code change. Playback
of the streamable fixture was then verified visually and through the UI clock.

Additional native build commands:

```sh
pnpm -C apps/desktop exec tauri build --debug --no-bundle
pnpm -C apps/desktop exec tauri bundle --debug --bundles app \
  --config '{"productName":"Nova P00 Smoke","identifier":"app.nova.p00smoke"}'
just build
```

Final run logs are local temporary artifacts: `/tmp/nova-p00-final-tests.log`,
`/tmp/nova-p00-release-final.log`, and `/tmp/nova-p00-debug-final.log`.


## Unresolved validation gates

- Real-provider import/refresh remains unqualified: the authorized follow-up
  below exercised the actual probe but received HTTP 403 independently of Nova.
  The user subsequently confirmed the same result outside Nova; no pipeline
  regression was demonstrated. Credentials and authenticated URLs were not
  emitted or committed. Synthetic importer tests do not prove provider access.
- Actual whisper/NLLB/embedding inference, model downloads/artifact provenance,
  caption quality, long media runs, sleep/wake, physical AirPlay output, and
  cross-platform native rendering are not certified by these hermetic tests.
- Signed/notarized distribution, clean-machine dependency closure, native Keychain
  entitlements, and physical-device testing remain separate release gates.

## Follow-up: reported HTTP 403 during import

The user reported that a `get.php` M3U link with `type=m3u_plus&output=hls`
worked in Nova before P00 but now fails during import. The actual saved link was
read only from the local profile database for this authorized investigation;
neither its credentials nor authenticated URLs were printed or written to the
repository. The example pasted in chat was anonymized.

Observed on 2026-09-25:

- The newest failed source and last successfully imported source have matching
  scheme/host/port/path but different username/password values.
- The current credential encoding produces unchanged request bytes for the
  failing source. P00 did not change the HTTP user agent, headers, endpoint
  routing, or transport for this case.
- Direct GET of the original M3U returns HTTP 403 through system curl and Python's
  HTTP client, independently of Nova's parser and database.
- Standard browser/player user agents also return 403. Both IPv4 and IPv6 curl
  requests return 403. There are no proxy settings reported by Python's client.
- The previously successful saved account also now returns 403 for both its
  original M3U link and the derived API endpoint.
- The actual `nova-xtream` headless probe (`NOVA_PROBE_URL` supplied in a child
  process environment; `RUST_LOG=error`) exits 1: zero entries and HTTP 403 on all
  three endpoints (live, VOD, series).

The focused rerun `SQLX_OFFLINE=true cargo test -p nova-xtream -p nova-m3u --locked`
passes all 30 tests.

This is an unresolved real-provider validation failure. A parser or migration
change cannot explain the direct M3U rejection observed here. No speculative
transport/fallback change has been applied or claimed to fix the reported 403.
The user later confirmed the same result in their own test. Successful real-provider
import validation still requires restored access; this is not evidence of a parser
or persistence regression.

### Confirmed profile follow-up

After the user clarified that the shared URL was a template and identified the
affected profile, the investigation selected that exact profile by name from the
app database and read its saved source. This selection supersedes the earlier
assumption that the most recently modified profile was the affected profile.

For the confirmed profile:

- The original saved M3U GET returns 403 through Python and system curl.
- The real headless probe exits 1 with zero entries and three HTTP 403 errors.
- The pre-P00 and current API request strings are byte-identical for this source;
  only the equality result was emitted, never the request strings.
- No trailing comma, HTML-escaped separator, or credential-encoding difference
  is present in the saved URL.
- The server identifies as OpenResty and returns 403 for the account endpoint and
  its root path as well. The exact reason for this refusal is not established.

There is still no demonstrated pipeline defect that explains this source's 403.
No product code was changed during this diagnosis. A current successful download
of the same URL, or restored provider access, is needed to continue the real
import validation. P00's earlier synthetic validation does not clear this blocker.

### Live provider revalidation — 2026-09-25

A newly supplied credential-bearing M3U link was checked without storing its URL
in the repository or printing its credentials. Its direct GET returned HTTP 200
with an M3U header. Nova routed it as Xtream and the headless probe completed
cleanly in 6.7 seconds: 8,667 live channels, 28,591 movies, and 5,299 series
(42,557 entries total), with zero errors.

An opt-in test now exercises the same fetch and `run_import` path against a
temporary migrated profile database. It passed for the supplied link: 42,557
entries parsed and 42,557 rows committed, with zero import errors. The test uses
`NOVA_PROBE_URL` from the process environment and deletes its temporary database;
the credential-bearing URL is absent from source, fixtures, and test output.
Validation commands: `cargo fmt --all --check`, the `nova-xtream` headless probe,
and `SQLX_OFFLINE=true cargo test -p nova-desktop --lib
real_provider_import_persists_catalog --locked -- --ignored --nocapture` with the
URL supplied only through `NOVA_PROBE_URL`. The ordinary
`SQLX_OFFLINE=true cargo test -p nova-desktop -p nova-xtream -p nova-m3u --locked`
run also passed (52 tests; the opt-in live test remains ignored by default).

This establishes that the current fetch, parser, and SQLite import path works for
this reachable provider account. It does not establish why the previously saved
profile account returned HTTP 403, or certify Tauri UI and playback behavior.

## M1 source-aware architecture audit — 2026-09-25

**Outcome:** M1.1 and M1.2 are complete as a source audit and dependency plan.
No application code, schema, model, or runtime was changed by this audit. The
P00 changes and audit are committed on a clean local branch; this is not full
release qualification. The next implementation milestone has not begun.

### Evidence and architecture map

The target is `master-plan.md`, read alongside `ARCHITECTURE.md`,
`docs/nova-production-architecture-blueprint.md`,
`docs/nova-100k-media-intelligence-architecture.md`, `docs/catalog.md`, and the
increment history in `docs/implementation/progress.md`. The production blueprint
supersedes the older player plan. These documents describe intent; the following
map describes executable source. Older comments about generated bindings,
streaming EPG downloads, model-free loading, or completed phases are not proof
of those properties.

The current serving path is:

```text
React invoke -> desktop commands -> per-profile SQLx repositories
M3U / Xtream -> ParsedEntry -> long import transaction -> numeric media rows
numeric media row -> PlaybackRepository -> mpv, or HLS proxy / FFmpeg / AirPlay
React player -> caption/translation session managers -> media/ASR/CT2 -> SQLite
React search -> LIKE / transcript FTS + optional E5 vectors -> legacy media DTOs
```

A separate, tested foundation exists:

```text
core catalog IDs / raw records / fingerprints / scope receipts
  -> protected snapshot files + encrypted record staging
  -> bounded local reasoner + validated, expiring protected derivations
```

The desktop does not depend on `nova-catalog-ingest` or `nova-catalog-reasoner`
and does not invoke those contracts. `nova-jobs` is a desktop dependency but its
queue is not instantiated by desktop workers. These foundations must be integrated
incrementally, not recreated. A passing library test is not a production cutover.

| Subsystem and source evidence | Reuse / existing behavior | Partial, missing, or conflicting target behavior |
| --- | --- | --- |
| Provider ingestion — `crates/m3u/src/lib.rs`, `crates/xtream/src/lib.rs`, desktop `commands/mod.rs::{start_import,run_import}` | Streaming producers, bounded channels, URL detection, credential-component encoding, parser diagnostics, batched inserts and atomic refresh rollback. Xtream-style M3U links use the detected API path. | `ParsedEntry` combines provider parsing and serving fields. No protected capture or compiler boundary in the app. Replace deletes/reinserts rows while its transaction remains open across producer work. No durable import cancellation, per-scope carry-forward, generation check or per-source job admission. |
| Catalog / identity — `crates/core/src/{lib.rs,catalog.rs}`, storage repositories | Typed, validated opaque source/record/entity/edition/variant IDs and raw-record bounds already exist. Existing catalog repositories and browse queries work. | Serving still uses integer channel/VOD/episode rows; series identity is `(source_id, series_title)`. No persisted canonical entities, editions, bindings, aliases, merge/split ledger or tombstones. `canonical_title` is TMDB display text, not canonical identity. |
| Fingerprints — `core/src/catalog/fingerprint.rs`, `core/tests/catalog_snapshots.rs` | Versioned framed SHA-256, independent test vectors, metadata/locator separation, credential revision input and deterministic attributes. | No persisted unchanged/changed/new/deleted planner, compiler-input revision, binding reuse or model/artifact invalidation integration. Hashes do not establish media identity or encrypt locators. |
| Snapshots — `core/src/catalog/snapshot.rs`, `storage/src/snapshots{.rs,/stream.rs}`, `catalog-ingest/src/stage.rs` | Complete/partial/failed/cancelled receipts, scope fallback, authenticated protected files, bounded streaming frames, quotas/locks/atomic file commit and exact raw-record replay. | Raw snapshot completeness is not canonical publication. Missing application wiring, network array framing/offsets, M3U and episode staging adapters, durable receipt index/retention, canonical generation membership, head CAS and publication outbox. Raw and derived stores need separate namespaces where locking requires it. |
| Entity resolution / reasoning — `core/src/catalog/intelligence.rs`, `catalog-reasoner/src/{lib.rs,protected.rs}`, `enrich/src/dedup.rs` | Bounded candidate-only local requests, strict schema/contradiction checks, evidence authority/expiry, abstention and protected context-bound cache. Legacy exact-key buckets with a cosine guard are reusable heuristics. | Caller must construct vetted context and candidates; no production blocking index, stable resolution ledger, overrides, verified knowledge store or history. Reasoner has no authority to write identities. Declared model/runtime digests are not artifact attestation. Legacy clusters are not canonical entities. |
| TMDB — `enrich/src/tmdb.rs`, desktop `commands/enrich.rs`, storage `tmdb_cache.rs`, migrations 0014 | Title/year search, scored matches, weak-art-only threshold, movie/series display enrichment and raw-response cache. | Missing trusted-ID Find, complete details/credits/people/season/episode graph, confidence/provenance history and tiered refresh. Cache stores timestamps without read-time expiry. Current enrichment entry point requires an embedding marker even for TMDB. |
| Search — `search/src/lib.rs`, desktop `commands/search.rs`, storage migrations 0017–0019 | Safe transcript FTS query construction, lexical catalog reads, E5 query/document prefixes, cosine scoring and RRF; profile-safe vector reads after P00. | No canonical FTS projection, metadata/people/alias indexing, exact-match protection or production relevance corpus. Catalog search loads all persisted vectors per query; transcript search may build missing embeddings synchronously. Missing/load-failed models permit lexical fallback, but later inference/index errors can fail the whole request. |
| Storage / migrations — `storage/src/db.rs`, `migrations/{app,profile}`, repositories | Separate app/profile pools; WAL, foreign keys, SQLx migrations/offline metadata, transactional user state, transcript windows/FTS and vector stores. Profile head is 0020. | No canonical-generation schema or model registry. `set_ignore_missing(true)` accepts migrations unknown to an older binary: explicit compatibility policy is required before cutover. Current row deletion semantics remove/rebind state, rather than retaining canonical history. |
| Jobs — `jobs/src/lib.rs`, `storage/src/repositories/jobs.rs` | Persisted queued/running/done/failed states, priority, attempts, atomic claim and kick/drain serialization including P00 lost-kick test. | Desktop imports/enrichment/EPG/AI use direct spawned tasks. No startup recovery, leases/heartbeats, bounded admission, cancellation propagation, retry budget/backoff or cross-subsystem priority policy. A mutex per queue instance is not global resource governance. |
| Transcription — desktop `caption_session.rs`, `commands/transcription.rs`, `crates/transcribe`, `media/src/audio.rs` | Local whisper CLI, track selection, approximately 30-second FFmpeg windows, word times, stitching/coverage, persisted segments, seek-aware requests and session-tagged events. | Not playback-fed stable PCM/VAD streaming; extraction rereads the upstream stream. Cache identity hashes the exact URL plus track/model, not canonical audio/timeline revisions. Cancellation marks the old session but does not join all extraction/inference work before starting the next. |
| Translation — desktop `translation_session.rs`, `commands/translation.rs`, `crates/translate`, migration 0016 | Actual local CT2 CPU/SentencePiece adapter, language mapping, bounded translation batches and persistent segment translations, session-tagged UI events. | No shared model runtime/leases; model load and batches can outlive cancellation. Cache key lacks artifact/tokenizer/options revisions. No cloud adapters or CT2 Metal implementation should be credited from design documents. |
| Embeddings / model infrastructure — `transcribe/src/model.rs`, `translate/src/model.rs`, `enrich/src/embed.rs`, desktop `state.rs` | Existing stable model names, path helpers, download flows and `Embedder` abstraction; optional semantic embedder in app state. | Fixed internal folders, file-existence/marker readiness, mutable download revisions, no verified registry, removable libraries/bookmarks, availability ledger, runtime unload or leases. Separate enrichment/search loads can duplicate model memory. `FastEmbedder::load(..., false)` hides progress; it does not prohibit downloads. |
| mpv playback — `mpv-player/src/lib.rs`, desktop `commands/playback.rs`, native surface/AirPlay code, `PlayerModal.tsx` | Real embedded libmpv/OpenGL rendering, reusable engine session, token-checked controls, tracks/seek/pause, HLS proxy/FFmpeg ownership and AirPlay bridge. P00 orders frontend opens/closes and cleans late opens. | No Rust owner of complete playback intent, variant selection, bounded recovery, QoE or autoplay. State reads intentionally return the current token even for a stale request. Rust must eventually own the policy without replacing the working engine/render path. |
| Tauri APIs — desktop `lib.rs`, `ipc.rs`, `commands/*`, `docs/ipc-contract.md` | Registered commands, structured error envelope, Rust DB access, native playback URL resolution and session-tagged caption events. | Serving DTOs serialize source URLs and stream URLs; profile settings return the TMDB key. Types are manually duplicated in React; `packages/types/src/index.ts` is a placeholder, not an active generated contract. Import/enrichment/EPG events lack profile/run identity. |
| React contracts — `src/types/index.ts`, browse `queries.ts`, `Home.tsx`, `PlayerModal.tsx`, `useProfilePreference.ts` | Query keys include profile/source, virtualization, optimistic user actions, stale-response guards, serialized profile preference persistence and useful browse components. | Page-local movie title normalization/collapse invents variant groups and strips year; series collapse invents presentation identity. Resume sampling/persistence, end handling and autoplay depend on the mounted player. Canonical DTOs must replace these decisions gradually. Filters, modal state and selection properly remain transient React state. |
| Live / EPG — `crates/epg`, desktop `commands/epg.rs`, storage `programs.rs`, migration 0005 | XMLTV parser, provider/community feed registration, batched transactional writes, now/next and range queries. | Feed is downloaded into bytes and parsed into a complete Vec despite “streaming” comments. Program identity is profile-wide `channel_tvg_id`, without source/canonical binding; overlapping feeds can overwrite each other. No arbitration/canonical-channel mapping or bounded end-to-end ingest. |

### Ownership and provider leakage

Durable favorites, watchlist, resume, settings, transcripts and translations are
already persisted in Rust/SQLite. They should not be described as entirely
React-owned. The problematic ownership is **decision authority**: React decides
movie/series equivalence, chooses variants, initiates/resumes playback, detects
completion/autoplay and decides when to write watch progress. UI teardown or
refresh can therefore affect durable behavior. Preserve the working persistence
repositories while moving those decisions behind canonical commands.

`Source.url`, `Channel.stream_url`, `VodItem.stream_url`, and
`SeriesEpisode.stream_url` in `core/src/lib.rs` cross IPC into equivalent React
interfaces. `ProfileSettings.tmdb_api_key` crosses that boundary too. Browse keys,
user actions, search hits and playback commands refer to provider-row IDs or series
titles. The new raw `SourceRecord` deliberately avoids unrestricted serialization
and redacts debug output, but does not protect the legacy serving path. Keep
credential-bearing source setup inputs narrowly scoped; use secret-free read DTOs
and opaque server-resolved references for output. Do not expose an alternate set
of provider-flavored frontend types as a migration shortcut.

### Significant coupling and failure risks

These are source-established gaps/risk paths, not newly reproduced runtime bugs.
No claim is made that the following scenarios were all exercised during P00.

1. **Refresh versus asynchronous writes.** `commands/enrich.rs` reads rows, computes
   embeddings/TMDB results later, then writes by numeric ID without a source
   generation predicate. Delete/reinsert refresh can invalidate those IDs. Migration
   0020 removes old persisted vectors but cannot reject an in-flight stale write.
   Concurrent imports also have no explicit latest-request-wins generation. A
   source revision check and cancellation are prerequisites for compiler workers.
2. **Writer contention and ambiguous state continuity.** `run_import` holds a
   transaction across network-fed receiving. Refresh restores movie/live flags and
   resume by exact URL, series flags by title; credential rotation/title changes
   and episode continuity are not solved. Atomic rollback is valuable but is not
   the target short publication transaction or stable canonical identity.
3. **Cross-profile events.** Import/enrichment/EPG progress identifies numeric
   source IDs, reused across profile databases. `Home.tsx` filters import events
   only by source ID and can react to another profile's outstanding worker. Add
   profile, operation and revision identity before wiring shared background jobs.
4. **Artifact path collision.** `commands/transcription.rs::transcripts_dir` is
   app-global; `caption_session.rs::transcript_basename` uses kind, numeric media
   ID, track and language, without profile/content/model identity. Separate
   profile DBs do not prevent VTT/JSON path collisions or reused-ID overwrites.
   SQLite segments remain separately stored; file artifact ownership needs an
   explicit migration and a focused regression before reliance on those exports.
5. **Secret exposure beyond P00's redaction fixes.** Legacy IPC still returns
   authenticated URLs and the TMDB key. `tmdb.rs::get` appends the key to the request
   query and uses `reqwest::Error::to_string`; enrichment forwards/debug-logs errors.
   Request errors can retain that URL. Tauri currently configures `csp: null`.
   No exploit was attempted; redact errors and qualify secret-free contracts/CSP
   with remote artwork and loopback playback before new webview integration.
6. **AI optionality and stale derivations.** `start_enrichment` returns before TMDB
   when the embedding marker is absent. Series embedding precedes TMDB overview
   updates and existing vectors are keyed by model name rather than document
   revision. Search's successful model-load path propagates inference errors,
   losing lexical results. Tests must cover absent, corrupt and failing models,
   plus metadata changes, before declaring optional AI fulfilled.
7. **Cancellation is not resource reclamation.** Caption and translation managers
   cancel by flag; running processes/`spawn_blocking` work may continue while the
   next activation starts. P00 prevents stale UI callbacks, but does not establish
   runtime leases, bounded overlap or global admission. Untokenized caption stop/
   playhead commands still depend on frontend ordering; mpv control tokens are a
   stronger boundary to reuse.
8. **Scale limits are structural.** EPG buffers bodies/programs; enrichment retains
   whole-source row/text/vector collections; search reads/scans whole vector sets.
   SQL pagination plus React page-local collapse can produce inconsistent variant
   groups/counts across pages. Protected staging is bounded by quotas but tracks
   occurrence IDs in memory; its limits are not unlimited constant-memory claims.
9. **Recovery and compatibility are incomplete.** Persisted running jobs have no
   desktop startup recovery; snapshot locks/quotas do not supply a durable head or
   GC policy; permissive migration loading needs an old-binary compatibility gate.
   Signed Keychain/removable-drive/native lifecycle behavior remains unqualified.

### Existing tests: what they establish and where they stop

| Evidence | Established behavior | Not established |
| --- | --- | --- |
| `core/tests/catalog_contract.rs`, `catalog_snapshots.rs` | Typed ID separation, raw text/size bounds, scope completeness/fallback and fingerprint vectors. | Stable identities surviving a real app refresh, published catalog generations. |
| `catalog-ingest` tests; storage `protected_snapshots.rs`, `streaming_snapshots.rs` | Object parsing/staging/replay, quotas, atomic commits, concurrent writers, corruption/truncation, wrong keys/profile, symlinks and crash/lock behavior. | Desktop capture wiring, retention policy and canonical publication. |
| `catalog-reasoner` tests; core intelligence tests | Candidate-only validation, contradictions, abstention, bounded local HTTP, timeout/cancel/busy behavior, context-bound protected cache. | Production model quality, vetted context construction, durable resolution history. |
| `tools/iptv-fixture/tests/provider_contract.rs`, M3U/Xtream tests, desktop import stabilization test | Synthetic HTTP contracts, credentials encoding, parsing, failed replace preserves catalog/user state, successful replace still works. | A currently accessible real provider; credential-rotation identity preservation. |
| `storage/tests/stabilization.rs`, repository tests | Migration-chain cleanup, deletion triggers, URL-hash transcript isolation, refresh/user-state and FTS/vector persistence cases. | Canonical backfill, split/merge semantics, old-binary compatibility after future migrations. |
| Search/enrich unit tests and desktop vector regression | FTS escaping, cosine/RRF, conservative legacy dedup, TMDB scoring and cross-profile equal-count vector isolation. | Relevance/SLO gates, AI-runtime failure fallback, canonical identity search. |
| Jobs and media process tests | Atomic claim/drain including lost-kick regression; ffprobe timeout kill, remux stderr-stall protection. | App-wide durable recovery, resource priority and cancellation of all workers. |
| Caption/translation Rust tests and frontend `captionSessions.test.tsx` | Stitching/cache/language rules and stale session/download guards. | Real multilingual inference/quality, bounded concurrent runtime ownership, profile-safe export paths. |
| Frontend `nativePlayback.test.ts`, `useAirPlay.test.tsx`, settings `persistence.test.tsx`, browse/route tests | Ordered open/close, late-handoff cleanup, preference flush/profile/merge behavior and browse interactions. | Native rendering via mocked Vitest, Rust-owned autoplay, EPG source arbitration. |
| Tooling Node tests, catalog evaluation and scale fixture generators | Environment manifest consistency and reproducible synthetic/evaluation contracts. | Measured 100k/250k/500k production app latency/RSS, production model provenance. |

The prior P00 isolated native smoke is retained above as separate evidence. It
is not a deterministic native E2E suite. Native Keychain entitlement qualification
remains an existing ignored test; the crash helper's ignore is intentional and its
parent tests invoke it. No additional tests are disabled by this audit.

### Likely additive migrations and compatibility sequence

No migration is introduced here. Final table/column choices belong to each bounded
implementation milestone after its own data-compatibility test. Reuse the app DB
for shared internal model-library metadata and profile DBs for catalog/user data.

1. **Model metadata (M2.1/M2.2/M2.4):** persist logical descriptors, artifact revisions,
   libraries, relative locations and availability; map existing internal installs
   without moving files or treating existence as integrity proof. External access
   grants/bookmarks are a later M2.3 addition, not absolute paths as identities.
2. **Source capture and secrets (M3.2):** protected source/credential references,
   scope/receipt indexes, credential revision and import operation identity.
   Preserve current provider URLs privately until a verified compatibility path
   can resolve legacy playback. Reuse protected files/key lifecycle; do not copy
   raw credentials into canonical tables, model prompts or frontend DTOs.
3. **Canonical identity/publication (M3.1–M3.7):** entities, editions, bindings and
   variant references; immutable generation membership/head; fingerprint indexes,
   provenance, lineage/redirect/override history and legacy row-to-entity bridge.
   Backfill in bounded batches, validate, then atomically publish. The old app
   remains on legacy reads until the corresponding slice is verified.
4. **User-state continuity:** bridge existing favorites/watchlist/history to stable
   entities and timeline-aware editions. URL/title matches are migration evidence,
   not universal identity. Preserve ambiguous references for reconciliation; never
   assign an arbitrary split child or discard history to force a clean backfill.
5. **Jobs and derived artifacts:** extend existing jobs with operation/revision,
   cancellation/retry/lease recovery fields as needed. Version transcript/audio/
   timeline/model/tokenizer/options identity, profile-safe file ownership and
   translation/vector dependencies. Rebuild derived indexes when safe; preserve
   transcript text and user state independently from disposable vectors.
6. **Metadata/search/EPG projections:** attach TMDB facts and freshness/provenance to
   canonical entities; migrate FTS/vector projection identity and rebuildability;
   add explicit source-to-canonical EPG bindings. Keep XMLTV programs outside VOD
   identity. These require stable identity before serving cutover.

Every schema slice needs fresh-install and upgrade-chain tests, FK/integrity
checks, restart/interruption behavior, preservation assertions and explicit binary
compatibility policy. Apply ADD → MIGRATE → VERIFY → CUTOVER → REMOVE; removing
legacy serving tables/columns is not part of the first schema migration.

### Milestone assessment against master-plan.md

| Milestone | Source-aware status |
| --- | --- |
| M0.1 / M0.2 | Stabilization inspection and listed fixes complete, with P00 evidence above. Newly identified audit risks still require focused qualification; this is not certification that every current-main bug is fixed. |
| M0.3 | Qualified local baseline, not full release sign-off: real-provider access, AI/native/security gates remain explicit. Stabilization and audit commits are clean locally; no remote publication was performed. |
| M1.1 / M1.2 | Satisfied by this audit, reuse/migration map and evidence-backed dependency adjustments in master-plan §21. |
| M2.1 / M2.4 | Model names/path helpers/adapters are reusable partial foundations. Registry domain/persistence/discovery is missing. M2.2/M2.3/M2.5/M2.6 are not satisfied by fixed folders or a cached Arc. |
| M3.1–M3.3 / M3.7 | Contract/hash/raw protected capture primitives have tests. Serving separation, persisted diff planning and canonical publication are not complete. |
| M3.4–M3.6 | Legacy blocking and bounded evidence/reasoning are partial; production entity resolution and identity history are missing. |
| M4.1 / M4.2 / M4.4 | Basic title/year movie/series enrichment exists. Trusted-ID identification, rich graph, episode metadata and M4.5 provenance/refresh remain gaps. |
| M5.1–M5.5 | Existing lexical/transcript/semantic/RRF features are reusable; none establishes the entire canonical search milestone. Transcript search (M5.3) exists in the legacy model. |
| M6.1–M6.7 | Working playback mechanics and UI autoplay/live behavior, but target Rust policy/coordinator/resolution/recovery/QoE are incomplete. |
| M7.1–M7.5 | Library primitives only; no production worker governance or durable resume. |
| M8.1 / M8.3 / M8.4 | Local ASR/translation/embeddings exist. Real inference qualification and target runtime/cache integration remain. M8.2 streaming target and M8.5–M8.9 are not complete; current CommandPalette is navigation/search, not typed intent reasoning. |
| M9 / M10 | Existing frontend/live/EPG should be reused. Canonical frontend contracts and EPG boundary migration are incomplete. |
| M11 | Useful security, migration and fixture tests exist. Failure matrix, measured scale, native lifecycle, accessibility and signed clean-machine distribution remain qualification work. |

### Dependency-aware next milestone and stop boundary

The next **architecture** slice is bounded M2.1 plus the minimum M2.2/M2.4 needed
for an **internal model registry**: reuse existing model identifiers/adapters;
define logical descriptors/library-relative locations; add metadata persistence;
resolve current installs with explicit missing/unverified/available states. Test
fresh/upgrade persistence, unchanged existing paths, missing/corrupt artifacts and
absence of implicit downloads during discovery. No external library UI, runtime
rewrite, inference backend replacement, canonical import cutover or broad feature
implementation belongs in that slice.

Before exposing new integration, triage the source-established data/privacy risks
above with focused regressions, especially stale enrichment writes, cross-profile
artifact/event identity and TMDB error redaction. They are current-main concerns,
not reasons to introduce replacement architecture. Preserve the existing P00
work and obtain a reviewable baseline checkpoint before schema cutover.

`master-plan.md` §21 now records only dependency adjustments: registry/discovery
before removable libraries; minimum bounded/cancellable job/resource hooks before
shared runtime admission; protected raw capture before resolution and canonical
publication afterward; secret-free DTOs and migration gates with each vertical
slice. Deterministic catalog compilation and TMDB must not require optional model
availability. Broader frontend, intelligence and hardening goals are unchanged.

**Stopped here:** no next-milestone code, migrations, model downloads, provider
re-probes, commits or publishing were performed during this audit.

### Post-audit baseline validation

Rerun after the source audit and documentation updates, on 2026-09-25:

| Command | Result |
| --- | --- |
| `just check` | Pass, whole Rust workspace including Tauri. |
| `just test` | Pass: 259 Rust tests/doctests, 2 existing ignores; 28 frontend tests. Includes migration, protected storage, provider, reasoner and P00 regressions. |
| `just lint` | Pass: workspace/all-target Clippy with `-D warnings`, then ESLint. |
| `just typecheck` | Pass. |
| `cargo fmt --all -- --check` | Pass. |
| `pnpm -C apps/desktop build` | Pass. Existing >500 kB Vite chunk warning remains. |
| `pnpm verify-environment --manifest-only` | Pass; native dependency closure, linked SQLite, model hashes/licenses and signed/E2E qualification explicitly remain unverified. |
| `node --test scripts/stack.test.mjs tools/catalog-eval/evaluate.test.mjs tools/catalog-scale/provider.test.mjs` | 19 pass. Initial sandbox run had 3 `EPERM` localhost-bind failures; rerun with loopback permission passed. |
| `pnpm -C apps/desktop exec tauri build --debug --no-bundle` | Pass; native debug application built successfully. |
| `git diff --check`; documentation whitespace/fence check | Pass, including a separate check of the two untracked Markdown files. |

Rust build/check/test commands ran sequentially to avoid shared-target artifact
races. Full tests used localhost permission for synthetic servers. No test was
added or disabled for these documentation-only changes; existing tests provide
the baseline, not regression coverage for every newly identified audit risk.

Logs: `/tmp/nova-m1-check.log`, `/tmp/nova-m1-tests.log`,
`/tmp/nova-m1-lint.log`, `/tmp/nova-m1-fmt.log`, `/tmp/nova-m1-node.log`,
`/tmp/nova-m1-tauri.log` (local transient files, not committed).

Not rerun in this audit: native interactive playback/AirPlay, live provider import,
real AI inference/downloads, full-scale app performance, signing/notarization and
release DMG packaging. Prior P00 native smoke/release-build evidence is retained
above and is not presented as a new run. No current provider credentials or user
profile databases were accessed for this audit.

## M2.1 / internal M2.2–M2.4 — internal model registry

Updated: 2026-09-25. This is the bounded next slice selected by the M1 audit.
It adds a logical registry for the three **existing internal** model layouts and
stops before external libraries, runtime ownership, resource admission or UI
redesign. `master-plan.md` dependencies did not change.

### Implemented contract and reuse

- `nova-core::models` defines validated `ModelId`, `ModelLibraryId`,
  `RelativeModelPath`, `ModelDescriptor`, `ModelLibrary`, `ModelLocation` and
  `ModelAvailability`. A location resolves relative to a registered library
  root; `..`, absolute paths, ambiguous segments and control characters are
  rejected. No absolute machine path is persisted as a model identity.
- Additive app migration `0002_internal_model_registry.sql` stores the internal
  library, logical descriptors, one location per model, availability and the
  observation time. Profile migrations and existing settings are unchanged.
  `nova-stack.json` records the new app migration head. SQLx offline metadata
  includes the four compile-time checked queries. The read path reconstructs
  and validates the persisted library root as well as each location, so a
  stored model can be resolved without treating its ID as a filesystem path.
- Desktop discovery uses the existing Whisper model descriptor/path helper,
  NLLB model ID/installed layout, and E5 logical ID. It probes only the current
  `app_data_dir/models/{whisper,nllb,embeddings}` locations and updates the app
  repository in one transaction. `list_internal_models` and the three existing
  model-status commands refresh this registry through one serialized offline
  observation path. Status responses add `availability` while preserving their
  existing `installed` boolean semantics. IPC exposes IDs, capability, backend,
  last observed availability and optional revision; it never returns an
  absolute path or credential-bearing URL. Existing download, inference,
  caption, translation and enrichment flows retain their current paths.
- Discovery does not create model directories, download data or load a model.
  `missing` means no required artifact evidence; `unverified` means a partial,
  empty, malformed or opaque layout; `available` means the Whisper or NLLB
  layout passed shallow local checks for this platform. **Available is not
  cryptographic verification or a successful inference.** Existing mutable
  download sources have no pinned digest, so `artifact_revision` remains null.
  The FastEmbed marker alone remains unverified because the cache layout is
  opaque to this metadata probe.

### Validation and limits

Focused tests cover normalized path/ID contracts, unchanged internal path
resolution, fresh app migration, upgrade from app schema 0001 with profile and
setting preservation, reopen and availability transitions, logical ID stability
when a persisted location changes, transaction rollback on an invalid batch,
rejection of corrupted persisted library roots/locations, and a real offline
missing → available → missing discovery/persistence cycle without downloads.
They also cover partial/empty/malformed layouts and discovery without
filesystem creation.
No production model or authenticated provider URL is read by those tests.

Validation commands/results for this slice:

- `just check`: pass, including Tauri and SQLx offline checks.
- `just lint`: pass, including Clippy `-D warnings` and ESLint.
- `SQLX_OFFLINE=true cargo test -p nova-storage --test model_registry --locked`:
  focused fresh/upgrade/rollback cases pass before the final expanded run.
- `SQLX_OFFLINE=true cargo test -p nova-desktop --locked model_registry::tests`:
  initial offline/partial/corrupt cases pass before the final expanded run.
- `just test`: pass — 268 Rust tests/doctests, 2 existing ignores; 28 frontend tests.
- `just typecheck`, `pnpm verify-environment --manifest-only`,
  `cargo fmt --all -- --check`, and `git diff --check`: pass.

Completion validation for the persisted-library read and status integration:

- `SQLX_OFFLINE=true cargo test -p nova-storage --test model_registry --locked`:
  pass (4 tests, including fresh/upgrade, relocation, invalid-batch rollback
  and corrupt-root rejection).
- `SQLX_OFFLINE=true cargo test -p nova-desktop --locked model_registry::tests`:
  pass (4 tests, including missing → available → missing persisted discovery).
- `just check`, `just lint`, `just typecheck`, `just test`,
  `pnpm -C apps/desktop build`, `pnpm verify-environment --manifest-only`,
  `cargo fmt --all -- --check`, and `git diff --check`: pass. The frontend
  production build retains its existing large-chunk warning.
- The first `just test` attempt could not bind the existing catalog-reasoner
  loopback fixture under the sandbox (`Operation not permitted`); the unchanged
  full suite passed with local loopback permission. No provider or user data
  was used.

A raw `cargo test` invocation without `SQLX_OFFLINE=true` picked up the local
development `DATABASE_URL`, whose database predates app migration 0002, and SQLx
reported missing model tables at compile time. The repository's `just` recipes
set `SQLX_OFFLINE=true`; the command above and CI use that intended path. The
new query metadata was generated using a temporary, synthetic combined schema
under `/tmp`, not a user database. `cargo sqlx prepare` itself could not parse
this installation's `cargo metadata` output, so the documented SQLx
`SQLX_OFFLINE_DIR` compile path generated the four cache files. The offline
workspace check then passed.

The bounded internal model-library/persistence milestone is complete. No new
migration was needed beyond the already additive app schema 0002; the read-path
and status integration use it without moving model files or changing settings.

Remaining after this internal slice: external/removable library authorization and
offline-device behavior (M2.3); artifact provenance/digest verification and
richer registry metadata; runtime leases/unload (M2.5); resource and worker
admission (M2.6/M7). The next bounded milestone, M2.3, is recorded below.

## M2.3 — removable model-library availability

Updated: 2026-09-25. This milestone extends the existing registry without
changing playback, catalog rows, profile databases, download paths or inference
scheduling. `master-plan.md` ordering remains valid.

- App migration `0003_external_model_libraries.sql` adds separate external
  library grants and per-library relative model locations. App schema 0002,
  internal model locations, profiles and settings are preserved. Each external
  library has a stable logical ID. The bookmark is stored as an opaque local
  blob in app.db; no absolute root path is stored as a model ID or sent through
  IPC. `nova-stack.json` names the new migration head, and SQLx offline query
  metadata was regenerated against a synthetic combined schema.
- The macOS folder picker runs on the main AppKit thread behind a Tauri command.
  The webview cannot submit an arbitrary filesystem path. Nova prefers a
  read-only security-scoped bookmark and falls back to a normal bookmark for
  the current non-sandboxed bundle. Bookmark resolution is noninteractive and
  does not mount a missing drive. A scoped grant is stopped after each bounded
  metadata probe, including failure paths. No bookmark bytes or resolved path
  appear in UI DTOs or logs; tests use synthetic local folders. The current bundle has no App Sandbox
  entitlement; a future sandboxed distribution must qualify entitlement and
  bookmark migration under Apple's [macOS App Sandbox file-access guidance](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox).
- Disconnect or failed grant resolution records `unavailable` for every model
  in that external library, without deleting its descriptor, relative location
  or bookmark. A connected folder with no model artifacts reports `missing`.
  Reconnection refreshes availability under the same IDs. Registry refreshes
  on startup, every 30 seconds while running, and when Settings requests the
  list. Only known Whisper, NLLB and embedding layouts are probed, with no
  download, runtime load or filesystem writes. Internal observations and
  external libraries are separate; one drive's failure cannot alter another
  library's rows. The UI shows connection and model state with a manual refresh.
- Existing AI jobs still use their existing internal model paths. External
  inventory is not advertised as an inference selection; path selection and
  inference leases belong to M2.5. Playback never consults this registry.

Focused validation covers schema 0002 → 0003 upgrade, preserved app settings,
internal records and FK integrity; multi-library isolation and transaction
rollback; removed/recreated folder state with stable IDs; symlink rejection;
and a real macOS bookmark → registry → SQLite disconnect/reconnect cycle using
a synthetic local folder.
There is no physical removable-drive, native picker interaction, signed bundle
or App Sandbox entitlement qualification in this run.

Validation commands/results for M2.3:

- `SQLX_OFFLINE=true cargo test -p nova-storage --test external_model_libraries --locked`:
  pass (3 migration, isolation and rollback tests).
- `SQLX_OFFLINE=true cargo test -p nova-desktop --locked model_registry::tests`:
  pass (6 tests); `SQLX_OFFLINE=true cargo test -p nova-desktop --locked
  native_bookmark_refresh_persists_disconnect_and_reconnect`: pass after the
  final integrated test was added.
- `just check`, `just lint` (zero warnings), `just typecheck`,
  `pnpm -C apps/desktop build`, `pnpm verify-environment --manifest-only`,
  `cargo fmt --all -- --check`, Prettier check and `git diff --check`: pass.
- `just test`: pass with loopback permission for existing synthetic HTTP
  fixtures; two pre-existing Rust tests remain intentionally ignored, and all
  28 frontend tests pass. The frontend build retains its existing large-chunk
  warning.

M2.3 is complete at the registry/authorization boundary. The next model
milestone is M2.5 runtime resolution and ownership; this work does not select
external files for inference or add scheduling policy.

## M2.5 — shared model runtime and lease ownership

Updated: 2026-09-26. Complete on branch `m2-model-runtime-manager`, based on
`33cf05b`. The starting point already contained M2.3 persistence, external
authorization and availability tests. Source inspection confirmed that search
cached its own embedder, enrichment loaded separate embedders, translation
loaded per session, and Whisper removal had no shared in-use exclusion. This
slice consolidates those existing backends without changing catalog identity,
playback ownership or the milestone order in `master-plan.md`.

### Implemented contract and reuse

- `AppState` owns one `ModelRuntimeManager` backed by the app database, separate
  from profile databases. It resolves the existing logical IDs through the
  persisted registry, prefers an available internal location, and otherwise
  uses an available authorized external library. Availability is refreshed
  before acquisition; missing/offline locations cannot silently reuse a cached
  backend. Reconnection preserves logical identity. An offline external
  library does not trigger an automatic replacement download.
- Existing FastEmbed/ONNX, NLLB/CTranslate2 and Whisper implementations are
  backend adapters. Concurrent first use loads one shared instance; a failed
  load is retryable. Runtime acquisition is local-only. E5 now reads the
  existing Hugging Face cache layout without constructing a network client;
  missing/partial caches fail without creating files, and cache symlinks must
  stay inside the registered library. Existing explicit install commands remain
  the download boundary. Runtime resolution uses registered locations rather
  than an ambient `HF_HOME` override.
- Strong leases retain both the backend and its macOS folder authorization.
  Unload, location replacement, installation and internal model removal are
  excluded while a lease is active. A cancelled or aborted load owner cannot
  release the lifecycle lock while native loading continues. Native load and
  destruction run outside the async executor. The owned bookmark guard reuses
  the M2.3 authorization path and remains alive until backend destruction.
- Search, transcript embedding, catalog/series enrichment, caption windows and
  translation batches acquire the shared runtime. Cached translation replay
  does not eagerly initialize a model. Embedding work yields per-model
  admission between batches; query/document prefixes and vector identity stay
  unchanged. Existing lexical fallback remains available when acquisition
  fails. Playback does not acquire a model lease.
- Cancellation while waiting for load/admission returns promptly. Native
  ONNX/CTranslate2 work retains its lease and admission permit until the
  current call completes, even if the requester is cancelled, aborted or
  panics. Whisper cancellation and its 300-second process deadline kill and
  reap the child before the owning worker releases its lease.
- Resource hooks expose loaded, leased, busy and transitioning counts plus
  known/unknown resident estimates. One operation per model is admitted at a
  time. `unload_model` and `model_runtime_status` are registered Tauri commands;
  the IPC contract documents their safe count-only responses and busy errors.
  Whisper's prepared adapter is distinct from resident subprocess weights.

No schema migration, model-file relocation, configuration rewrite or new
replacement registry was necessary. App migrations 0002/0003 and existing
logical IDs remain authoritative. Existing status DTO shapes are preserved;
their installed flag now includes usable external locations. The new runtime
commands have no React consumer in this milestone.

### Regression coverage and validation

Added 18 focused tests: 13 runtime/lifetime tests, three offline embedding-cache
tests, and two subprocess cancellation/deadline tests. They cover concurrent
first load, cloned leases blocking unload, cancellation and task abortion
during load/work/mutation, retry after failed load, panic cleanup, admission
exclusion, batching cancellation, location replacement with stable identity,
missing-location refresh, and macOS bookmark disconnect/reconnect. Fixtures
use synthetic model artifacts and fake native adapters; subprocess tests use
a local synthetic child. No provider credentials or production models are
needed.

Validation commands/results:

- `SQLX_OFFLINE=true cargo test -p nova-desktop --lib model_runtime::tests --locked`:
  pass during focused iteration; all 13 final cases also pass in the full run.
- `SQLX_OFFLINE=true cargo test -p nova-desktop -p nova-enrich -p nova-transcribe --locked`:
  pass during integration, including offline cache and child-lifetime tests.
- `just test`: pass for the full Rust workspace, migration/integration tests and
  all 28 frontend tests. Existing synthetic HTTP fixtures used loopback
  permission; the two pre-existing ignored tests remain ignored.
- `just check` and `just lint`: pass (whole workspace including Tauri;
  Clippy all targets with `-D warnings`, plus ESLint).
- `just typecheck`, `pnpm -C apps/desktop build`, and
  `pnpm verify-environment --manifest-only`: pass. The production frontend
  retains the existing large-chunk warning.
- `cargo fmt --all -- --check` and `git diff --check`: pass.
- `pnpm exec prettier --check docs/nova-implementation-status.md`: reports
  pre-existing table formatting and older paragraph indentation; the appended
  M2.5 section matches Prettier output. Historical sections were not reformatted.

### Qualification limits and next milestone

The concurrency tests establish ownership and cancellation with deterministic
adapters; this run does not certify real ONNX/CTranslate2 inference quality,
peak native memory, playback-under-inference stress, a physical removable
drive, native picker interaction, or signed App Sandbox entitlements. Bookmark
tests exercise the current macOS bundle's native bookmark behavior using
temporary directories. Structural availability is not digest verification.

Native ONNX/CTranslate2 calls finish their current batch after cancellation;
they are not forcibly preempted. Backend resident sizes remain explicitly
unknown. M2.5 is complete. The bounded M2.6 resource-policy work is recorded
below; measured memory budgets, queue limits and automatic eviction remain
future qualification/work items.

## M2.6 — playback-aware model resource policy

Updated: 2026-09-26. A code map was used to locate M2.5 runtime callers,
the playback engine and the persisted jobs subsystem, then verified against
their source. It showed the intended navigation links, but source inspection
was treated as authoritative. `ModelRuntimeManager` is owned by `AppState`;
model callers are search, enrichment, caption and translation. Playback has
separate mpv, HLS/web and AirPlay lifecycles. `JobQueue` and `JobRepository`
persist priorities, but desktop code does not instantiate that queue or enqueue
these workers. No schema was changed.

- Reused the existing shared runtime, per-model operation guards, Tauri
  `AppState`, mpv open/close, HLS `resolve_stream`/`stop_playback`, and AirPlay
  engage/disengage lifecycles. RAII playback guards are acquired and released
  with those tokens; session replacement, close, cleanup and drop release the
  correct guard. Distinct players use reference counting so closing one player
  cannot resume background work while another is still active.
- Added a process-wide, cancellable priority admission lane for model
  initialization and inference. It admits one expensive model operation at a
  time while native resident-memory estimates remain unknown. Priorities follow
  the master-plan order: realtime captions/translation, interactive search,
  foreground, metadata, background and maintenance. Queued eligible work is
  ordered by priority and FIFO within a class. Cancelled waiters are removed.
- Active playback lets realtime and interactive requests use the lane and
  suspends foreground/background classes until playback ends. If playback
  starts during native inference, the bounded operation already in progress
  completes; ONNX/CTranslate2 cannot be safely preempted. Embedding remains
  chunked at 256 documents, and translation retains its existing bounded batch.
- Search and transcript semantic work use interactive priority; caption ASR
  and translation use realtime priority; catalog/series enrichment uses
  background priority. Status reports active playback count, queued work and
  the current class in addition to the existing lease/loaded-model metrics.
- No migration or durable configuration change was needed. Global admission
  capacity is intentionally conservative because production memory sizes have
  not been measured. This is a model-runtime policy, not a general worker pool.

Focused regression tests cover playback guard reference counts, interactive
work remaining available during playback, background suspension/resume,
priority/FIFO admission behavior, cancellation cleanup, and cold model loading
waiting until playback ends. Desktop sources show that imports, EPG and
enrichment are currently spawned directly; the durable `JobQueue` is not
integrated there. M2.6 applies classes at the existing model boundaries, while
durable recovery, queue bounds, backpressure and enforcement of persisted
`jobs.priority` across desktop workers remain M7 work, as planned.

Validation commands/results:

- `SQLX_OFFLINE=true cargo test -p nova-desktop --lib --locked`: pass (43
  passed, one pre-existing ignored test).
- `just test`: pass for the Rust workspace/doctests and 28 frontend tests;
  existing loopback fixtures required local loopback permission.
- `just check` and `just lint`: pass, including Tauri checks, Clippy with
  warnings denied, and ESLint.
- `cargo fmt --all -- --check` and `git diff --check`: pass.

M2.6 is complete at the model-runtime/playback priority boundary. Full durable
job scheduling remains in M7; no subsequent milestone has been started.

## P06 — Resource governor

Updated: 2026-09-26. Branch `m2-model-runtime-manager`, uncommitted on top of
`17c9f63`. P00–P05 were not revisited. The worktree already held unrecorded,
in-progress work that routed imports, EPG refresh and enrichment through the
persisted `JobQueue` and had each job take (and immediately drop) an
admission permit before claiming. P06 preserved and evolved that work rather
than adding a second scheduler. Code search located the `AdmissionController` ↔ `ModelRuntimeManager` ↔
`DesktopJobs` paths; three read-only investigations (jobs, model runtime,
playback/failure) mapped them; conclusions were verified against source.

### Findings that shaped the slice

- A permit taken and dropped before `claim_next` ordered only job *starts*:
  a job admitted before playback ran unthrottled to completion.
- Heavyweight residency was unbounded: E5 and NLLB could stay resident
  indefinitely; nothing evicted idle models. `unload` already refused leased
  models.
- Selection had no aging, so lower classes could starve behind a continuous
  higher-class stream.
- Jobs left `running` by a crash were never requeued; repeated requests
  enqueued duplicate rows; queued jobs could not be cancelled.
- `mpv_close` leaked its playback guard on any engine error, which would
  suspend background work until the next open.
- The investigations reported that holding the model lane while waiting on the per-model
  semaphore could cause inversion. Rejected: with one model-lane slot, whoever
  holds the per-model semaphore already holds admission.

### Implemented contract

- **One governor, two bounded lanes** (`resource_policy.rs`). The existing
  `AdmissionController` keeps its single model lane and gains a job lane
  (`ResourceBudget::DEFAULT`: 1 model operation, 2 job slots shared across
  every profile and job kind, 2 resident models; `LOW_RESOURCE`: 1/1/1).
  `WorkClass` is the explicit 7-tier order (`tier()` 2–7; tier 1 is playback,
  represented by `PlaybackActivityGuard`s). Selection per lane is
  deterministic: realtime first, then any waiter bypassed `AGING_LIMIT` (8)
  times by higher-class grants, then class rank, then FIFO.
- **Playback pressure.** Active playback suspends Metadata, Background and
  Maintenance on both lanes. Realtime, Interactive and Explicit continue.
  Explicit import/refresh is admissible during playback because it never uses
  the model lane; it still ranks below realtime and interactive work.
- **Held job slots and cooperative preemption.** `JobHandler::ready` now returns
  a `JobPermit` that `handle(job, permit)` owns for the whole run (drop on
  completion, failure or panic unwind). Resource-blocked jobs stay durably
  `queued`. `JobSlot::checkpoint` releases the slot when playback suspends the
  class or a higher-priority job waits while the lane is full, then reacquires
  it cancellably. Enrichment checkpoints between 256-document embedding units
  (catalog and series), before each stage, and per TMDB item. It drops its E5
  lease while paused so the idle model stays evictable, and reports
  `stage: "queued"` while waiting. Checkpoints are never taken inside a
  database transaction.
- **Model residency budget** (`model_runtime.rs`). Only in-process backends
  count (E5, NLLB); Whisper weights live in its subprocess. A
  `ResidencyToken` reserved before a cold load is owned by the loaded backend
  and released only after the backend drops, so failed loads, cancellation,
  unload, mutation and replacement cannot leak or double-count. Before
  loading, the least-recently-used idle, unleased model is evicted under the
  same lifecycle-lock + zero-lease exclusion as `unload`; a leased model is
  never evicted or unloaded. When nothing is evictable, Realtime and
  Interactive may exceed the budget, so a suspended low-priority lease holder
  cannot block them (priority inversion). Lower classes wait cancellably for a
  lease release. No caller holds one lease while acquiring another, so the
  wait cannot self-deadlock.
- **Cancellation.** Per-(kind, source) cancellation handles for running jobs.
  `delete_source` removes the source's queued import/EPG/enrichment jobs and
  cancels running enrichment (next checkpoint; model waits also observe it)
  and EPG (between feeds). A running import is not interrupted: its single
  catalog transaction commits or rolls back as a unit, and SQLite's
  serialized writers plus enforced foreign keys prevent orphaned rows.
- **Backpressure and recovery.** `enqueue_unique` coalesces requests for work
  that is already queued, so queue depth is bounded by distinct work. A
  running job does not suppress one follow-up request, which may see newer
  provider state. `recover_interrupted` requeues stale `running` rows once, when
  a profile's workers are first created, before any drain. Append imports stay
  idempotent through the source's commit stamp.
- **Typed state.** `model_runtime_status` adds `residentModels`,
  `residentModelBudget`, `queuedJobs`, `activeJobs`, `jobSlots` and
  `suspendedWork`, mirrored by the `ModelRuntimeStatus`/`WorkClass` TS types.
  React only reads; scheduling truth stays in Rust. `docs/ipc-contract.md`
  documents the policy, the new progress stages and dedup behavior.
- `mpv_close` now releases the playback guard unless the token is stale, in
  which case a replaced session's live guard is kept.

No language model participates in any scheduling decision. No schema
migration was needed: the existing `jobs` columns cover dedup, recovery and
cancellation. Four new SQLx offline query files were generated.

### Regression coverage

24 new or rewritten tests, all deterministic, with synthetic adapters and jobs:

- Governor (9): master-plan tier order and playback/residency eligibility;
  strict priority ordering across all six classes; aging bounds starvation
  without overtaking realtime; job lane bounded and independent of the model
  lane; low-resource budget with cancelled waiters leaving no state; checkpoint
  yields only to higher priority or playback, then resumes; checkpoint
  cancellation, including while suspended; panicking holders release permits
  and slots; a 240-task multi-threaded contention run with playback toggling
  completes (no deadlock) and leaves zero residual state.
- Residency (6): idle-model eviction under the low-resource budget; default
  budget keeps both resident; a leased model is never evicted and explicit
  unload is refused while lower classes wait for release; realtime is not
  blocked by a suspended background lease holder (inversion) and the budget
  is restored afterwards; cancelled waiters and failed loads leave no
  reservation; subprocess backends consume no budget.
- Desktop jobs (6): playback suspends queued metadata jobs but not explicit
  import, and they resume afterwards; a running job yields its slot during
  playback and resumes; a bounded lane where only higher-priority waiters
  preempt at checkpoints; failed or panicking jobs release their slots;
  cancellation of an unadmitted job keeps it queued; source cancellation stops
  only that source's running job.
- Jobs crate (3): duplicate requests coalesce; interrupted jobs are requeued
  per kind and cancelled queued jobs are removed; the admission permit is
  owned for the whole job.

### Independent review

A fresh reviewer that did not author the code inspected the diff, the
governor/runtime/lease interactions, playback signaling, the tests and the
call paths. It found nothing Critical. Each finding was verified
independently:

- High: the series-embedding stage ran as one unbounded call without
  checkpoints, holding a job slot during playback. **Confirmed and fixed**:
  both embedding stages now share the checkpointed `embed_checkpointed` loop.
- Medium: `delete_source` does not interrupt a running import. **Not a
  defect**: import atomicity plus SQLite writer serialization and enforced
  foreign keys prevent orphaned commits. This behavior predates P06 and is
  documented above.
- Low: no dedup against a *running* job. **Intentional**; see backpressure.

While re-validating, one desktop-jobs test showed a timing race in its own
design (a legitimate preemption sometimes happened before its observation
point). It was restructured to assert the same property deterministically.
Afterwards the concurrency modules passed 25 consecutive runs.

### Validation commands/results

- `SQLX_OFFLINE=true cargo test -p nova-desktop --lib`: 64 passed, 1
  pre-existing ignored; `resource_policy`, `desktop_jobs` and `model_runtime`
  passed 25 consecutive runs. `cargo test -p nova-jobs`: 7 passed.
- `just test`: pass: 328 Rust tests/doctests, 0 failed, 3 pre-existing
  ignored; 29 frontend tests.
- `just lint` (Clippy all targets `-D warnings` + ESLint), `just check`,
  `just typecheck`: pass.
- `pnpm -C apps/desktop build`: pass, with the existing large-chunk warning.
  `pnpm verify-environment --manifest-only`: pass; it lists its standing
  UNVERIFIED native/packaging gates.
- `cargo fmt --all -- --check`, `git diff --check`: pass.

### Remaining known gaps

- No OS memory-pressure, thermal or battery signal exists. `LOW_RESOURCE` is a
  deterministic construction-time profile; production uses `DEFAULT`, and
  budgets remain unmeasured estimates (resident sizes are still unknown).
- A paused player counts as active playback, so suspended classes wait until
  the session closes. There is no paused/playing distinction or long-suspension
  aging.
- Guards leak if the webview dies without calling `stop_playback` for a
  resolved proxy stream. There is no heartbeat or expiry.
- Native ONNX/CTranslate2 calls finish their current bounded unit; they are not
  forcibly preempted. Realtime/interactive over-budget residency is temporary
  and is restored by the next constrained load.
- The job lane governs persisted desktop jobs only. Captions, translation and
  search remain session-driven on the model lane (by design). The periodic
  external-library refresh and AirPlay status loops are not governed.
- Import cannot be cancelled mid-transaction. Queue-level backpressure is per
  distinct payload, not a numeric depth limit. `set_source_auto_refresh` has no
  scheduler yet. Profile workers persist for the process lifetime.
- No frontend consumer reads `model_runtime_status` yet. There is no
  on-device qualification of playback under concurrent enrichment/inference.

### Next milestone

**P07 — canonical catalog vertical slice**: evolve the catalog toward
`SourceRecord != CanonicalEntity != StreamVariant` (master-plan M3.1/M3.2
with the minimum M9 secret-free DTO contract), using an additive migration.
P06 did not start it.

## P07 — canonical catalog vertical slice (movies)

Updated: 2026-09-26. Branch `feat/p07-canonical-catalog` off `main` at
`7ce9d95`. Scope: evolve the catalog toward
`SourceRecord != CanonicalEntity != StreamVariant` for one coherent slice:
movie browse → detail → favorite/watchlist → play/resume (master-plan
M3.1/M3.2 plus the minimum M9 secret-free DTO contract). Provider ingestion was
not rewritten, and there is no parallel catalog.

### Mapping before changes

| Concern | Before P07 (source evidence) |
| --- | --- |
| Source record | `vod_items` row (`VodRepository`), numeric ID, deleted/reinserted on every refresh (`snapshot_and_clear`). |
| Provider types | `ParsedEntry` (M3U/Xtream) → `VodItem` in `push_entry`; the Xtream `stream_id` was dropped. |
| User state | `vod_items.is_favorite`/`is_in_watchlist` on the provider row; resume/completion in `watch_history(media_kind='vod', media_id=row)`, cascaded away by trigger `catalog_state_vod_ad`. |
| Refresh continuity | Snapshot/restore keyed by exact credential-bearing `stream_url`, so Xtream credential rotation dropped favorites, watchlist and resume. |
| Tauri/React DTOs | `VodItem` (with `streamUrl`) through `list_vod`; favorites/playback by row ID; React invents version groups. |
| Existing contracts | `nova_core::catalog` already had opaque `EntityId`/`VariantId`/`SourceRecordId` and `StreamVariant`, plus framed-SHA-256 `Fingerprint`, all unused by the app. |

### Implemented contract

- **SourceRecord:** `vod_items` stays the provider record and the end of
  ingestion. `ParsedEntry.upstream_id` is additive: Xtream sets
  `xtream:vod:<stream_id>`, M3U sets none. At the compiler boundary
  (`VodRepository::insert_batch_tx`), each row gets a secret-free
  `binding_key` (versioned framed SHA-256 of the stable ID, else of the
  locator) and a `locator_key`. The provider ID is not persisted, and
  `VodItem.upstream_id` is `#[serde(skip)]`.
- **CanonicalEntity:** `canonical_entities` (opaque `mov-…` `EntityId`) owns
  favorite, watchlist and `entity_watch_state` (resume, completion, caption
  mode). No provider row or source deletion removes an entity or its state.
- **StreamVariant:** `stream_variants` (opaque `var-…` `VariantId`), unique per
  `(source_id, binding_key)`. `vod_item_id` points at the current record and
  is `ON DELETE SET NULL`, so refresh unbinds and the compiler re-binds.
- **Catalog compiler:** `CanonicalRepository::compile_movies_tx` runs inside the
  existing import transaction, after rows and the legacy restore. It re-binds
  by key, upgrades locator-keyed variants to stable keys in place, creates
  entities only for unclaimed keys (seeded from legacy flags/history), refreshes
  display title/year, and re-derives the legacy flag mirror from the entity. A
  failed import rolls back and keeps the previous bindings. Only records sharing
  a key within one source are unified. Cross-source/duplicate consolidation is
  entity resolution (M3.4/M3.5) and is deliberately not attempted, to avoid
  false merges.
- **Canonical API** (`commands/movies.rs`): `list_movies`, `count_movies`,
  `get_movie`, `set_movie_favorite`, `set_movie_in_watchlist`,
  `resolve_movie_playback`. DTOs (`nova_core::catalog::media`) carry no stream
  or source URLs. Rust selects the variant (a requested variant only if it
  belongs to the entity) and returns a transient player locator. Progress
  written against it lands on the entity.
- **Single source of truth:** the legacy `VodRepository::set_favorite` /
  `set_in_watchlist` and `WatchHistoryRepository::record_progress` /
  `resume_position` for `vod` write through to (or read from) the owning
  entity in the storage layer. Unmigrated surfaces (semantic search results,
  player progress, Continue Watching cards) therefore stay consistent.
  Continue Watching movies come from entity state.
- **Frontend slice:** the Movies tab, counts, rails, hero, hover preview, detail
  (via `get_movie`, including a "Resume · m:ss" hint) and favorite/watchlist use
  the canonical DTOs and `entityId`. Play resolves through Rust. The existing
  player is unchanged.

### Migration

`0021_canonical_movies.sql` is additive only: new tables `canonical_entities`,
`stream_variants`, `entity_watch_state`, and nullable
`vod_items.binding_key`/`locator_key` with indexes. Nothing is dropped or
reinterpreted, and legacy columns and `watch_history` rows remain as a mirror.
The data step (`backfill_movies`, run by `init_profile_db` after SQL
migrations) keys and compiles each pre-0021 source in one `BEGIN IMMEDIATE`
transaction, so an interrupted upgrade leaves no half-keyed source and a
concurrent runner waits instead of failing. It seeds entities from legacy
favorites, watchlist, resume and completion, taking the most recent history
across duplicate rows. `0022_vod_unkeyed_index.sql` (additive partial index)
makes the steady-state check about 1 ms at 30k movies. Profile opens are
serialized in the desktop (`AppState::profile_open`), because SQLx's SQLite
migrator takes no lock. It follows ADD → MIGRATE → VERIFY. Cutover is this
movie slice only; removing legacy columns is later.

### Regression coverage

16 new tests plus updated frontend tests:

- Core (2): binding keys are stable, namespaced and credential-free across
  rotation; canonical DTO JSON contains no locators.
- Storage (12): one entity/variant per record and idempotent recompile; state
  survives refresh and credential rotation with a changed row ID, and never
  leaks onto a reused ID; completion is preserved and hidden from Continue
  Watching; a temporarily dropped item keeps its entity and state and returns;
  locator→stable-key upgrade keeps identity; duplicate records share an
  entity; distinct items with the same title are never merged; source deletion
  keeps entities and state; filters/sort/count/paging use entity state; variant
  selection never plays another entity's variant; legacy row writes land on the
  entity and survive refresh; upgrade chain from a representative migration-20
  database preserves every kind of state, is idempotent, and passes
  `integrity_check`/`foreign_key_check`.
- Desktop pipeline (1): through the real `run_import`, a failed refresh keeps
  the previous binding, and a credential-rotated refresh keeps entity, favorite
  and resume.
- Frontend: movies are listed through `list_movies` as `movie` items keyed by
  `entityId` with no URLs; detail and toggles call `get_movie` /
  `set_movie_*` with the entity ID.

The existing `failed_refresh_preserves_catalog_flags_and_resume` test caught a
real regression during development: a legacy direct repository write bypassed
the entity and was overwritten by the mirror on refresh. The fix moved the
write-through into the storage repositories, so every caller is covered.

Scale probe (`scale_probe_compile_30k`, ignored, release build): at 30k movies
the insert alone is 0.45 s, the first compile 0.98 s, a steady-state refresh
compile about 0.6 s, a deep browse page 44 ms, an entity lookup 21 µs, and the
reopen check 0.9 ms.

Real-data qualification is opt-in: `real_profile_upgrade` (ignored) copies the
database named by `NOVA_UPGRADE_DB` with `VACUUM INTO`, upgrades only the copy,
and asserts no lost favorites, watchlist or resume, no unmapped rows,
integrity, foreign keys and idempotent reopen. It prints counts only. It was
exercised on a synthetic pre-0021 database. Running it against the user's real
profiles was not performed: access to personal profile data was declined in
this session, so the user needs to run it themselves.

### Independent review

A fresh reviewer inspected `6cec388`. It found nothing Critical. Each finding was
verified in source before any change:

- High, confirmed and fixed: unserialized profile opens could run the data
  step concurrently and fail a profile open. Opens are now serialized, and the
  data step uses `BEGIN IMMEDIATE`. A regression test runs concurrent backfills.
- High, confirmed and fixed: the canonical grid lost the enrichment
  `dedup_group` grouping fallback. It is now carried as a presentation-only
  `dedupGroup` hint.
- Medium, confirmed and fixed: the steady-state reopen re-scanned the catalog.
  Fixed with atomic per-source keying plus the 0022 partial index (97 ms →
  0.9 ms at 30k).
- Medium, confirmed and fixed: the legacy mirror skipped unbound duplicate
  rows. The mirror and write-through now match rows by binding key.
- Low, confirmed and fixed: resume seeding kept the oldest history. It now keeps
  the newest, across all rows sharing the key. The new regression initially
  failed and exposed that seeding read only the bound row.
- Low, documented: the self-referencing rebind depends on
  `UNIQUE(source_id, binding_key)`. The invariant is commented, and a test
  rebinds six variants with duplicates in one statement.

### Validation commands/results

- `just test`: pass on three consecutive runs: 345 Rust tests/doctests, 0
  failed, 5 ignored (3 pre-existing plus the opt-in scale and real-profile
  probes); 30 frontend tests. The stabilization suite passed 15 consecutive
  isolated runs.
- `just lint` (Clippy all targets `-D warnings` + ESLint), `just check`,
  `just typecheck`: pass.
- `pnpm -C apps/desktop build`: pass, with the existing large-chunk warning.
- `cargo fmt --all -- --check`, `git diff --check`, Prettier on changed
  frontend files: pass.
- SQLx offline metadata regenerated with `cargo sqlx prepare --workspace` against
  a schema built from every app/profile migration. Stale query files for
  superseded query text were pruned, including three job queries superseded in
  P06.

### Remaining known gaps

- One entity per provider item per source. The same film from two sources, or
  language/quality copies, are separate entities. React still gathers copies
  by raw title for presentation (`movieVersionKey`) until entity resolution
  (M3.4/M3.5) consolidates server-side. There is no alias/merge/split history
  yet (M3.6).
- M3U items have no stable upstream ID. A changed M3U URL creates a new entity,
  matching pre-P07 URL-keyed behavior. Deleting and re-adding a source creates
  new entities (old ones are retained, unlisted, with their state).
- The player still addresses the transient `('vod', row)` locator returned by
  Rust. A variant-addressed PlaybackCoordinator is M6.
- Semantic search, the Command palette and Continue Watching cards still carry
  legacy `VodItem`/row DTOs (consistent through write-through). Series, live and
  EPG are unmigrated. Legacy serving DTOs still expose `streamUrl`.
- Artwork URLs come from the provider or TMDB and are treated as public display
  assets.
- The first large import pays about 1 s of compile time. No generation/snapshot
  publication yet (M3.7).
- Not yet verified in the running app (`just dev`) or against real profiles.
  Not merged to `main`.

### Next milestone

Next in the plan: M3.3 versioned fingerprints (unchanged/changed/new/deleted
classification for reuse), then candidate blocking and entity resolution to
consolidate movie variants, or the next M9 vertical slice (series). P07 did not
start them.

## P09 — deterministic canonical entity resolution (movies)

Branch `feat/p09-entity-resolution`, stacked on P08 (`feat/p08-catalog-reconciliation`,
blocking primitives). Consolidates movie entities that P07 left one-per-provider-item,
without ever merging on uncertain evidence.

### Implemented contract

- **Pure resolver** (`crates/core/src/catalog/resolution.rs`). Evidence per
  entity (titles, years, trusted IDs namespaced by authority and kind). Blocking
  runs via P08's `blocking_keys`, and blocks over 64 abstain. Hard contradictions
  always reject: kind mismatch, conflicting trusted IDs, different series,
  different season/episode numbers, years more than 1 apart, and cannot-link.
  Merge rules: a trusted ID corroborated by a title or year; identical complete
  episode coordinates; or an identical normalized title *and* year. Everything
  else abstains with a typed reason. An ambiguous-hub guard and conflict-checked,
  size-capped clustering prevent transitive false merges. The oldest entity
  survives (ties go to the lowest ID). `RULESET_VERSION = 1`.
- **Identity authority**: `MergeDecision` has private fields and is only built
  by `resolve`. Two compile-fail doctests prove it can't be forged or converted
  from an `AiSuggestion`.
- **AI boundary**: `AmbiguityCase` (at most 8 real candidates, resolver-built)
  → `assess_ai_verdict`. It accepts only `abstain` / `same_as` / `distinct_from`
  over the case's own candidates, with a 4 KiB limit, protocol version check,
  subject check and `deny_unknown_fields`. Output is an advisory `AiSuggestion`
  with no path to storage. No model is wired to it yet.
- **Storage** (`repositories/canonical/identity.rs`): `resolve_movies_tx` runs
  in the import transaction after `compile_movies_tx`. `ensure_movies_resolved`
  runs once on open per ruleset version. Each merge is atomic: variants move to
  the survivor; favorite/watchlist are ORed; the latest resume/completion wins;
  aliases and cannot-links follow the merge; the absorbed entity is tombstoned
  and redirected, with chains compressed; append-only lineage records prior
  state; the legacy mirror is re-derived. Other APIs: `canonical_entity_id`
  (bounded, cycle-checked historical-ID walk), `record_cannot_link`, `lineage`.
- **Historical IDs** are honoured by `movie_detail`, `resume`, `resolve_playback`,
  `set_favorite`, `set_in_watchlist` and `record_progress`. Detail returns the
  survivor's ID. `list_movies`/`count_movies` return one card per entity per source.
- **Trusted-ID producer**: Xtream `get_vod_streams.tmdb` (number or string; 0,
  empty and non-numeric are rejected) → `ParsedEntry`/`VodItem.provider_tmdb_id`
  → `vod_items.provider_tmdb_id` → durable `external` alias. Nova's lexical TMDB
  enrichment match is deliberately *not* treated as identity evidence.

### Migration

`0023_entity_resolution.sql`, additive:

- `canonical_entities.state` (`active` | `merged`) and `vod_items.provider_tmdb_id`;
- `entity_lineage`, append-only, enforced by UPDATE/DELETE triggers;
- `entity_redirects`, with `CHECK old_id <> new_id`;
- `entity_aliases` (`title` | `year` | `external`, not globally unique);
- `entity_cannot_link`, as an ordered pair with `CHECK a < b`;
- `entity_resolution_state`, the ruleset marker.

Existing profiles are resolved once on their next open.

### Regression coverage

- Core (20 + 2 compile-fail doctests): exact merge into the oldest entity; movie
  vs series; conflicting IDs; cross-namespace IDs; remake years reject and near
  years abstain; missing year yields a bounded ambiguity case; Rocky vs Rocky 2
  are never compared; localized titles merge only on a corroborated ID; episode
  coordinates and series; no chaining across ID conflicts or year drift;
  cannot-link; inconsistent evidence; order independence; oversized blocks;
  20k-entity scale (under 3 s in a debug build, about 0.3 s observed); ID
  validation and redaction; AI accept, abstain, reject (7 malformed or
  out-of-case verdicts, size limit) and candidate bound.
- Storage (12): cross-source merge with combined state, historical IDs through
  every API, legacy mirror, lineage prior state (secret-free) and idempotent
  re-resolve; contradictions and missing years prevent merges; a renumbered
  item rejoins its entity with favorite/watchlist/completion; trusted IDs
  outlive their rows and keep blocking; one card per entity with every variant
  playable; redirect chain compression; corrupt redirect cycles are rejected;
  cannot-link blocks and makes a third copy ambiguous; lineage is append-only;
  open-time resolution runs once per ruleset.
- Xtream (1 new + 1 extended): `tmdb` parsing accepts only positive numbers.
- Desktop pipeline (1): through the real `run_import`, a renumbered film and a
  second source's copy (with a provider TMDB ID) resolve to the original entity
  with favorite, resume and the TMDB alias intact.
- `real_profile_upgrade` (opt-in) now also asserts that every merged entity
  redirects in one hop to an active survivor.

Scale probe (release, 30k distinct movies): the resolution pass takes about
0.25 s per import. A deep browse page (offset 10k) went from 47 to 68 ms because
of the one-card-per-entity subquery. A covering index was tried and brought no
measurable gain, so it was not added.

### Validation commands/results

- `just test`: 401 Rust tests, 0 failed, 5 ignored (opt-in probes); 30 frontend tests.
- `just lint` (Clippy all targets `-D warnings` + ESLint) and `just typecheck`: pass.
- `cargo fmt --all`: applied. SQLx offline metadata regenerated with
  `cargo sqlx prepare --workspace -- --all-targets` against a schema built from
  every app/profile migration; three superseded query files were removed.

### Remaining known gaps

- Movies only. Episode rules exist in core but series/episodes/live are not
  canonical entities yet.
- No split/unmerge yet. Lineage keeps both sides' prior state for it.
  `record_cannot_link` has no command or UI.
- The frontend still groups movie cards by raw title with years stripped
  (`movieVersionKey`). That is presentation-only, but it can visually group
  remakes that resolution deliberately keeps apart. It should be narrowed or
  dropped now that server-side resolution exists.
- AI verdicts are validated but no model is invoked, and ambiguity cases are
  not persisted.
- On merge, the latest resume wins. An older completion superseded by a newer
  in-progress rewatch is retained only in lineage.
- Browse filters and search match a merged entity's representative row only.
- Few providers supply TMDB IDs, and M3U titles often lack years, so many true
  duplicates will abstain. That is by design.
- An entity that gains contradictory evidence after merging is excluded from
  further merges but not split.
- Redirects are not generation-scoped (no catalog generations yet).
- Not verified in the running app (`just dev`) or against real profiles. The
  first open after upgrade resolves existing libraries, and running
  `real_profile_upgrade` on a real profile copy is recommended before relying on it.

### Next milestone

Candidates: user-facing "not the same" correction plus split (compensating
lineage operations), narrowing the frontend title grouping, a bounded model job
over persisted ambiguity cases, or extending canonical entities to series and
episodes, where the episode-coordinate rules already apply.

## Fix — refresh crash: translation engine removed

**Symptom:** pressing Refresh crashed the app with `SIGBUS`.

**Root cause:** after an import, the app auto-starts catalog enrichment, which
loads the E5 embedding model through ONNX Runtime (`ort`). `ct2rs` (the NLLB
translation engine) statically linked sentencepiece, which bundles its own
protobuf-lite. Both `libonnxruntime.a` and `libsentencepiece.a` define
`google::protobuf::MessageLite::ParseFromArray`, and the linker keeps one copy,
so ONNX Runtime ran protobuf code from a different version.

- Since `ct2rs` landed (2026-06-11), the file-path load failed with
  "Protobuf parsing failed", so embeddings, semantic search and enrichment were
  silently unavailable.
- Since the model runtime switched to in-memory loading (`8d6da3b`, 2026-09-26),
  the same fault crashed the process.

The cached model was intact (its SHA-256 matched its content address).

**Evidence:** the same model loaded fine in a binary with only ONNX Runtime and
crashed with `SIGBUS` once sentencepiece was linked. After the fix it loads
(384-dimension embedding) inside the desktop package.

**Change:** translation is deferred to a later milestone, so the engine was
removed rather than isolated:

- Removed: `crates/translate`, `ct2rs`/sentencepiece, the translation session and
  commands, the NLLB runtime adapter and registry entry, the player's
  translated/dual modes and translation card, the translation model settings row,
  and `useTranslatedCaptions`.
- Kept: translation tables and cached rows. They remain readable and searchable.
- Legacy `translation`/`dual` default caption modes open as source captions.
- Runtime and registry tests now use the Whisper ID as the generic second model.

**Not verified:** the running app (`just dev`) after the fix. The NLLB files
(about 620 MB in `models/nllb`) remain on disk until the user deletes them.

## P10 — compile → validate → atomic publish

Branch `feat/p10-atomic-publish`, stacked on `fix/remove-translation` → P9 → P8.

### Gaps closed

- **Destructive collapse (data loss):** a refresh whose provider answered `[]`
  or a truncated list *without an error* committed and wiped that kind, for
  example every movie. Refreshes are now rejected when a kind of 50 or more
  items would keep under 10%.
- **No validation step:** the candidate went straight from compile to commit.
  An integrity check now runs after compile and resolution. It covers
  received-versus-persisted rows per kind, unbound movie rows, variants on
  merged entities and dangling tombstones.
- **No notion of an active generation:** `catalog_publications` (migration
  0024) records a `published` row atomically with the catalog. Failed and
  rejected attempts are logged after rollback and never become active.
- **Implicit rollback:** a failed candidate relied on dropping the
  transaction. It is now rolled back explicitly before the failure is recorded,
  so the log write never waits on the candidate's write lock.
- `run_import` is restructured: `build_candidate` returns a typed
  `ImportFailure`, replacing ten copies of the failure-emitting code, and the
  wrapper commits or rolls back, records the outcome and emits once.

### Tests (12 new)

- Core (4): empty endpoint collapse; the 10% threshold; growth, small libraries
  and first imports pass; integrity codes.
- Storage (3): uncompiled rows detected and then cleared by compile; variants on
  merged entities and dangling tombstones detected; failures never become
  active; published rows cannot carry a reason; a deleted source's failure is
  skipped.
- Desktop, through the real `run_import` (5):
  - successful append and replace publish an advancing active generation;
  - a collapsed refresh is rejected with a clear message, and the prior rows,
    favourite, resume, playback, active publication and `last_refreshed_at` are
    unchanged;
  - endpoint-error and empty refreshes are recorded, no provider text is
    persisted, and a later refresh publishes;
  - an in-flight candidate stays invisible to another connection until it
    publishes;
  - after a rejected candidate plus reopen, and after an aborted mid-build
    candidate plus reopen, the prior generation is intact, `integrity_check`
    passes, and the next candidate publishes and survives another reopen.
- Mutation check: with the collapse guard disabled, 2 desktop tests fail.
  Concurrency tests passed 8 of 8 repeated runs.

Validation costs about 29 ms per import at 30k movies (release scale probe).

### Validation commands/results

`just lint`, `just typecheck`, `just test` (407 Rust, 0 failed, 5 ignored; 28
frontend), `cargo fmt --check`, `git diff --check`: pass. SQLx metadata was
regenerated.

### Remaining known gaps

- Publication is per source. There is no profile-wide catalog generation or
  generation-scoped rows yet, so a refresh still replaces rows in place (inside
  its transaction) rather than switching a head pointer.
- The collapse thresholds (50 items, 10%) are v1 and are not user-overridable.
  A provider that genuinely removed most of a kind needs a code change, or two
  refreshes of honest shrinkage.
- First imports (Append) with partial endpoint errors still publish, with a
  warning. There is no prior generation to protect.
- The UI shows the failure message but not publication history. No command
  exposes `catalog_publications` yet.
- Not verified in the running app (`just dev`).
