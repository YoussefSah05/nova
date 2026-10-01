# Nova Implementation Status

> **Architecture target (2026-09-28):** Nova's long-term architecture is
> [`docs/architecture/long-term-architecture.md`](architecture/long-term-architecture.md).
> The reconciliation below maps the repository onto its milestones (M00–M31, AI01–AI24). Everything
> after it is the historical per-increment log (P00–P12 and later slices); milestone IDs such as
> `M4.6` there refer to [`docs/archive/master-plan.md`](archive/master-plan.md), not the bootstrap.

## Bootstrap reconciliation — 2026-09-28

### Repository baseline

- Branch / commit: `main` at `1e0a170`. Clean tree.
- Build and tests: `just test` passes: 528 Rust tests/doctests, 0 failed, 5 opt-in ignored; 56
  frontend tests in 16 files.
- Known blockers: none for development. Real-provider, live TMDB, native-device, signing and scale
  qualification remain open gates (see the milestones below).
- Method: a code map, three independent read-only investigations (catalog/naming/search/frontend,
  playback/jobs/security, AI), each key claim re-checked in source. One investigation claim was refuted:
  ffmpeg stderr is already replaced with a fixed message before reaching captions
  (`crates/media/src/audio.rs:143`).

### Milestone reconciliation

| Milestone | Status | Evidence | Remaining gap |
|---|---|---|---|
| M00 Current-main stabilization | COMPLETE | P00 section below; fixes plus regressions. | — |
| M01 Source-aware audit | COMPLETE | M1 audit below; superseded as the status map by this reconciliation. | — |
| M02 Model library domain/persistence | COMPLETE | `nova-core::models`, app migrations 0002/0003, `model_registry.rs`. | Descriptor set is fixed at compile time (Whisper, E5); `artifact_revision` always null. |
| M03 External model storage | COMPLETE (unqualified) | Bookmarks in `mac_model_library.rs`; offline → `unavailable`, reconnect restores. | No physical-drive or sandbox qualification; downloads always go to the internal library. |
| M04 Runtime manager / leases | COMPLETE (backend) | `model_runtime.rs`: shared load, leases, LRU eviction, residency budget. | `model_runtime_status` / `unload_model` have no React consumer. |
| M05 Resource governor | PARTIAL | `resource_policy.rs` lanes; playback guards suspend background classes. | Channel-logo matching, series refetch, `url-tvg` registration and on-demand TMDB/Xtream fetches bypass admission. A webview reload now releases the old page's playback guards (M29). |
| M06 Canonical domain / provider boundary | PARTIAL | Movies: `canonical_entities` / `stream_variants` (0021). Series identity, favorite/watchlist and TMDB match: `kind = 'series'` entities via `series_bindings` (0036); `SeriesSummary.entityId`. Secret-free legacy DTOs. | Seasons, episodes and live are provider rows; series screens still key on `(sourceId, seriesTitle)`. |
| M07 Naming / normalization / classification | PARTIAL | Naming v1 parser; raw title/group kept (0026); `naming::display_title`. | Media type is endpoint/URL-heuristic only, M3U defaults to Live, no `Unknown` in `MediaKind`. Movie cards use a hand-written SQL COALESCE instead of `display_title`; series `raw` fallback is set to the parsed title. |
| M08 Fingerprints / candidate blocking | PARTIAL | `catalog::{fingerprint, reconcile}`; blocking used by movie resolution. | Refresh still deletes and reinserts. Unchanged/changed/new/deleted classification is unwired; movie/live embeddings and enrichment status are discarded on every refresh. |
| M09 Entity resolution / identity history | PARTIAL | P09: deterministic resolver, redirects, lineage, cannot-link (movies). Series identity survives provider renames (by series ID, or by episode URLs for M3U). | No split or user correction; no cross-source series resolution; episodes/live have no entities. |
| M10 Atomic catalog snapshots | PARTIAL | P10: validate + per-source `catalog_publications`. | No profile-wide generation/head; protected snapshot store (`catalog-ingest`) unwired. |
| M11 TMDB identification / progressive enrichment | PARTIAL | Movies and series: entity-owned matches with provenance and trusted IDs (`tmdb:movie`, `tmdb:tv`); tiered on-demand facts (0035). | No episode-level identification; no manual match correction. |
| M12 Canonical naming / localization | PARTIAL | Localized + original titles stored (0028); one policy function. | Policy not applied uniformly (see M07); no correction tier. |
| M13 Canonical search | PARTIAL | `search_movies` over `movie_search` FTS with aliases, exact-first; every semantic leg falls back to lexical on failure. | Series/live search is LIKE over legacy DTOs; no cross-kind search; a slow semantic leg still delays lexical results. |
| M14 PlaybackCoordinator / resolver / selector | PARTIAL | `PlaybackCoordinator` (Rust) owns the native session's resume, progress, completion, end, next episode and recovery; `VariantSelector` (`catalog::variants`) ranks a movie's sources. | Media addressed by row ID; series/live have one source each; web fallback path still React-owned; not verified on device. |
| M15 Recovery / QoE | PARTIAL | Bounded recovery (same item once, then other sources, ≤ 5 per 10 min per title) and per-source health (`variant_health`, 0039). | No decoder fallback; no QoE metrics (startup latency, rebuffering); buffering stalls aren't failures. |
| M16 Jobs / bounded queues | PARTIAL | Persisted `JobQueue` for import, EPG, enrichment: dedup, cancel, startup recovery. | No retry/backoff; `auto_refresh_hours` never scheduled; several bare spawns (M05). |
| M17 Streaming transcription | PARTIAL | VOD caption sessions: segment-aligned windows, Realtime leases, subprocess kill on cancel. | Not playback-fed PCM/VAD; live transcription never merged (see skipped work). |
| M18 Timed translation | MISSING | Engine removed (protobuf clash); legacy rows readable. | Engine choice pending. |
| M19 Embeddings / dialogue search | PARTIAL | Per-title dialogue search seeks playback (`TranscriptPanel` → `seekTo`). | Entity vectors belong to provider rows, not entities; no cross-library dialogue search. |
| M20 Command-K typed intents | MISSING | `CommandPalette` is search + navigation. | No intent model or validator. |
| M21 Smart discovery / personalization | PARTIAL | TMDB related/collections/trending intersected with the library. | No embedding similarity or history-based personalization. |
| M22 Frontend canonicalization | PARTIAL | Movies and series address canonical entities (series by `seriesId`: detail, episodes, resume, favourites, art/facts/related). | Live, player, transcripts and episode/live Continue Watching play by row ID; client `seriesGroupKey`/`prettyBrand` grouping remains. |
| M23 Series / autoplay | PARTIAL | Episode progress owned by the series entity per durable episode key (0037); resume, Up next and series Continue Watching read it; autoplay uses the normal play path. | In-player next episode chosen in React; episodes aren't canonical entities; no cross-source episodes. |
| M24 Live / EPG boundary | PARTIAL | Programmes owned per feed (0040); feed spooled with a 512 MiB cap, staged batches (0041), one short publish; empty feeds refused; one guide feed per channel. The guide is on screen: live cards, hover preview, bundle channels, the live player and its channel strip; refreshed after every import; Settings shows each source's guide status. | Channels still matched by `tvg_id` (no canonical channel/ChannelMatcher); no gzip feeds; no scheduled refresh; no grid guide view. |
| M25 Security / privacy | PARTIAL | TMDB credential write-only; source and stream URLs never cross IPC; the stream proxy is session-scoped (opaque per-session resource ids, no open relay, no local upstreams incl. DNS); strict CSP; every log line and UI-bound error has URLs reduced to `scheme://host`. | Provider logo/poster URLs still reach the webview (opaque artwork ids not done); CORS `*` on token routes (tokens unguessable); `Debug`-formatted URLs and panics bypass the redactor; private/LAN upstreams allowed. |
| M26 Migration-chain qualification | PARTIAL | Upgrade-chain tests; `real_profile_upgrade` (opt-in). | Never run against a real profile copy. |
| M27 Scale qualification | PARTIAL | 30k release scale probes; `tools/catalog-scale`. | No 100k/250k app-level measurements. |
| M28 AI failure matrix | PARTIAL | Missing/offline model and cancellation tests exist. | No matrix; see M13 inference-failure case. |
| M29 macOS lifecycle | PARTIAL | Reload releases the old page's playback (page epochs; in-flight opens undo themselves); quit writes progress and stops remuxes/transcription; sleep pauses, wake reconnects a playing live channel (`lifecycle.rs`). | Not verified on device; AirPlay across sleep unhandled; no window-state/reopen handling beyond the window-state plugin; no display-sleep or App Nap handling. |
| M30 Accessibility / signing | PARTIAL / MISSING | 46 `aria-label`s over 78 components. | Keyboard handling minimal; no signing, notarization or entitlements. |
| M31 Final convergence review | NOT STARTED | — | — |

| AI milestone | Status | Note |
|---|---|---|
| AI01 Model domain / libraries | COMPLETE | = M02 |
| AI02 External storage | COMPLETE (unqualified) | = M03 |
| AI03 Registry / discovery | PARTIAL | Fixed descriptor set; no revision/digest. |
| AI04 Runtime / leases | COMPLETE | = M04 |
| AI05 Governor integration | COMPLETE for ASR/embeddings | Reasoner would bypass it if wired as-is. |
| AI06 ASR baseline | COMPLETE | Whisper subprocess, Core ML, DTW timestamps. |
| AI07 Streaming transcription | PARTIAL | = M17 |
| AI08 / AI09 Translation | MISSING | = M18 |
| AI10 Entity embeddings | PARTIAL | Provider-row owners; lost on refresh. |
| AI11 Transcript embeddings | COMPLETE (per title) | Lazy segment embedding, model-keyed. |
| AI12 Semantic search | PARTIAL | Optional leg with lexical fallback; vectors on provider rows (AI10). |
| AI13 Dialogue search | PARTIAL | Within the playing title only. |
| AI14 Reranker | MISSING | Optional by design. |
| AI15 Catalog ambiguity reasoning | LIBRARY-ONLY | `catalog-reasoner` and `assess_ai_verdict` unused by the app. |
| AI16 Command-K | MISSING | = M20 |
| AI17 Smart discovery | PARTIAL | TMDB-based, not AI. |
| AI18 Personalization | MISSING | |
| AI19 Derived content | MISSING | "About this title" never merged. |
| AI20 AI settings UX | PARTIAL | Installed / not installed plus external library state. |
| AI21 AI failure UX | MISSING | Runtime errors map to generic `Internal`; no capability-specific states. |
| AI22 Evaluation harness | PARTIAL | `tools/catalog-eval` (reasoner smoke only); no ASR/search eval. |
| AI23 Provenance / versioning | PARTIAL | Model ID stored on transcripts/embeddings; no revision. |
| AI24 Privacy / offline | PARTIAL | No LLM prompts in the app; credential exposure is a UI/IPC issue (M25). |

### Skipped or stranded work

- `feat/local-llm-about` (6 commits, June: llama.cpp "About this title") and
  `feat/phase-9-live-transcription` (2 commits: rolling live captions) were never merged and are 77+
  commits behind `main`. They predate the model registry, runtime manager and governor, so merging
  them as-is would create AI paths that bypass leases and admission. Treat them as reference only.
- `update/nova-next` and `tooling/orchestrator-v2` hold unmerged developer tooling, not product code.

### Coexisting legacy and new systems

- Movie state is written to entities and mirrored to `vod_items` flags / `watch_history`; movie TMDB
  data is stored on `entity_external_matches` and mirrored onto rows (read paths use the mirror).
- Enrichment `dedup_group` clustering coexists with P09 entity resolution; `dedupGroup` still ships.
- Legacy `list_vod`, `count_vod`, `set_vod_*`, `semantic_search_catalog` remain registered (no UI caller).
- Two Continue Watching queries; `get_profile_overview.lastTitle` uses the legacy one with raw names.
- Unwired parallel pipeline: `catalog-ingest`, `catalog-reasoner`, `catalog::{snapshot, fingerprint,
  reconcile, intelligence}` beside the production importer. No `*-v2` systems exist.

### Architecture invariants currently violated

1. ~~Credentials reach React~~ — fixed by the secret-free IPC boundary below, except the loopback
   playback locator (`/proxy?url=…`), which still carries the upstream URL.
2. ~~Deterministic search must survive AI failure~~ — fixed by the lexical fallback below.
3. ~~Playback truth belongs to Rust~~ — resume, progress, completion and next episode moved to the
   playback coordinator (native path); variant selection is still a single SQL pick.
4. User state belongs to canonical identity (§3): series favorite/watchlist and episode progress are
   now entity-owned; live state is still keyed to provider rows and restored by URL matching.
5. Jobs are bounded (§29): the EPG download has no size cap.

### Answers to the §60 questions

Q1–Q5 and Q18–Q23 are answered by the tables above. Q6: `stream_url`, `Source.url`, numeric provider
row IDs and `(sourceId, seriesTitle)` keys reach React. Q7: movies — SQL COALESCE of TMDB title,
entity title, raw name; series — `display_title` over `series_meta`; live — polished provider name.
Q8: localized TMDB titles work for matched movies/series. Q9: yes for movies; series lack a true raw
fallback; live has no layers. Q10: movies only. Q11: yes (opaque `mov-…` IDs; TMDB IDs are matches,
not identity). Q12: yes for movies (tested upgrade chain); non-movie state relies on URL/title
restore. Q13: yes when the model is absent; no when inference fails. Q14: no. Q15: commands are
Rust-token-checked; state events are filtered in React. Q16: yes for persisted jobs. Q17: running
model work finishes its current bounded unit; new background work is suspended. Q24: no LLM prompts
exist; credentials reach the webview instead. Q25: no — discovery intersects with the library. Q26:
yes, per title. Q27: movies abstain honestly; unmatched series fall back to the parsed title. Q28: no.
Q29: yes, except movies. Q30: see below.

### Secret-free IPC boundary (M06/M25) — 2026-09-28

Branch `fix/secret-free-ipc`. Chosen first because it was a live invariant violation (Xtream
credentials shown in Settings and held in webview memory) and every later canonical DTO slice needs
secret-free contracts.

- `nova_core::Source.url` stays in Rust; it serializes only as `displayUrl` =
  `display_source_url(url)`: `scheme://host[:port]` for URLs (user info, path, query, fragment
  dropped), or a bare file name for local paths and malformed URLs. Deserialization still reads
  `url`; storage builds these structs from SQL rows, so no repository or schema changed.
- `stream_url` on `Channel`, `VodItem` and `SeriesEpisode` is `#[serde(skip_serializing)]`. React
  never read it; playback already resolves locators in Rust (`resolve_stream`).
- Frontend: `Source.displayUrl` replaces `url` in `types/index.ts` and the Settings sources row;
  `streamUrl` removed from the three TS types and from test fixtures.
- Tests: `crates/core/tests/ipc_secrets.rs` round-trips each DTO from credential-bearing JSON and
  asserts no user, password or `streamUrl` survives, plus 14 `display_source_url` cases (IPv6,
  `@` in passwords, scheme-less and backslash URLs that the add-source form accepts).
- Independent review: no Critical/High. **Medium, confirmed and fixed:** a malformed URL without
  `://` (e.g. `http:host/get.php?password=…`, which `new URL` accepts and `add_source` stores even
  though its import then fails) kept its query in the file-name fallback. Both fallbacks now drop
  query, fragment and user info; a bare `user:pass@host` was found and covered during the fix.
  Low, recorded as gaps: provider-supplied logo/poster URLs, and the `/proxy?url=` playback locator.
- Validation: `just lint`, `just typecheck`, `cargo fmt --check`, `git diff --check` pass;
  `just test` passes (531 Rust, 0 failed, 5 opt-in ignored; 56 frontend) before the review fix;
  `nova-core` (142 tests) and lint re-run after it. Not verified in the running app (`just dev`).

### Lexical fallback for semantic search (M13 / AI12) — 2026-09-28

Branch `fix/search-lexical-fallback`.

- All five search commands (`search_transcript`, `search_movies`, `semantic_search_catalog`,
  `semantic_search_channels`, `semantic_search_series`) computed lexical results first, then used `?`
  on query/segment embedding and vector loads, so an inference error or a busy/offline model threw
  the lexical hits away and the palette (which maps errors to `[]`) showed nothing.
- Each semantic leg is now a helper that returns plain hits: its fallible body (`try_semantic_*`) runs
  through `semantic_leg`, which logs the error (`warn`, no query text or URLs) and returns no semantic
  hits. Because the helpers return `Vec`, not `Result`, a command can no longer `?` a semantic failure
  — the invariant is enforced by the type, not only by tests. Lexical, DB and hydration errors still
  surface. Ranking, fusion, `k` values, lease lifetimes, admission class and cancellation are unchanged.
- Tests: `semantic_leg_passes_hits_through_and_swallows_failures` (runtime-busy, not-found and
  inference errors); the existing ranking tests cover the unchanged lexical/fused ordering.
- Independent review: no Critical/High. **Medium, confirmed and fixed:** the first tests exercised
  only the helper, so reverting a command to `?` would not fail them; fixed by making the helpers
  infallible by type (reviewer's suggestion). Medium, pre-existing, recorded as a gap: a slow semantic
  leg (e.g. first-time segment embedding of a long transcript) still delays lexical results — no
  timeout. Low, recorded: repeated `warn` logs per keystroke while the model is broken; the
  transcript panel's search effect lacks a stale-response guard.
- Validation: `just lint`, `just test` (533 Rust, 0 failed, 5 opt-in ignored; 56 frontend),
  `cargo fmt --check`, `git diff --check` pass; after the review fix, `just lint` and the desktop
  library tests (105 passed) were re-run. Not verified in the running app (`just dev`).

### Canonical series, slice 1: identity and series-level user state (M06/M09) — 2026-09-28

Branch `feat/canonical-series`.

- **Migration 0036.** `canonical_entities.kind` now admits `'series'`. SQLite can't alter a CHECK, so
  the table is rebuilt the 0010 way: all seven children (`stream_variants`, `entity_watch_state`,
  `entity_lineage` with its AUTOINCREMENT and append-only triggers, `entity_redirects`,
  `entity_aliases`, `entity_cannot_link`, `entity_external_matches`) are parked in FK-less copies,
  dropped, recreated with byte-identical DDL and reloaded. New `series_bindings(source_id,
  binding_key, entity_id, series_title)`; `series_title` is NULL while unlisted. Additive otherwise.
- **Binding key** (`nova_core::catalog::media::series_binding_key`): the Xtream series ID from the
  catalog sentinel `xtream://series/<id>`, else the exact parsed title (M3U has no series ID).
- **Compiler** (`CanonicalRepository::compile_series_tx`, in the import transaction after movie
  resolution): re-bind by series ID (even under a new title); upgrade a title-keyed binding to the ID
  once the sentinel appears; keep a title's previous binding when compiled without its sentinel;
  re-bind by title; else create an entity seeded from the legacy flags. Every listed series ends up
  bound. The entity follows the current title; per-row flags are re-derived from it (mirror).
- **Renames:** Xtream renames keep identity through the series ID — including a series that was
  opened, which the legacy URL/title restore lost. M3U renames are recognised by the refresh restore
  through unchanged episode URLs and the title-keyed binding is moved
  (`carry_series_renames_tx`), so state is carried, not copied.
- **Write-through:** `SeriesRepository::set_favorite` / `set_in_watchlist` update the entity, then
  the rows. `init_profile_db` backfills existing profiles (`backfill_series`, `BEGIN IMMEDIATE` per
  source); the steady-state check costs ~20–30 ms at 100k episodes (covering indexes both sides).
- **Tests (9):** core key namespaces/stability; upgrade from schema 35 preserving every movie
  identity table, the lineage trigger, integrity and FK checks, and seeding series by ID/title
  (idempotent reopen); through the real `run_import`: opened-then-renamed Xtream series keeps entity
  and flags (fails without the compile step), title→ID upgrade then rename, entity-owned toggles
  incl. clearing, M3U rename carries identity and a revert brings back no stale state (fails without
  the carry-over), a sentinel-less compile keeps an opened series' identity, source deletion keeps
  entities.
- **Independent review:** no Critical/High; migration verified by schema diff. **Medium, confirmed
  and fixed:** (1) an M3U rename seeded a copy and left stale state on the old entity, which a title
  revert brought back — fixed by moving the binding; (2) a compile without the sentinel forked an
  opened series' identity — fixed by reusing the title's previous binding.
- **Known gaps (Low, recorded):** an M3U rename of a series with *no* favorite/watchlist isn't
  detected (the restore snapshots flagged series only), so it gets a fresh, empty entity; the movie
  entity APIs (`set_favorite`, `record_progress`, …) don't reject series IDs (not reachable until
  series IDs cross IPC); the mirror rewrites every episode row per import; `MAX(stream_url)` picks
  the sentinel textually when two series share a title; episode resume and series TMDB matches are
  not entity-owned yet; series DTOs don't carry the entity ID.
- **Validation:** `just lint`, `just test` (540 Rust, 0 failed, 5 opt-in ignored; 56 frontend),
  `cargo fmt --check`, `git diff --check` pass after the review fixes. The first full run failed only
  because the disk was full; the 93 GB debug cache was cleaned with the user's approval and the
  suite rerun from scratch. Not verified in the running app (`just dev`) or on a real profile copy.

### Canonical series, slice 2: entity-owned series TMDB matches (M11) — 2026-09-28

Branch `feat/canonical-series-tmdb`. No migration.

- `SeriesRepository::apply_tmdb` records each decision on the series entity
  (`entity_external_matches`, namespace `tmdb:tv`, same provenance columns as movies) under
  `BEGIN IMMEDIATE`; a title no entity holds any more (renamed mid-matching) is skipped and
  re-queued under its current title. `series_meta` and the episodes' TMDB genre are now a mirror
  (`ExternalMatchRepository::mirror_series_tx`), re-derived per entity on apply and per source after
  each import, where rows no entity-owned match backs (a renamed series' old title) are deleted.
  All readers (`list_series`, title facts/art, Continue Watching, discovery) are unchanged.
- The match queue (`list_for_tmdb`) is entity-based with the same retry rules (never decided, older
  matcher, other language, unresolved past the retry age, unresolved-by-search with a new provider id).
- Upgrade: `backfill_series_matches` (after `backfill_series`, on open) carries legacy `series_meta`
  decisions onto the bound entities as **provisional** (matcher version 0): they keep showing and the
  queue re-confirms each one (mostly from the TMDB response cache), since a legacy row keyed by title
  may describe a different series that used the title before.
- `SeriesSummary.entityId` (core + TS). Movie `set_favorite` / `set_in_watchlist` / `record_progress`
  refuse series IDs (`movie_entity_conn`).
- Tests (7 new or extended): match entity-owned, mirrored onto a renamed series with its genre and
  not re-queued (re-queued for a newer matcher/language; fails without the import mirror); unresolved
  retried on a new provider id; movie APIs refuse a series; a newcomer under a renamed series' old
  title doesn't show or inherit its match, also after reopening (fails without the cleanup);
  unresolved clears the mirror and only the TMDB genre; a decision for an unheld title leaves no
  orphan; upgrade carries a matched and an unmatched legacy decision and queues all for
  re-confirmation.
- Independent review: no Critical/High. **Medium, confirmed and fixed:** (1) stale old-title
  `series_meta` rows could show on — and be copied onto — a different series later using that title;
  fixed by the post-import cleanup, dropping the unbound mirror-only write path, and provisional
  carry-over; (2) the read-first transaction failed with SQLITE_BUSY when overlapping an import;
  fixed with `BEGIN IMMEDIATE`. Low, recorded: queue items carry the title, not the entity (a title
  reassigned mid-matching gets the other series' decision until re-queued); the per-entity mirror
  scans bindings (`?1 IS NULL OR …` defeats the entity index) and the import mirror rewrites every
  matched series' episodes; `movie_detail`/`record_cannot_link` accept series IDs without corrupting
  state; discovery's `library_series` reads `series_meta` by TMDB id (now only live rows).
- Validation: `just lint`, `just test` (546 Rust, 0 failed, 5 opt-in ignored; 56 frontend),
  `cargo fmt --check`, `git diff --check` pass. Not verified against the live TMDB API or in the
  running app (`just dev`).

### Canonical series, slice 3: entity-owned episode progress (M23/M06) — 2026-09-28

Branch `feat/canonical-episodes`.

- **Migration 0037:** `series_episodes.episode_key` (the durable `episode_binding_key`: Xtream episode
  ID, else a locator digest), set at insert in both series write paths; `episode_watch_state` keyed
  `(series_entity_id, episode_key)` with last known season/episode, position, duration, completion,
  caption mode. Additive.
- **Write:** `record_progress('episode', row)` upserts the entity state (then the legacy
  `watch_history` mirror) in one `BEGIN IMMEDIATE` transaction.
- **Read:** `resume_position`, `episode_progress` (detail page), `continue_series` (series rail,
  `get_series_resume` Up next) read the entity state and find the episode's current row by key. The
  latest episode that can be placed decides a series' card; a finished episode with a known position
  gives Up next from the rows. `series_with_pending_episodes` (background re-fetch after a refresh)
  lists series still listed by their sentinel alone that have progress, by current title.
- **Upgrade:** `backfill_episode_state` (on open) keys pre-0037 rows per source and seeds progress
  from `watch_history` and `pending_episode_history`, never overwriting newer entity state.
- **Legacy mirror kept:** `watch_history` episode rows, snapshot/park/reattach, and the overview's
  `list_continue_watching` still use them; removal is a later step (M3U rename detection depends on
  the snapshot).
- **Tests (6 new or extended):** progress survives a refresh + provider rename, the re-fetch runs
  under the new title, resume/rail/per-episode progress come back by key (fails without the entity
  write); a finished episode offers the next one; an unlisted latest episode gives way to the
  previous one and orphans aren't re-fetch candidates (fails without the fix); a progress write
  waits for a concurrent writer (fails with "database is locked" without the fix); upgrade seeds
  from row-keyed and parked history and keys every row; the continue-watching fixture now reaches
  the rail through the upgrade path.
- **Independent review:** no Critical/High. **Medium, confirmed and fixed:** (1) the read-first
  deferred transaction failed instantly under a concurrent writer (also latent for movies since P07)
  — `BEGIN IMMEDIATE`; (2) one unplaceable latest entry hid a series' whole card — fall through to
  the next entry; (3) the re-fetch list counted permanent orphans, crowding out series that needed
  it — restricted to sentinel-only series. Low, recorded: parked history seeded by title can land on
  another series that took the title (upgrade only); an unbound series' progress is only mirrored
  until the next open; the overview counts still read the legacy table; keys aren't source-scoped
  (safe while series entities are per source); duplicate M3U URLs share progress; deleted sources
  leave unreachable state.
- **Validation:** `just lint`, `just test` (550 Rust, 0 failed, 5 opt-in ignored; 56 frontend),
  `cargo fmt --check`, `git diff --check` pass. Not verified in the running app (`just dev`).

### Gap fixes for the canonical series slices — 2026-09-28

Branch `fix/series-canonical-gaps`. Closes the Low findings recorded by the three series reviews.

- **Episode progress:** parked history is no longer seeded onto an entity by title at upgrade (another
  series may hold the title); it reaches the entity through its episode row when re-attached — in the
  import now *after* the series compiler, so a renamed + credential-rotated series is covered — and
  legacy parked series stay in the background re-fetch list. Rows an earlier build seeded that way are
  dropped for re-attachment. `resume_position` falls back to the legacy row when entity state is
  missing. The overview (`list_continue_watching`) reads entity progress under display titles.
- **Renames:** M3U renames are also detected through watched episodes (not only favourite/watchlist).
- **TMDB:** queue items and writes carry the series entity, so a rename during matching can't redirect
  a decision. The series mirror uses per-scope statements that hit their indexes.
- **Compiler:** the flag mirror rewrites only stale rows; a title shared by several sentinels binds to
  the smallest numeric series ID (text order picked "20" over "10"). `record_cannot_link` is
  movies-only. Migration 0038 indexes `series_episodes(source_id, stream_url)` for the per-URL
  refresh lookups.
- **Tests (8 new):** watched M3U rename keeps progress; re-attached parked history reaches the entity;
  a decision follows its entity through a mid-match rename; the overview reads entity progress under
  display titles; resume legacy fallback; cannot-link refuses a series and sentinels bind numerically
  (IDs 10/20, which text order gets wrong); parked progress of a renamed + rotated series reaches the
  rail (fails without the post-compile re-attach); upgrade test updated.
- **Independent review:** no Critical/High. **Medium, confirmed and fixed:** (1) the import re-attached
  parked history before the compiler bound the new titles, dropping the entity copy; (2) the sentinel
  test used IDs whose text and numeric order agree. Low, recorded: split series / reused URLs choose
  the rename target by snapshot order; parked rows keep their old title, so a renamed sentinel series
  isn't re-fetched automatically (resume still works when opened); duplicate M3U URLs yield one
  overview card per row; leading-zero or 20+-digit series IDs key differently (not produced by the
  importer); decisions for a merged series entity are dropped (no series merges exist).
- **Validation:** `just lint`, `just test` (557 Rust, 0 failed, 5 opt-in ignored; 56 frontend),
  `cargo fmt --check`, `git diff --check` pass. Not verified in the running app.

### Series frontend on canonical IDs (M22) — 2026-09-28

Branch `feat/series-frontend-canonical`.

- Series commands take `seriesId` (the series entity): `list_series_episodes`, `get_season_episodes`,
  `get_series_resume`, `set_series_favorite`, `set_series_in_watchlist`, and the series
  art/facts/related subject (`{ kind: "series", seriesId }`). Rust resolves where the series is
  listed now (`series_key` → `CanonicalRepository::series_source`), so references made before a
  provider rename or refresh stay valid; an entity that isn't a listed series is `NotFound`
  (art and related titles stay never-failing and return empty).
- Frontend: detail page, rails, favourites/watchlist (moved to `features/browse/toggles.ts`) and
  title art send `seriesId`; React and React Query keys follow the entity (`seriesKey`); a finished
  import/enrichment also invalidates the series detail queries (keyed by the entity, which survives
  while its episode rows are replaced); an unbound series isn't queried and its toggles give a clear
  message instead of sending `null`. `docs/ipc-contract.md` documents the rule.
- Tests (6 new): the detail hooks send only `seriesId`; unbound series aren't queried; toggles
  address the entity and refuse an unbound series; a refresh invalidates series detail data; the
  subject deserialises only in its canonical form; a series ID resolves to its current listing
  after a rename and movie/unknown IDs are `NotFound`. A pre-existing flaky storage assertion
  (random entity IDs containing "593") was made deterministic in its own commit.
- Independent review: no Critical/High; all callers migrated. **Medium, confirmed and fixed:** series
  episode caches, now keyed by the surviving entity, weren't invalidated by a refresh, so play could
  hit a replaced row for up to five minutes. Low, fixed: unbound-series toggles sent `null`; art and
  related titles broke their never-fails contract. Low, recorded: `get_related_titles` doesn't check
  that its `source_id` matches the series' source (the only caller passes it); `series_source` relies
  on one binding per series entity.
- Validation: `just lint`, `just typecheck`, `just test` (559 Rust, 0 failed, 5 opt-in ignored; 61
  frontend), `cargo fmt --check`, `git diff --check` pass. Not verified in the running app.

### Playback coordinator, slice 1 (M14) — 2026-09-29

Branch `feat/playback-coordinator`.

- **`PlaybackCoordinator`** (`apps/desktop/src-tauri/src/playback_coordinator.rs`) owns the native
  (libmpv) session's policy on one ordered actor task: engine wakeups only poke it (it snapshots the
  engine itself, in order; `mpv:state` is emitted independently and never waits on it), and
  `begin`/`finish` go through the same queue. It seeks to the resume point, persists progress every
  5 s while playing, on pause (also from buffering), at the end, on close and when the next item has
  opened (the previous item's last observed position), decides completion (≥ 95 %), and announces
  `playback:ended { token, nextEpisode, autoplay }` — the next-episode lookup and the autoplay
  setting are read off the queue, at the end.
- **`SessionPolicy`** is the pure decision core: a VOD item is ready only once the engine knows its
  duration; the resume seek is confirmed against the reported position and retried (3 seeks, 2 s
  apart) — until then the stored position is never overwritten, even by the final write; time is
  monotonic.
- **Engine (`nova-mpv-player`):** a new load reports `opening` until libmpv's `FILE_LOADED` for it
  (the new token no longer carries the previous file's state), and the end of a file is read from
  `eof-reached` (under `keep-open` libmpv pauses on the last frame instead of `END_FILE`, so
  autoplay never fired before — in the React player too).
- **Frontend:** the native path drops its resume effect, 5 s progress timer, next-episode lookup and
  ended handling; `usePlaybackEnded` opens the episode it is handed, for its own token and not while
  another open is in flight; caption changes go to `playback_caption_mode`. The hls.js fallback keeps
  its own logic.
- **Tests (12 new or reworked):** 8 policy tests (readiness before duration, confirmed/retried/abandoned
  resume that never overwrites the stored point, 5 s writes, pause from playing and buffering, final
  write, completion, late snapshots after the end, live, errors); a desktop test driving the actor
  against a real profile database (resume kept while unconfirmed, progress written by Rust, a
  replaced token ignored, end → completed + next episode + autoplay announced); 2 frontend tests for
  the autoplay handoff (other tokens, autoplay off, no next, open in flight).
- **Independent review:** no Critical. **High, confirmed and fixed:** (1) the engine reports
  "playing" while still loading, so the one-shot resume seek failed and the stored point was
  overwritten ~3 s later; (2) the coordinator lock was held across DB writes in front of the
  `mpv:state` stream; (3) `ended` never fired under `keep-open`, so autoplay was dead.
  **Medium, fixed:** a new token briefly carried the previous file's state; out-of-order wake tasks;
  the previous session was dropped before a possibly failing open; `begin` could replace a session
  without finishing it. **Low, fixed:** an end event could override a user's in-flight switch;
  autoplay read at the start of the item; no pause write from buffering; seek failures only at debug.
  Low, recorded: while AirPlay plays, mpv is paused and the close writes the pre-AirPlay position
  (as before); progress-write errors are logged, not surfaced.
- **Validation:** `just lint`, `just typecheck`, `just test` (568 Rust, 0 failed, 5 opt-in ignored; 63
  frontend), `cargo fmt --check`, `git diff --check` pass. **Not verified on device:** the engine's
  `FILE_LOADED`/`eof-reached` phase changes and autoplay need a run with a real stream (`just dev`).

### Playback coordinator, slice 2: variant selection and recovery (M14/M15) — 2026-09-29

Branch `feat/variant-selection`.

- **Variant selector** (`nova_core::catalog::variants::rank_variants`, pure): explicit choice, no
  recent failure, the profile's preferred audio language (provider prefixes mapped from country to
  language — `UK`/`US` → en, `BE` → fr —, `VF*` → fr, then multi-language, then unknown; `en-US`,
  `eng`, `fra` all understood), picture quality (full HD first: IPTV 4K tends to stall rather than
  fail, so recovery can't help), then the previous lowest-row rule. `resolve_movie_playback` and
  recovery use it (`CanonicalRepository::resolve_playback_with`, `playback_candidates`).
- **Health:** migration 0039 `variant_health`; the coordinator records a source as working after
  20 s of real playback and as failing on an engine error; a failure within 24 h and after the last
  success demotes it.
- **Recovery** (coordinator): on an error, one retry of the same item per title, then the next-ranked
  untried source (≤ 3 switches), never more than 5 recoveries per title in 10 minutes (so a source
  that plays and fails again can't loop), `playback:recover { token, mediaKind, mediaId, attempt,
  otherSource }`; an open the user starts resets the title's recoveries. The player buffers events
  for a session it doesn't hold yet (a very fast failure can beat the open's reply), reopens the
  item (same id) or opens the other source, and shows "Retrying…" / "Trying another source…".
- **Tests (12 new or reworked):** selector (language, country prefixes, 3-letter codes, quality,
  explicit choice, failures); policy (healthy only after sustained play, failure once); a desktop
  test through a real profile DB (retry → failover → healthy after 20 s → demotion → all sources
  tried → user replay resets → a live play-then-fail loop capped at 5; fails if a healthy source
  resets the cap); frontend (an event announced before its open settles is handled, stale ones
  dropped).
- **Independent review:** no Critical. **High, confirmed and fixed:** a source that played and then
  failed was retried forever (the chain reset on reaching playback). **Medium, fixed:** an exhausted
  chain blocked recovery until restart; with 3+ sources only 2 were tried; a very fast failure's
  recover event was dropped; country prefixes and 3-letter codes didn't match languages. **Low,
  fixed:** a stale recovery note on the next user open; 4K ranked above full HD. Low, recorded:
  the error overlay can flash before a recovery clears it; an offline laptop demotes sources for up
  to 24 h (until one plays); the in-entity explicit choice has no UI yet.
- **Validation:** `just lint`, `just typecheck`, `just test` (575 Rust, 0 failed, 5 opt-in ignored; 64
  frontend), `cargo fmt --check`, `git diff --check` pass. **Not verified on device** (recovery needs a
  real failing stream).

### Live / EPG boundary (M24) — 2026-09-29

Branch `feat/epg-boundary`.

- **Feed ownership:** migration 0040 gives `programs` an `epg_source_id` (cascades from
  `epg_sources`, so deleting a source removes its guide). A feed's refresh replaces only its own
  rows; programmes written before ownership (no feed) go only for the channels the new feed covers.
  Two feeds covering a channel (a provider feed and a community one, or two sources) no longer
  overwrite each other.
- **Bounded ingest** (`commands/epg.rs`): the body is spooled to an anonymous temp file, refused past
  512 MiB (declared `Content-Length` or while streaming). A blocking thread parses it (streaming
  `quick_xml`) into 500-programme batches through a channel of 4. Batches are staged in short
  writes (0041 `program_staging`), then published with the feed's logos in one short transaction.
  The write lock is never held while parsing. A feed that parses to no programmes (an HTML error
  page, `<tv></tv>`, a compressed body) fails and keeps its previous guide.
- **Source-aware reads:** `epg_now_and_next` / `epg_range` take an optional `sourceId` (the Live tab
  passes it when every channel on screen shares one source). A channel shows one feed's guide: the
  preferred source's first feed covering it, else the first covering feed, legacy rows last. So now,
  next and the grid never mix feeds, and lookups keep their index-ordered `LIMIT 1`.
- **Tests (7 new):** storage (feeds never overwrite each other, the source is preferred, the grid has
  no duplicates, the source cascade, legacy rows replaced per channel, one feed per channel within a
  source); desktop (a multi-batch ingest replaces only its feed and keeps logos, an empty or HTML
  feed keeps its guide and leaves staging clean, the size cap is enforced by a loopback HTTP server
  for declared, chunked and under-cap bodies).
- **Independent review:** no Critical or High. **Medium, confirmed and fixed:** the write lock was
  held for the whole parse (other writers could hit the 5 s busy timeout); an empty or non-XMLTV
  response wiped the feed's guide; the grid duplicated programmes when two feeds of one source
  covered a channel. **Low, fixed:** now and next could come from different feeds; the preference
  ordering disabled the index-ordered lookup; legacy rows won ties with no preference; spooling
  wrote with blocking I/O on the async runtime; the refresh key could collide on ids with spaces.
  Mutation check: removing the empty-feed guard fails its test.
- **Validation:** `just lint`, `just typecheck`, `just test` (581 Rust, 0 failed, 5 opt-in ignored; 64
  frontend), `cargo fmt --check`, `git diff --check` pass. **Not verified on device:** a real provider
  feed refresh (`just dev`, Live tab → refresh the guide).

### Live guide on screen (M24, part 2) — 2026-09-29

Branch `feat/epg-surface`. The first part stored guides, but nothing showed them: the Now/Next chip
was dropped with the old browse view on 2026-06-01 (`e385640`) and never re-wired, a source refresh
never refreshed its guide, and a guide URL added in Settings stayed empty.

- **Reads:** `epg_channels_now_and_next { channelIds }` resolves each channel row's `tvg_id` and
  source and reads that source's guide feed. `list_epg_feeds` reports each feed's kind, last
  refresh, redacted last error and programme count (never the URL).
- **On screen** (`features/browse/guide.ts`, `GuideViews.tsx`): one TanStack query per channel,
  requests from the same moment coalesced into one call (≤ 100 channels), refetched when the
  programme ends, invalidated once per finished refresh job (`epg:refresh:done`). Live cards show
  "Now · title" with its elapsed share; the hover preview and the live player's title bar show now
  (time slot, progress) and next; brand-bundle channels and the player's channel strip show now.
  Cards that aren't a single channel (series, VOD, bundles) never show a guide.
- **Refresh:** every successful import (add or refresh, and a recovered one) queues the source's
  guide refresh in Rust; adding a guide URL queues it too; Settings → Sources shows the guide status
  with a "Refresh guide" button.
- **Tests (9 new):** storage (channels read their own source's guide in request order; feed status
  counts and never carries a URL; URL redaction); frontend (one batched request for many channels; a
  finished refresh refetches; a disabled id never shows another item's cached guide; a failed batch
  settles every waiter; refetch timing; Settings summary).
- **Independent review:** no Critical. **High, confirmed and fixed:** series and VOD cards (and
  bundles) showed a cached channel's guide under the same id. **Medium, fixed:** Settings could stay
  "Refreshing…" when a job ended without a per-feed terminal event; adding a guide URL gave no
  feedback or status reload. **Low, fixed:** a recovered import skipped its guide refresh; errors
  stored before URLs were stripped could show credentials; unbounded batch size; the cinema tiles
  relied on context for the profile; block markup inside buttons and no progress semantics. Low,
  recorded: four small queries per channel per batch; 11 px guide text follows the existing cards
  (off the documented scale). Mutation check: dropping the disabled-id guard fails its test.
- **Validation:** `just lint`, `just typecheck`, `just test` (583 Rust, 0 failed, 5 opt-in ignored; 70
  frontend), `cargo fmt --check`, `git diff --check` pass. **Not verified on device.**

### macOS lifecycle (M29) — 2026-09-29

Branch `feat/macos-lifecycle`. Before: every playback resource was released only by a frontend
call, so a webview reload left native video playing behind a fresh page (and background work
suspended until restart), quitting could leave ffmpeg remuxes and transcription running without a
final progress write, and nothing handled sleep/wake.

- **Page epochs** (`AppState::page_epoch`): each playback resource (native session, proxy stream,
  AirPlay) records the page it was opened for. When the webview starts loading a page the epoch
  moves on at once; `lifecycle::release_for_page` releases older pages' resources — the native
  session after its final progress write (bounded to 2 s), AirPlay, proxy sessions and remuxes,
  caption transcription — and their guards; the new page's stay. An open still in flight when the
  page changed undoes itself ("the page reloaded").
- **Quit** (`RunEvent::Exit`, main thread): `release_for_exit` writes progress and stops remuxes and
  transcription; mpv and AirPlay end with the process. The render context is freed inline when
  already on the main thread (never `dispatch_sync` onto the main queue from it).
- **Sleep/wake** (`NSWorkspace` notifications, queued synchronously onto the playback coordinator in
  order): sleep pauses the native session (the pause writes progress); wake, after 3 s for the
  network, reconnects a live channel that was playing via `playback:recover { reason: 'wake' }`
  with a fresh recovery budget, unless the Mac slept again; VOD and a paused channel stay paused.
- **Tests (3 new):** a reload releases the old page's playback and keeps the new page's; quitting
  releases everything; the coordinator reconnects only a playing live channel, once, not after a
  replacement or a newer sleep, with a fresh budget. Mutation checks: dropping the epoch filter,
  the sleep sequence or the budget reset each fails a test.
- **Independent review:** no Critical. **High, confirmed and fixed:** quitting with native video on
  screen would `dispatch_sync` onto the main queue from the main thread (crash or hang). **Medium,
  fixed:** a reload released whatever existed at that instant — an old page's open landing later
  kept playing, and a new page's quick open could be killed. **Low, fixed:** sleep and wake could be
  applied out of order (each was spawned); the wake reconnect fired before the network was up and
  spent the recovery budget; an unused recovery marked the user's next open as one. Low, recorded:
  an in-flight proxy request keeps its remux alive until it completes.
- **Validation:** `just lint`, `just typecheck`, `just test` (586 Rust, 0 failed, 5 opt-in ignored; 70
  frontend), `cargo fmt --check`, `git diff --check` pass. **Not verified on device:** reload during
  playback (`just dev`, Cmd+R), quit during playback, sleep/wake with a live channel.

### Security / privacy hardening (M25) — 2026-09-29

Branch `feat/security-hardening`. Before: the local proxy's `/proxy?url=` relayed any non-loopback URL
with `Access-Control-Allow-Origin: *` (any page in the user's browser could fetch through Nova), and
every segment/file locator — seen by hls.js, the webview and AVPlayer — carried the upstream URL
with the Xtream account's credentials; the webview ran with `csp: null`; ffprobe/ffmpeg stderr and
reqwest errors could carry credential-bearing URLs into logs and into `fallbackReason` / errors sent
to React.

- **Session-scoped proxy** (`crates/hls-proxy`): `/proxy?url=` removed. A manifest's URIs become
  `/stream/{token}/r/{id}` (opaque ids from a per-session keyed hash, least-recently-used table of
  20 000), direct files play from `/stream/{token}/file`; the proxy only fetches what a session's
  own manifests named. Playlists are recognised by URL, type or `#EXTM3U` start (BOM included),
  resolved against the post-redirect URL; local upstreams are refused by literal host and at DNS.
- **CSP** (`tauri.conf.json`): own scripts only (`'self'`, no inline in production, no eval), network
  to IPC and the local proxy, images from the artwork cache or http(s), no objects or frames; a dev
  policy adds Vite's server. A test pins the policy and an `index.html` free of inline script/style.
- **Secret-safe text**: `nova_core::redact_urls` (linear; recognises schemes glued to ANSI codes or
  values; user info up to the last `@`) reduces URLs to `scheme://host[:port]/…`. Applied to every
  log line (redacting tracing writer), the proxy's fallback reasons, playback/AirPlay/native errors
  sent over IPC, and stored EPG errors.
- **Tests (11 new or reworked):** proxy end to end against a fake provider (opaque URIs only, keys and
  segments resolve, no relay, unknown/foreign ids refused, file + Range, released sessions dead,
  loopback upstream refused, extensionless `text/plain` playlist behind a redirect with `EXT-X-MAP`,
  DNS-to-localhost refused); LRU keeps a live stream's playlist and key; redaction cases and a
  linear-time bound; logs with ANSI on and off; CSP invariants.
- **Independent review:** no Critical/High. **Medium, confirmed and fixed:** playlists without `.m3u8`
  or an HLS type were streamed raw (credentials to the webview, playback broken); first-in eviction
  would drop a long live channel's playlist and key; the log redactor missed URLs right after ANSI
  colour codes; quadratic redaction on hostile text; hosts resolving to 127.0.0.1 reached local
  services. **Low, fixed:** relative URIs resolved against the pre-redirect URL; `/` or `@` in a
  password, brackets in paths; inline-style CSP hazard pinned. A BOM-prefixed playlist bug was found
  by the new test and fixed. Mutation checks: the redirect base and playlist sniffing each fail a
  test when reverted. Low, recorded: CORS `*` on token routes; `Debug`-formatted URLs and panic
  messages bypass the redactor; scheme-less `host:port/path` text is not redacted.
- **Validation:** `just lint`, `just typecheck`, `just test` (596 Rust, 0 failed, 5 opt-in ignored; 70
  frontend), `cargo fmt --check`, `git diff --check`, production `vite build` (no inline script)
  pass. **Not verified on device:** playback of live HLS, VOD files and AirPlay through the new
  routes under the CSP (`just dev`, then a production build).

### Frontend redesign, UI-1: shell and Home (M22 / §49–51) — 2026-09-29

Branch `feat/ui-shell-home`. Plan: floating glass top bar, sections with their own design and one
identity, a clickable prototype approved first (Home "Tonight", Cinémathèque, a movie page, Shows,
the Live guide, the ⌘K palette). UI-1 builds the shell and Home; Movies, Series, Live, Search,
Library and Settings get their own designs in UI-2..UI-7.

- **Shell** (`src/app/`): `routes/Home.tsx` (857 lines) split into session (stable) + session status,
  playback (stable actions + a separate "playing" flag), overlays, library actions and catalog
  events. A floating top bar (sections, ⌘K pill, playlist switcher, profile menu; a deep drag
  region; transparent over art, glass once scrolled via opacity only). One navigation stack per
  section (`navigation/nav.ts`, pure reducer): pages push inside the shell instead of replacing
  it; Esc/⌘[ back, ⌘1–5 sections, re-selecting a section returns to its root; switching playlist
  returns every section to its root. View Transitions cross-fade where WebKit has them.
- **Keep-alive screens** (`app/Screen.tsx`): every visited screen stays mounted in its own scroll
  container, hidden (visibility + `content-visibility: hidden` + `inert`), so switching and back are
  instant with the same scroll, images and state. Hidden screens' queries unsubscribe (TanStack
  `subscribed`), so invalidations never refetch or re-render them until shown; focus follows the
  shown screen; hover previews close with their screen.
- **Home "Tonight"** (`src/sections/home/`, no backend change): resume hero (TMDB text-free backdrop
  and logo, progress, "Also in progress"), or a spotlight on a new film; On air now (recent and
  favourite channels with programme and time left); Up next episodes; New in your library (a
  feature and a list); Trending this week that you own (ranked); Because you watched X (TMDB
  recommendations in the library, with a why chip). Every section has the add-a-playlist state.
- **Performance / cache:** only the top bar blurs while scrolling; off-screen Home modules skip
  layout and paint (`content-visibility: auto`); fixed image boxes, lazy decoding, hero fade-in;
  memoized screen bodies and stable contexts so navigation never re-renders hidden screens; the
  cache is warmed at start and on idle, and resting on a card prefetches its page (shared query
  definitions: `titleArtQuery`, `titleFactsQuery`, `canonicalMovieQuery`).
- **Tests:** shell (section counts, search, favourites on the page, no-playlist state, stacks kept
  across switches, Esc back, re-select to root, a later-visited section never remounts another
  screen — scroll kept, a failed favourite rolled back with the reason); nav reducer (stacks,
  re-select, depth bound, patch, reset); Home feed; hover prefetch fills exactly what pages read.
- **Independent review:** no Critical. **High, confirmed and fixed:** screens of a section were
  remounted (scroll and state lost) when an earlier section was first visited (unkeyed nested
  lists). **Medium, fixed:** pages and filters survived a playlist switch; enrichment refetched every
  kept screen; scroll saved after hiding; most of the bar didn't drag the window; the grid assumed
  the old padding; session status re-rendered every screen. **Low, fixed:** previews over the new
  section; focus stranded on hidden screens; discovery lists didn't flip favourites; an episode
  listed twice on Home; missing empty states; weights/strokes/second blur. Low, recorded: no
  mouse back-button qualification; a red LIVE dot kept as a broadcast status colour. Mutation
  check: reverting the flat screen list fails its test.
- **Validation:** `just lint`, `just typecheck`, `just test` (596 Rust, 0 failed, 5 opt-in ignored; 86
  frontend), `cargo fmt --check`, `git diff --check`, production build pass. **Not verified on
  device** (`just dev`): scroll smoothness, the glass bar, View Transitions, the WebKit behaviour of
  hidden screens (scroll kept under `content-visibility: hidden`).

### Frontend redesign, UI-2: Movies "Cinémathèque" and the film page — 2026-09-29

Branch `feat/ui-movies`.

- **Backend (movies DTOs):** summaries carry card facts flattened in (genres, runtime, rating with
  its source — TMDB over the provider, zero treated as unrated — quality/audio badges from stored
  hints, version count in the playlist). `ContentSort` gains title, year and rating; filters gain
  genre, a year range and a TMDB collection. New commands `list_movie_genres`,
  `list_movie_decades`, `list_library_collections` (≥ 2 films owned). The detail gains original
  title, release date, votes and keywords; each variant carries badges, "failed recently" and
  whether it is Nova's pick (the same ranking playback uses, with the profile's audio language).
  The film list sorts and cuts a materialized page before computing per-film source hints.
  Migration 0042 indexes genre and year per playlist.
- **Movies section** (`src/sections/movies/`): a letterboxed spotlight (watchlist first), a
  collections shelf (fanned poster stacks), decade tiles, a genre cloud sized by share, "Just
  arrived", then every film on the windowed wall. Choosing a collection, decade or genre (or
  searching) narrows the wall with chips from the facet model; sort by recent, A–Z, year, rating.
  Plain search shows one relevance-ranked set, without a list count or sort.
- **Film page** (`src/pages/MoviePage.tsx`): full-bleed backdrop and logo, meta with rating source
  and votes, audio language choice (re-ranks Nova's pick once saved), a Versions panel that
  explains each source's place from its evidence, cast and facts, keywords, the collection strip
  and more like this. Choosing a source plays exactly it; a source that is gone returns NotFound
  and the page refreshes. Translate is a disabled "Soon" placeholder.
- **Tests:** storage (card facts from provider facts written by the real serializer, filters and
  sorts, facet counts, the page's badges and pick); version ordering and reasons (MULTI, all
  failed); immediate settings save; shell test narrowing by decade and playing a chosen source.
- **Independent review:** no Critical. **High, confirmed and fixed:** runtime and release date read
  snake_case keys that import never writes (fixtures had hand-written JSON). **Medium, fixed:** the
  audio change refetched before the debounced save; per-film hint subqueries ran for every
  matching film before the sort (≈4× slower at 100k); the decade poster scan; the plain-search
  count disagreed with the results; zero ratings sorted as scores; misleading version reasons;
  a stale source silently played another. **Low, fixed:** reasons hidden from assistive tech,
  an off-scale type size, duplicate keys, the count refetched on sort. Low, recorded: cards count
  a film's versions in the active playlist while the page lists every playlist's sources.
- **Validation:** `just lint`, `just typecheck`, `just test` (601 Rust, 0 failed, 5 opt-in ignored; 92
  frontend), `cargo fmt --check`, `git diff --check`, production build pass. **Not verified on
  device** (`just dev`): the page on a real library, wall scrolling with facts at 100k.

### Frontend redesign, UI-3: Series "Shows" and the series page — 2026-09-29

Branch `feat/ui-series`.

- **Backend:** `SeriesSummary` carries flattened `SeriesFacts` from stored evidence — genres (TMDB,
  else Nova's), rating with its source (zero is unrated), TMDB status verbatim, first/last year
  (TMDB, else the provider's release year), the next scheduled episode — and the profile's
  progress: finished episodes and last watched, from `episode_watch_state` by durable episode
  key. One page-first query (`repositories/series/browse.rs`) serves `list_series`,
  `count_series`, title lookups (semantic hits), the genre facet and the schedule; filters gain
  genre and status (airing / ended), sorts gain title, year and rating. New commands
  `list_series_genres`, `list_series_schedule` (dates validated, ≤ 60) and `list_series_cards`
  (≤ 50 titles). A desktop test pins the TMDB facts JSON keys the SQL reads.
- **Shows section** (`src/sections/series/`): Continue your shows (text-free backdrop and logo,
  resume or next episode, one tick per episode, a play button), This week (a seven-day
  calendar of scheduled episodes, facts of followed shows warmed a few at a time, once per
  session), then every show on a windowed wall of wide cards (`ShowCard`, TMDB images sized to the
  slot) with their status. Search, genre and status narrow the wall as chips.
- **Series page:** progress and status under the meta line, season tabs with TMDB season posters
  (arrow/Home/End keys); fresh card facts over the navigation seed's flags.
- **Tests:** storage (facts and progress, zero rating, provider year, genre/status/sort with
  count agreement, schedule window, progress surviving a URL change, category-scoped cards);
  status/meta/ticks/week helpers; show cards patched and rolled back on a toggle; shell test for
  the section and opening a show.
- **Independent review:** no Critical. **High, confirmed and fixed:** progress read the row-keyed
  history mirror, which loses rows when a provider changes an episode URL. **Medium, fixed:**
  category-filtered cards described the whole show; show cards kept stale flags after a
  toggle; warming could skip refreshing the schedule; status filtering only covers shows with
  fetched facts (now said so, and years fall back to the provider). **Low, fixed:** stale
  scheduled dates read as airing, "In production" shown as returning, overclaiming week copy, an
  early click on a Continue card played instead of opening, the week didn't roll over, the
  wall's progress stayed stale after playback, season tab keys. Low, recorded: a whole-library
  facts backfill (status for every show) is not done; the A–Z list is ~30 ms at 5k shows (was
  ~1 ms: the page is now sorted after grouping); some spacing values follow the prototype rather
  than the 4 px grid; no "new episode" badge (episode `added_at` resets on refresh).
- **Validation:** `just lint`, `just typecheck`, `just test` (608 Rust, 0 failed, 5 opt-in ignored; 99
  frontend), `cargo fmt --check`, `git diff --check`, production build pass. **Not verified on
  device** (`just dev`).

### Frontend redesign, UI-4: Live "Channels" and the guide (EPG repass) — 2026-09-29

Branch `feat/ui-live`.

- **Backend:** `epg_guide` reads a twelve-hour window for channel rows in one query (each channel's
  own source's feed first, then any feed, then pre-feed programmes); `epg_channels_now_and_next`
  is one query per call (was four per channel), ranking the programme on air (latest start) and
  the next one separately; guide requests are bounded (200 channels, 13 hours; `epg_range` too).
  `list_live_overview` (channels, channels with a programme on now or later, category counts).
  `epg_refresh_if_stale` queues a refresh when a feed is 12 hours old or never refreshed and no
  refresh is queued or running. Gzip XMLTV feeds are inflated while parsing (≤ 2 GiB); a stream
  error (truncated/corrupt gzip, the cap) or 1,000 malformed programmes in a row fails the feed
  and keeps its guide.
- **Channels section** (`src/sections/live/`, `src/features/live/`): header with coverage and the
  time, a rail (all, favourites, categories with counts, regions, countries), a virtualized guide
  (pinned channel column and time header, amber now-line, rows read in blocks of 30 as they
  scroll, arrow/Home/End/Enter), the chosen channel's now/time left/progress/description/later
  with Watch and favourite, and a toggle to the logo wall. Search narrows the guide in place.
  Opening Live re-checks guide freshness every 12 hours.
- **Bundles** (`pages/ChannelBundlePage.tsx`) open on the same guide. The old shared browse
  section and genre rails are retired (every section has its own screen); guide text is back on
  the type scale.
- **Tests:** storage (guide window by feed preference, overlapping now/next, repeated ids,
  coverage now/later, category counts); gzip ingest (whole, truncated, trailer-less, trailing
  bytes — none hang, the guide is kept), the inflate cap, request bounds, freshness rule;
  timeline maths; the grid's rows, no-guide state and keys; programme blocks across window moves
  and failures; shell test searching the guide and switching to the wall.
- **Independent review:** **Critical, confirmed and fixed:** a gzip feed with a cut trailer or
  trailing bytes spun the parser forever and blocked the EPG queue. **High, fixed:** a truncated,
  corrupt or capped gzip published a partial guide as a success. **Medium, fixed:** guide
  queries ignored the hidden-screen rule and blanked rows every half hour; keyboard scrolling
  and the now-line were offset by the header; no tab stop once the chosen row scrolled away;
  search remounted the guide; a duplicate full refresh after an import; region/country filters
  lost with the old section; wrong now/next with overlapping programmes. **Low, fixed:** the
  now-line over the channel column, endless "Loading the guide" on errors, coverage counting
  ended guides, repeated ids, freshness checked once per session. Low, recorded: the red LIVE
  dot is a broadcast status colour, as on Home; surfaces use literal hex like the other sections;
  Live lost its "Recently watched"/"Trending" rails (Home keeps live Continue watching and On air).
- **EPG repass, still open:** channels match guides by `tvg_id` only (no canonical channel
  matcher); playlist `auto_refresh_hours` is still unused (guide freshness now works).
- **Validation:** `just lint`, `just typecheck`, `just test` (615 Rust, 0 failed, 5 opt-in ignored; 104
  frontend), `cargo fmt --check`, `git diff --check`, production build pass. **Not verified on
  device** (`just dev`).

### Frontend redesign, UI-5: Search — 2026-09-30

Branch `feat/ui-search`. Main implementation: `bfde45f`; review fixes completed in the
follow-up. UI-5 is complete within the scope and qualification limits recorded below.

- **Search page:** opened from the command palette's "Search everything" action, with
  separately queried film, show, channel and dialogue Moments groups. A deterministic
  interpreter recognizes library genres, decades, contextual years and media types as
  dismissible chips. Plain title queries use the existing lexical/semantic commands;
  faceted queries use filtered lists. Missing semantic models have a capability notice.
- **Moments:** FTS5 across the playlist's transcribed films and episodes, including cached
  translations, with title and timestamp; choosing a moment passes `PlayMedia.startSec` to
  playback. Live transcripts are excluded. This goes beyond the previously reserved group;
  cross-library dialogue search is lexical, while per-title transcript search stays hybrid.
- **Existing review fixes retained:** scope dialogue candidates before the ranking cap so
  another playlist cannot crowd them out; collapse repeated moments from different
  transcripts; reject dialogue queries shorter than two characters; reattach orphaned
  transcripts by stable stream URL content key during playlist publication.
- **Continuation fixes:** collapse repeated moments before the result limit, avoiding
  underfilled pages; align the frontend's minimum dialogue query length with the command;
  show a retry action for failed search groups instead of claiming no results. Regenerate
  SQLx metadata against a temporary schema built from every app/profile migration (the
  local development schema is stale).
- **Independent review:** one high and two medium findings confirmed and fixed. Explicit
  moment starts now belong to the native playback coordinator, overriding saved resume and
  using its readiness/retry policy with a one-second lead-in; recovery resumes current
  progress. The web fallback skips saved resume for explicit starts. Type-only searches
  (`movies`, `shows`, `channels`) now query bounded lists. Extra genre/date terms remain
  searchable text instead of displaying ignored chips. A fresh re-review found no further
  actionable code defects. The IPC documentation was updated as requested by the reviewer.
- **States:** an empty library offers Add a playlist; unsupported date filters for explicit
  show/channel queries explain the limitation; failed Moments do not claim no matching lines.
- **Validation:** `just lint`, `just typecheck`, `just test` (618 Rust, 0 failed, 5 opt-in
  ignored; 116 frontend), `cargo fmt --all -- --check`, `git diff --check` and the frontend
  production build pass (existing bundle-size warning). Rust fixture tests require localhost
  access. SQLx offline metadata is current. The native debug `.app` build passes.
- **Native smoke:** a separate debug bundle and isolated profile, with generated 90-second
  H.264/AAC media, one film/show/channel and a seeded transcript. Verified keyboard entry
  through the palette, the empty-library CTA, type-only Shows and Channels, genre/type/decade
  film results, the Moments layout, and query/result preservation after closing playback.
  A 42-second moment opened at **0:41**, overriding a later saved resume; playback advanced
  normally and could be paused. This fixture validates playback positioning, not transcription
  accuracy or a real provider. No existing user profiles were modified.
- **Limits:** series year filtering is not implemented; groups are bounded previews without
  pagination. Transcript reattachment requires an unchanged stream URL and is covered by
  storage regression tests; real-provider refresh qualification remains outstanding. Native
  smoke is not full device/performance certification; the web fallback was not exercised on
  device. M26's real-profile upgrade qualification remains separate.

### Frontend redesign, UI-6: Library and history — 2026-10-01

- **Contract:** `docs/implementation/ui-6-library.md`. Library now has keyboard-accessible
  Watchlist, Favourites and History views scoped to the selected playlist. Saved views reuse
  the existing bounded, virtualized grids, card actions and detail navigation; films/shows
  have A–Z and playlist-added ordering, with search across every media type. Channels retain
  their existing family grouping. Empty states offer a relevant browse or clear-search action.
- **History:** a read-only `list_library_history` command reads durable canonical film and
  episode progress plus current live-channel history. It includes completed titles and
  unavailable films/episodes, groups by local date, and pages 50 rows with one-row lookahead.
  Each row is the latest watch of that film, episode or channel, not a session event log.
  Resume, Watch again and Watch live use the existing player; completed playback starts at
  zero. Missing media has no playback action. No migration or new AI/provider work.
- **Lifecycle:** source changes reset controls; hidden screens unsubscribe from queries.
  Saved-item toggles invalidate membership grids/counts. Catalog refresh invalidates history;
  player teardown refreshes it again after the final asynchronous progress write, fixing
  a race found in independent review. Unsupported channel sort controls were also removed.
- **IPC types:** the new DTO is generated from Rust through a test-only `ts-rs` dependency;
  `just bindings` regenerates and formats it. Existing handwritten DTOs are unchanged.
- **Automated validation:** `just lint`, `just typecheck`, `just test` pass: **621 Rust tests,
  5 environment-dependent tests ignored; 123 frontend tests**. Formatting and diff checks
  pass. New regressions cover scope, deterministic paging, completed/unavailable media,
  episode reattachment, filters/source reset, empty/retry states, playback intents, saved
  membership invalidation and delayed final progress writes. Independent review verified fixes.
- **Native validation:** debug Tauri app build passed. An isolated synthetic profile on
  macOS verified Watchlist/Favourites separation, removal through the film page followed
  by the empty Watchlist, date-grouped history, completed and unavailable states, episode
  filtering, and filter preservation after playback. A completed film replayed at **0:00**;
  a saved 25-second episode resumed at **0:24** (the existing one-second start lead-in).
  Closing playback immediately refreshed progress and changed the film action to Resume.
  Artwork fallback and layouts were visually inspected. No user profiles were modified.
- **Limits:** unavailable episodes lack retained episode title/numbering-kind metadata.
  Saved coordinates may be synthetic, so they show the series name and “Episode” with
  date/progress rather than invented numbering. Live history requires a current channel row.
  No clear-history control or playback-event diary. Real-provider refresh qualification and
  M26's real-profile upgrade qualification remain separate. Native smoke is not full
  device/performance certification; the web fallback was not exercised on device.

### Frontend redesign, UI-7: Settings, models and storage — 2026-10-01

- **Contract:** `docs/implementation/ui-7-settings-models.md`. Settings now uses an
  accessible dialog with seven keyboard-navigable sections: Playback & captions,
  Appearance, Models, Storage, Catalog, Playlists & guide, and Profile & about.
  Portal selects remain usable, Escape closes the appropriate layer, and Command-K
  does not stack another overlay. Intentional Add-playlist/profile handoffs preserve
  destination focus; closing Settings restores the entry point. The profile menu
  finishes closing before opening Settings so it cannot steal dialog focus.
- **Models:** one view for Whisper and embedding installation, with loading/error/retry
  states, per-artifact Whisper progress, installed/repair states, runtime workload and
  memory estimates, and external-folder status. Pending mutations survive section/dialog
  remounts and prevent duplicate model actions. Completion refreshes Search capability.
  External folders remain read-only registrations; current jobs use internal models.
- **Storage and catalog:** separate profile transcript and shared artwork caches with
  explicit scoped confirmations. Counts refresh when reopened; clearing transcripts
  invalidates that profile's Moments results. Catalog retains opt-in metadata, write-only
  credentials and explicit enrichment, using the existing serialized preference writer.
  Model installation links to Models. Playlist and profile actions report safe errors;
  failed playlist loading no longer masquerades as an empty library.
- **IPC and scope:** eight existing DTOs are generated from Rust with test-only `ts-rs`;
  `just bindings` includes desktop exports. No new command, migration, model backend,
  automatic download, scheduler or external-model execution. Transcript byte totals are
  app-wide in the existing command, so the profile cache displays its transcript count only.
- **Automated validation:** `just lint`, `just typecheck`, and `just test` pass: **629 Rust
  tests, 5 environment-dependent tests ignored; 135 frontend tests** after the final
  frontend rerun. Regressions cover dialog/menu focus, portal selects, section navigation,
  remounting pending downloads, progress, model errors, capability invalidation, scoped
  cache clearing, fresh counts, playlist failures and credential/persistence behavior.
  Formatting/diff checks and the native debug app build pass. An initial concurrent
  build/doctest artifact collision was resolved by a sequential full-suite rerun.
- **Independent review:** fixed Search capability invalidation, Add-playlist focus handoff,
  and stale cache/Moments data; re-review found no further actionable issues. Native
  inspection also prompted a responsive toolbar and scrollable Settings sidebar at the
  minimum 1024×600 window size, plus the profile-menu focus fix; these were reviewed again.
- **Native validation:** synthetic profile databases in a separate debug bundle; inspected
  model/runtime and external-folder states, cache scopes, playlists, catalog, profile and
  playback settings. Verified keyboard section navigation, select-popup Escape, palette
  suppression, toolbar access and sidebar scrolling at the minimum window size. The final
  build verified profile-menu entry focuses Settings, Tab stays inside, and Escape returns
  focus to the profile button. Shared
  installed-model/artwork locations were read only. No models were downloaded/removed,
  caches cleared, external folders authorized, or existing user profiles modified.
- **Limits:** actual download/removal/failure/progress and cache mutations are covered by
  mocked frontend regressions, not exercised against live model services or user caches.
  No live provider/TMDB enrichment qualification or performance certification. Existing
  backend model capabilities and preference persistence semantics remain unchanged.

### Next dependency

**UI-1–UI-7 are implemented.** Earlier phase entries retain their stated device validation
limits. M26's real-profile upgrade qualification still needs a real profile copy; it is
separate from the completed frontend redesign. No UI-8 milestone is defined by this work.

---

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

At the time of this audit the target was `master-plan.md`, read alongside `ARCHITECTURE.md`,
`docs/nova-production-architecture-blueprint.md`,
`docs/nova-100k-media-intelligence-architecture.md` (all now in `docs/archive/`), `docs/catalog.md`, and the
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

## P11 — TMDB matching: trusted IDs, provenance, progressive

Branch `feat/p11-tmdb-evolution` off `main`.

### Gaps closed

- **Weak matches published:** a 0.3–0.6 score attached a poster. Now anything
  below 0.6, or ambiguous (two different titles within 0.05), stays unresolved
  with no metadata.
- **No use of trusted IDs:** provider TMDB IDs (P09 aliases) are now looked up
  directly via `/3/movie/{id}`, and accepted only when year or title corroborates.
- **No provenance:** each decision now stores method, confidence, matcher
  version, request keys, top candidates and outcome.
- **Nova ID vs TMDB ID:** matches live on the entity in
  `entity_external_matches`. Search results never become identity aliases, and
  two entities sharing a TMDB ID stay distinct.
- **Re-matched on every refresh:** movie TMDB data lived on provider rows and was
  wiped by every refresh. Matches are now entity-owned and mirrored back after
  import with no API calls.
- **TV IDs as movie matches:** `/search/multi` TV results could be recorded as
  movies. Results are now filtered by media kind.
- **Credential leak risk:** reqwest error text included the URL with `api_key`.
  Errors now drop the URL, and the Read Access Token (Bearer) is supported.
- **Progressive:** priority order (personal → in progress → recent), 25-title
  batches, Home refresh while matching, and retry of unresolved titles after 7
  days or on a matcher bump.

### Tests (22 new or reworked)

- Enrich (10): clear match; low confidence and the former weak band stay
  unresolved; remakes are ambiguous without a year and matched with one;
  duplicate IDs across searches; determinism; original titles; trusted-ID
  corroboration rules; credential detection and redaction; secret-free evidence;
  media-kind filter.
- Desktop matcher with an offline fixture fetcher (6): a corroborated trusted ID
  makes no search call; a contradicted ID and a 404 fall back to search (the
  cascade stops early); conflicting provider IDs are unused; low
  confidence/ambiguous stay unresolved with evidence; a TV-only result never
  matches a movie; series follow the same rules.
- Storage (5): priority ordering; queue exit, retry age and version bump;
  mirror, refresh survival without re-queueing, and clearing on unresolved; a
  shared TMDB ID never merges or aliases entities; schema CHECKs.
- Mutation check: accepting unverified trusted IDs fails a test.

`just lint`, `just typecheck`, `just test` (424 Rust, 0 failed, 5 ignored; 28
frontend): pass. SQLx metadata regenerated; the removed per-row movie TMDB
queries were pruned.

### Remaining known gaps

- Not run against the live TMDB API in this session (no credential here). The
  behaviour is covered by offline fixtures only.
- Movies only have trusted IDs. Series have no entity, and Xtream `get_series`
  `tmdb` is not parsed yet.
- Existing libraries lose weak-match posters as titles are re-decided under
  matcher v2. That is deliberate and trades coverage for correctness.
- Search matching is still lexical (title Jaccard + year). No
  embedding/original-language rerank.
- A merged entity keeps its own match record. The survivor is matched
  independently.
- There is no UI yet to inspect evidence or manually pick a candidate.

## Catch-up — naming v1, episode resume, P12 search, artwork v1, live tiles (2026-09-27/28)

Merged to `main`; logged here after the fact (details in the phase history and commit
messages):

- **Naming v1:** one versioned provider-title parser, raw evidence kept, provider
  collections separated from Nova genres, localized TMDB titles with the
  original kept, one display-name policy (`nova_core::naming`).
- **Episode resume across refresh:** episode history is parked by a stable
  binding key and reattached after a re-import.
- **P12 canonical movie search:** `movie_search` FTS5 projection with tiered
  ranking (exact title first), people/keywords/genres indexed.
- **Artwork v1:** shared image-URL policy, XMLTV channel icons, opt-in iptv-org
  logo directory, validated `nova-art://` disk cache, TMDB title logos and
  text-free backdrops on demand.
- **Live brand tiles:** logos presented on their own colour (`ChannelTile`).

## Planning — series parity, per-section Continue Watching, rich metadata (2026-09-28)

On-device review findings:

- Series take a thinner path than movies: small header, no hero or title
  logo, no Play/Resume/Next-episode action, no per-episode progress, provider
  episode data only, no React Query caching.
- Continue Watching is one rail shared by all tabs (movies + episodes, no
  source filter, no live), with raw provider titles and one card per episode.
- Only five TMDB endpoints are used; `/tv/{id}`, seasons, episodes, runtime,
  rating, certification, trailers and similar titles are unused, and people /
  keywords are stored but never shown. Xtream series/episode plots, cast,
  ratings, durations and `get_vod_info` are dropped.

Decisions: master plan §9 gains M4.6 (TMDB GET capability map with T1/T2/T3
tiers + provider facts) and §14 gains five ordered slices (per-section
Continue Watching → provider metadata → rich TMDB facts → unified title detail
→ catalog-aware discovery). Trailers open in the browser. Canonical series
identity, user corrections, people pages and in-app trailers are deferred.

## Series parity, per-section Continue Watching, rich metadata — implemented (2026-09-28)

Slices from the planning entry above, one commit each (stacked branches
`feat/continue-watching-sections` → `feat/provider-metadata` →
`feat/tmdb-rich-facts` → `feat/title-detail` → `feat/library-discovery`):

1. **Per-section Continue Watching:** `list_continue_watching` takes a
   section and a source; series show one card per show (resume or Up next,
   finished shows drop out); live lists recently watched channels.
2. **Provider facts:** Xtream plots, cast, directors, genres, dates, ratings,
   runtimes, trailers and backdrops are normalized (`ProviderFacts`) and kept
   (migration 0034); series facts survive the catalog row replacement; the
   provider's series TMDB id is stored.
3. **Rich TMDB facts:** `get_title_facts` / `get_season_episodes`, stored per
   TMDB work (0035). *Adjustment to the plan:* facts are fetched **on demand
   when a page opens** (one cached call per title viewed) instead of in the
   bulk matching pass, per M4.3's "do not eagerly download every field";
   TTLs refresh ratings and new episodes. Series match through the provider's
   TMDB id when TMDB corroborates it.
4. **Unified detail page:** movies and series share `TitleHero` (meta line,
   cast, trailer opened in the browser by a YouTube-only backend command);
   series get Resume/Next/Play, a season picker and rich episode rows with
   progress.
5. **Discovery:** "More like this", collections and "Trending in your
   library", intersected with the library (never unavailable titles).

Gaps: not exercised against the live TMDB API or a real Xtream panel in this
session (offline fixtures only); series remain per-source (no cross-source
identity); no manual match correction; hover previews do not show facts yet.
