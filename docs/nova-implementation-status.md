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
