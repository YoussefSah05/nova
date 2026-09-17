# Nova Production Architecture & Implementation Blueprint

**Status:** proposed engineering blueprint, not an implementation-complete claim. **Baseline:** commit `c2841bc`; repository and external research inspected 17 September 2026. This document expands and supersedes the proposed Player 2.0 plan for future planning; existing accepted architecture remains in force until the corresponding ADRs and migrations are adopted.

**Reading guide:** §§6–23 define the canonical catalog; §§24–51 define playback and experience; §§52–75 define operational engineering and evidence; §76 defines migration gates. Code and SQL below are contract sketches, not migrations to run against user databases. Numerical budgets are proposed acceptance targets, not measured Nova results.

**Evidence limits:** lockfiles and source were inspected; a fresh app build, real-provider session, native device test and model benchmark were not executed for this report. Sources on moving upstream branches establish research leads, not compatibility with an unidentified local binary. Each implementation PR must pin the exact upstream revision it relies on. No recommendation here requires updating to the newest release.

**Document validation:** all 78 numbered sections and all ten required fields in each migration phase were checked. The SQL kernel executed in an isolated in-memory SQLite 3.53.4 database and passed ten checks covering generation-preserving reassignment, invalid memberships/bindings/facts, resume bounds, foreign keys and head compare-and-swap. This validates the sketch, not a production migration or Nova's linked SQLite build. No production database was opened or changed.

## 1. Executive architecture

Keep Nova's working Tauri 2, Rust, React/TypeScript, per-profile SQLite and libmpv stack. Add two strong domain boundaries: a **Catalog Compiler** that transforms preserved provider evidence into Nova-owned entities, and a **Playback Coordinator** that selects and resolves stream variants and controls one foreground playback session.

The canonical catalog answers “what is this?”; editions and timeline identities answer “which cut and time coordinate system?”; variants answer “which playable source?” These distinctions prevent false duplicate merges, broken resume after version changes and caption caches attached to the wrong audio.

Providers remain replaceable adapters. Local models supply evidence through stable interfaces, never authoritative identities or arbitrary schemas. Search, rails, embeddings and display projections are rebuildable. Entity IDs, lineage, user corrections and user state are durable.

Use short-lived snapshot/generation staging in each profile database, atomically publish a validated generation, then update generation-aware projections. Keep the previous catalog visible during failures. Preserve existing playback while introducing typed state, cancellation, bounded recovery, output capabilities and diagnostics.

The first release objective is a reproducible, validated macOS Apple Silicon alpha. Timeshift, DVR, multiview, additional platforms and a general-purpose catalog LLM are extensions, not prerequisites. Packaging feasibility and licensing start immediately, not after the last feature phase.

## 2. Architecture principles

- Provider data is evidence, not Nova's domain schema; preserve it without exposing credentials.
- Rust owns catalog and playback truth; React owns presentation and interaction state.
- A channel brand family is a collection, not one channel; a movie work is not necessarily one interchangeable timeline.
- Stable random Nova IDs survive corrections; title hashes and provider IDs are lookup evidence, not canonical primary keys.
- Prefer an unresolved duplicate over a destructive false merge. Every merge must be explainable and reversible.
- Store claims, observations and user corrections separately; apply deterministic precedence by field.
- Import and basic playback work without models, TMDB or successful background enrichment.
- One bounded owner per write/control boundary; cancellation, generation guards and backpressure are mandatory.
- Publish whole catalog generations; never expose partly replaced source data.
- Keep secrets behind references; keep the app local-first and network activity explicit.
- Pin artifacts and behavior versions, not only package names. “Installed” means verified, not merely present.
- Cap resource consumption and retain diagnostic evidence. Unknown capabilities/metrics remain unknown.
- Migrate incrementally, preserving user state and a rollback path. Avoid rewriting working subsystems for conceptual neatness.

## 3. Current state assessment

Existing M3U/Xtream parsers, profile repositories, browse components, libmpv wrapper, native surface, caption stitching, translation and search provide a substantial foundation. Import already uses a transaction so refresh does not expose partially replaced rows; the target improves write-lock duration and replayability rather than inventing atomicity from scratch.

| Observed area | Finding | Required evolution |
|---|---|---|
| Catalog | `channels`, `vod_items`, title-keyed series/episodes and stream URLs are closely coupled | Stable entities, editions, variants, source bindings and revisions |
| Import | `push_entry` cleans live/series names; missing episode numbers default to 1 | Preserve original records; represent unknown numbering explicitly |
| Browse | `queries.ts` contains page-level duplicate/version grouping and raw row payloads | Server-owned canonical grouping/counts; generated clean DTOs |
| Identity | Provider/row IDs and normalized titles drive much behavior | Conservative resolver plus durable identity ledger and lineage |
| Enrichment | Embeddings and TMDB update existing rows | Provenance facts, idempotent jobs and controlled re-publication |
| Search cache | In-memory key uses source/kind and row-count invalidation | Include profile, generation, model and index revision |
| Player | Native `vo=libmpv`, OpenGL render context, `hwdec=auto-safe` | Keep renderer; centralize coordinator/policies and observed capability |
| PiP | Explicitly disabled when mpv is active; HTML-video fallback implements PiP | Native adapter feasibility; never claim current mpv PiP support |
| AirPlay | Separate AVFoundation implementation exists | Move output handoff truth into Rust; certify actual device/format behavior |
| AI | Whisper CLI; NLLB through CT2 CPU; E5 through ONNX | Pin artifacts/tokenizers and benchmark; no assumed Metal translator |
| Storage | WAL, four-connection pool, `ignore_missing` migrations | Writer discipline and explicit schema compatibility; safe refusal on downgrade |
| Environment | Stable/latest/major-only tool pins; system mpv discovery; mutable model URLs | Verified native SDK/model manifests and strict drift checks |
| Security/tests | CSP disabled; few mocked frontend tests; no full-app suite found | Explicit content boundary and real macOS tests |

Resolved packages in the inspected locks: Tauri Rust **2.11.2**, Tauri JS **2.11.0**, wry **0.55.1**, tao **0.35.2**, React **19.2.6**, TypeScript **5.9.3**, Vite **6.4.2**, SQLx **0.8.6**, libsqlite3-sys **0.30.1**, Tokio **1.52.3**, ct2rs **0.9.19**, fastembed **4.9.1**, ort/ort-sys **2.0.0-rc.9**, pnpm declaration **9.15.4**. These are resolved/declaration facts, not proof of installed runtime versions or mutual validation.

The loaded mpv/FFmpeg/SQLite versions, exact model revisions and Xcode/SDK still require runtime inventory. Retain the historical large-provider import result as a comparison point, not a guarantee for the new compiler.

Primary repository evidence: [import commands](/Users/youssefsah/Desktop/PProjects/nova/apps/desktop/src-tauri/src/commands/mod.rs), [browse queries](/Users/youssefsah/Desktop/PProjects/nova/apps/desktop/src/features/browse/queries.ts), [player UI](/Users/youssefsah/Desktop/PProjects/nova/apps/desktop/src/features/player/PlayerModal.tsx), [mpv integration](/Users/youssefsah/Desktop/PProjects/nova/crates/mpv-player/src/lib.rs), [native layer](/Users/youssefsah/Desktop/PProjects/nova/apps/desktop/src-tauri/src/mpv_layer.rs), [Whisper runtime](/Users/youssefsah/Desktop/PProjects/nova/crates/transcribe/src/whisper.rs), [NLLB runtime](/Users/youssefsah/Desktop/PProjects/nova/crates/translate/src/nllb.rs), [application state](/Users/youssefsah/Desktop/PProjects/nova/apps/desktop/src-tauri/src/state.rs) and [Tauri configuration](/Users/youssefsah/Desktop/PProjects/nova/apps/desktop/src-tauri/tauri.conf.json). These links describe the inspected checkout, not immutable upstream permalinks.

## 4. Complete system diagram

```text
SOURCE WORLD
 M3U / Xtream / XMLTV / source credentials / raw names and groups
       |             credentials -> macOS Keychain / protected vault
       v
 SourceAdapter -> ImportSession -> immutable SourceSnapshot + SourceRecord
       |
NOVA DOMAIN WORLD
       v
 CatalogCompiler
 validate -> extract facts -> classify -> resolve identity -> group variants
 -> build entities/editions -> taxonomy -> EPG candidates -> validate
       |
       v
 staged CatalogGeneration --atomic publish--> canonical catalog
       |                                      |
       |                    user state / lineage / user corrections
       +-> FTS / semantic index / rails / background metadata facts
                                              |
EXPERIENCE WORLD                               v
 React Browse / Details / Search -----> PlaybackRequest(entity, preferences)
                                              |
 Tauri typed boundary                         v
 Rust PlaybackCoordinator -> VariantSelector -> StreamResolver
       |                       |                    |
 Live / Movie / Episode policy |              resolved transport lease
       |                       +---- fallbacks -----+
       v
 PlaybackSession -> libmpv -> FFmpeg demux/decode -> VideoToolbox or software
       |                                                |
       |                         mpv render API -> macOS GPU surface
       |                         current: OpenGL; new backend: gated spike
       +-> CaptionTimeline -> React overlay / native subtitle adapter
       |        ^
       |        +-- embedded/external / Whisper / translation
       +-> QoE / diagnostics / watch progress
       +-> Native bridge: window, fullscreen, Now Playing, outputs, sleep

OPTIONAL ADVANCED LIVE
 provider -> CaptureSession -> bounded SegmentStore -> local manifest -> mpv
                                  |        |
                                  |        +-> PCM -> live ASR -> captions
                                  +-> RecordingService -> recording library
```

Application services stay in-process initially. FFmpeg and whisper-cli are managed child processes; CT2/ONNX work runs away from the async/UI threads. Native rendering is isolated by thread/lifetime, not by a premature service deployment. WebKit retains its own OS process model.

## 5. Monorepo structure

```text
apps/desktop/src/
  features/{browse,player,settings,profile}/   retained presentation modules
  types/                                     generated Rust IPC contracts
apps/desktop/src-tauri/src/
  commands/                                  validation + delegation only
  services/                                  composition root
  platform/macos/                            native surface/output bridge
crates/
  core/                                      IDs, shared small value types
  catalog/                                   compiler/domain/policies [new]
    src/{source,facts,classify,naming,identity,variants,taxonomy,lineage}/
  playback/                                  coordinator/domain/policies [new]
  captions/                                  timeline/adapters [extract later]
  {m3u,xtream,epg,storage,search,enrich}/        retain and adapt
  {mpv-player,media,transcribe,translate,jobs}/ retain and adapt
  capture/                                   timeshift/DVR [future]
tools/xtask/                                  environment/build/research helpers
manifests/                                   native, model, platform, format locks
tests/{catalog,fixtures,e2e,visual}/
docs/{adr,research}/
```

`catalog` and `playback` do not depend on Tauri; Tauri injects repositories, provider adapters and event sinks. Domain code must not import React or platform-native objects. Storage implements domain persistence ports; avoid cycles by placing small port/data contracts in the owning domain crate or a narrow shared module. Start with modules and extract crates only when dependency directions are clear.

## 6. Source ingestion

Adapt existing parsers to emit typed records plus endpoint completeness, rather than directly building frontend rows. The stream stays bounded: network bytes → parser → record channel → staging writer. Limit response size, field length, nesting, redirects, idle time and concurrent endpoint requests.

```rust
struct SourceRecord {
    id: SourceRecordId,
    source_id: SourceId,
    snapshot_id: SnapshotId,
    upstream_key: Option<UpstreamKey>, // namespace + kind + upstream ID
    raw_title: String,
    raw_group: Option<String>,
    raw_epg_id: Option<String>,
    raw_logo: Option<RemoteAssetRef>,
    stream_ref: StreamReference,       // opaque locator, not exported URL
    attributes: BTreeMap<String, BoundedValue>,
    raw_payload_ref: Option<ProtectedBlobRef>,
    metadata_fingerprint: ContentHash,
    locator_fingerprint: ContentHash,
}

enum AdapterEvent {
    Record(SourceRecord),
    EndpointComplete { scope: EndpointScope, result: Completeness },
    Warning(ImportWarning),
}
```

Xtream list and episode endpoints are separate scopes. Lazy episode details become child snapshots with explicit parent/source references; an unfetched season is not an empty season. M3U record order is not stable identity. Duplicate/missing provider IDs produce diagnostics and disambiguated record occurrences; do not overwrite earlier rows silently.

`ImportError` distinguishes authentication, transport, parse, limits, cancellation, storage and integrity errors. UI progress distinguishes fetched records, accepted records, quarantined records, canonical entities and playable variants. These counts are intentionally different.

## 7. Source snapshots

Persist snapshot metadata, normalized SourceRecords and a protected raw payload sufficient for re-parsing. Preserve original title/group bytes and parser version. Byte-exact provider payloads often contain credentials: encrypt retained raw bodies with a profile-scoped key stored in Keychain, restrict file access, exclude them from ordinary diagnostics and never commit them to Git. Record payload hashes and bounded retention.

State machine: `Receiving → Complete/Partial → Compiling → Validated → Published`; terminal alternatives are Failed/Cancelled/Quarantined. Only validated immutable snapshots enter a generation manifest. Retain current and previous successful inputs plus a configurable diagnostic allowance; garbage collection must respect active compilers, playback leases and lineage evidence.

If a movie endpoint succeeds and the series endpoint fails, do not infer deletion of all series. For refresh, carry the previous complete endpoint snapshot into the new manifest with a stale marker. On first import, a partial source can publish available scopes only if the user-facing result clearly reports partial completion. A valid empty authoritative endpoint and a suspicious collapse in counts are separate cases; quarantine destructive-looking refreshes under a documented policy.

A source removal tombstones availability but keeps favorites/history and a clear “source unavailable” state. Credential removal and raw-snapshot purge are separate lifecycle actions so secret retention remains explicit.

## 8. Catalog Compiler

`Compiler::compile(input_manifest, base_generation, rules_fingerprint, overrides)` yields a validated `CatalogPatch` and decision trace. Its deterministic core performs no network/model calls. Optional enrichment is another versioned evidence input, never an import prerequisite.

Stages and outputs:

1. Validate record shape and limits; quarantine invalid entries with reasons.
2. Extract typed claims with offsets and parser rule IDs.
3. Classify kind and structured attributes, preserving ambiguity.
4. Generate conservative identity candidates using indexed blocking keys.
5. Accept, reject or abstain using hard constraints and calibrated evidence.
6. Attach source bindings to playable variants and timeline/edition identities.
7. Construct typed entities, localized names and relationships.
8. Map provider collections into versioned taxonomy and EPG candidates.
9. Validate graph integrity, identity stability, user overrides and completeness.
10. Stage immutable revisions; publish through the storage coordinator.

Use bounded parallelism for pure extraction; resolve connected identity candidate components deterministically. Persist tie-breaking policy so thread scheduling cannot change IDs or winners. Limit candidate buckets and handle common titles without quadratic comparisons. Reject connected-component “chain merges” where A resembles B and B resembles C but A contradicts C.

`CompileContext` includes profile ID, rules versions, data/model fingerprints and cancellation token. Errors are typed: malformed evidence, conflicting identity, unsupported schema, invariant failure, stale base and storage failure. A stale base triggers rebase/revalidation, not an unconditional overwrite.

Idempotence means compiling identical inputs under identical policy and identity ledger produces equivalent memberships, names and bindings, excluding run timestamps. It does not mean generating a new UUID for each repeated record. Store per-stage fingerprints and a compile report; use the report for regression diffs and human review.

## 9. Fact Extraction

Extraction keeps original text, Unicode display text and a separate comparison form. Use explicit Unicode normalization versions; never transliterate or strip Arabic distinctions in the authoritative display name just to improve search.

Token rules produce spans and claims: provider prefix, title, year candidate, season/episode, edition/cut, resolution, codec, HDR, FPS, audio/subtitle language hints and country/region hints. Context matters: `1917` can be a title, `France 24` a channel, `AR` may mean Arabic in a provider convention rather than Argentina, and `IMAX` can carry edition information. Remove a token from the derived title only after preserving its extracted meaning and rule evidence.

For `[FR] DUNE PART TWO (2024) 4K HEVC MULTI`, derive a title candidate and year plus claimed quality/codec/multilingual properties. `[FR]` initially means a provider locale hint, not proven production country or spoken French. Canonical punctuation such as `Dune: Part Two` requires trusted metadata or a correction; the parser must not pretend to know it.

Test extraction as a pure function with multilingual positives and explicit counterexamples. Version rules and snapshot replay; do not retroactively overwrite the original provider title.

## 10. Classification

Use `ContentKind = LiveChannel | Movie | Series | Episode | Event | Radio | Unknown`. The UI initially exposes only supported shelves; unsupported/unknown records remain recoverable in source diagnostics, not silently coerced into Movies.

Endpoint semantics usually outrank filename conventions, but remain evidence: a misconfigured Xtream provider can lie. Separate identity attributes from technical properties. Controlled codes include BCP-47 languages, ISO country codes, versioned Nova region/genre IDs, codec enums with an `Other(code)` escape hatch, rational FPS and structured HDR metadata. Unknown is not false or “English.”

Keep `SourceCollection` for provider group labels, including raw value and source scope. Derive Nova facets independently. Never translate “FR MOVIES 4K” directly into a genre. A radio stream, one-off sports event and 24/7 themed channel need distinct classification tests. Avoid introducing required genres when evidence is weak.

## 11. Canonical naming

Naming protocol v1:

| Entity | Identity/display fields | Excluded from title |
|---|---|---|
| Movie | Canonical title, original title, localized aliases, release year | Codec, resolution, audio language, source |
| Series | Canonical/original/localized titles, debut year where useful | Season, episode, provider release tags |
| Episode | Series ID, numbering scheme, season/episode or airdate, optional episode title | Quality and source suffixes |
| Live channel | Station/service name plus identity-relevant feed/region | Quality tags and backup-source labels |
| Edition | Cut/version label, timeline identity, trusted distinguishing facts | Provider marketing boilerplate |

Preserve proper capitalization from trusted sources; do not blindly title-case all languages. Name precedence is user correction → trusted matched localized metadata → unambiguous cleaned provider title → safe raw fallback. Original title remains separately available. A missing episode number remains null/unknown, never automatically S01E01. Specials and absolute numbering retain their scheme.

Changing a display title never changes entity ID. Reordering or renaming variants never changes variant ID. Common brand collections such as “beIN Sports” group distinct numbered channel entities rather than merging their identities.

## 12. Identity resolution

Use an identity ledger with strong bindings, external IDs, aliases, evidence and explicit cannot-link constraints. External IDs are namespaced by authority **and media kind**; even strong IDs must be checked for contradictory/malformed source evidence. Identity is scoped to one profile for initial implementation; cross-profile dedup is not needed.

Resolution order: existing verified binding → trusted external identity → deterministic compound key with no contradiction → bounded candidate generation → optional semantic/reranking evidence → Accept/Abstain/Reject. A candidate score is not a probability until calibrated on Nova's benchmark. Do not invent a universal 0.95 merge threshold.

Hard blockers include incompatible media kind, conflicting trusted year/remake, distinguishing sequel/channel numbers, incompatible episode numbering and divergent regional/time-shifted feeds. A title/year key is candidate evidence, not globally unique truth. Prefer creating a provisional entity with a stable ID rather than guessing.

A movie work can contain multiple editions; an episode's alternate cut likewise gets a separate timeline. Different audio languages may share a work but are only time-interchangeable after evidence. “BBC One” and “BBC One +1” are related channels, not backup variants. Exact runtime equality is useful evidence, not sufficient proof of matching edits.

**Implementation research.** Mature catalog systems distinguish media versions and external IDs: [Jellyfin versions](https://jellyfin.org/docs/general/server/media/movies/) and [Kodi video versions](https://kodi.wiki/view/Video_versions). Adopt that separation, not their filesystem naming rules. TMDB's [external-ID lookup](https://developer.themoviedb.org/docs/finding-data) supplies candidates, not infallible provider identity. Reject embedding-only merges because semantic closeness cannot establish sameness. Current Nova's title/page grouping is the migration source, not ground truth. Spike C4 benchmarks deterministic resolution against optional embeddings/reranking with provider-held-out no-merge pairs (§74).

## 13. Stream variants

Use three levels: `Entity → Edition/Timeline → StreamVariant → SourceBinding`. A live channel has a feed identity; no movie-style edition is required. A variant represents a playable presentation from a source; a binding maps it to a particular provider record/locator revision.

```rust
struct StreamVariant {
    id: VariantId,
    entity_id: EntityId,
    edition_id: Option<EditionId>,
    timeline_id: Option<TimelineId>,
    binding_id: SourceBindingId,
    claimed: ClaimedMediaFacts,
    observed: Option<ObservedMediaFacts>,
    availability: Availability,
    reliability: ReliabilitySummary,
}
struct ObservedMediaFacts {
    content_revision: ContentRevision,
    observed_at: UtcTime,
    duration: Option<MediaTime>,
    video: Option<VideoFacts>, // dimensions, codec/profile, rational FPS, HDR
    audio: Vec<AudioTrackFacts>,
    subtitles: Vec<SubtitleTrackFacts>,
    observer_build: StackFingerprint,
}
```

Keep hints and measurements separate. Playback observations normally win for technical display, but expire after content/locator revision changes and carry their source/time. A transient decoder failure is not permanent codec incompatibility. Observations update a technical revision and invalidate affected projections; they do not silently merge entities.

Reliability is recent, decayed and contextual: success count, startup distribution, stalls, failures, sample count, output route and network context. Avoid ranking an untried source as perfect, or penalizing every source for a local network outage.

## 14. Entity lineage

Keep append-only lineage operations with actor, rule/model version, evidence, timestamp, previous memberships and state-transfer policy. Supported operations: alias, merge, split, redirect, restore and explicit no-merge correction.

For a merge, choose a stable survivor, publish old-ID redirects and recompute memberships atomically. Redirects form an acyclic graph; resolve with bounded traversal and cycle validation. Favorites/watchlist combine by logical OR; explicit removals are retained as ordered actions. History remains an event log. Resume positions remain tied to timeline and variant context; never take the maximum timestamp across unrelated editions.

For a split, create new entity IDs and reassign source bindings based on retained evidence. Provenance-specific history/resume follows the originating variant. Ambiguous old favorites remain represented and flagged for resolution; do not duplicate watch completion to every split child. Save the pre-operation state so correction is possible.

User overrides are immutable inputs to future compiles, with explicit scope and precedence. Reversing a merge creates a compensating operation instead of deleting the past. A generation rollback reverts visibility, not newer user actions; replay their stable-ID references through lineage. Removed sources leave tombstoned identities accessible to history.

## 15. Catalog persistence

Keep one SQLite database per profile; app.db remains the profile registry. Use stable identity tables plus immutable revision tables selected by a profile-wide catalog generation. A generation includes one chosen snapshot per source endpoint scope, allowing partial-refresh preservation. It is not simply “the latest imported source.”

Schema kernel below is executable SQLite DDL for design validation only. IDs are opaque UUID strings in this sketch and typed newtypes in Rust. Production migrations add audited timestamps, bounds, JSON schema validation and foreign-key behavior tests; they must not run this as a wholesale replacement.

```sql
PRAGMA foreign_keys = ON;
CREATE TABLE catalog_source (
  id TEXT PRIMARY KEY, adapter_kind TEXT NOT NULL,
  config_ref TEXT NOT NULL, credential_ref TEXT
);
CREATE TABLE source_snapshot (
  id TEXT PRIMARY KEY, source_id TEXT NOT NULL REFERENCES catalog_source(id),
  endpoint_scope TEXT NOT NULL, status TEXT NOT NULL,
  parser_version INTEGER NOT NULL, protected_payload_ref TEXT,
  payload_hash TEXT NOT NULL, completeness_json TEXT NOT NULL,
  created_at INTEGER NOT NULL, UNIQUE(id, source_id)
);
CREATE TABLE source_record (
  id TEXT PRIMARY KEY, snapshot_id TEXT NOT NULL,
  source_id TEXT NOT NULL, occurrence_key TEXT NOT NULL,
  upstream_key TEXT, raw_title TEXT NOT NULL, raw_group TEXT,
  stream_ref TEXT NOT NULL, attributes_json TEXT NOT NULL,
  metadata_hash TEXT NOT NULL, locator_hash TEXT NOT NULL,
  FOREIGN KEY(snapshot_id, source_id) REFERENCES source_snapshot(id, source_id),
  UNIQUE(snapshot_id, occurrence_key)
);
CREATE INDEX source_record_upstream ON source_record(source_id, upstream_key);
CREATE TABLE catalog_generation (
  id INTEGER PRIMARY KEY, base_id INTEGER REFERENCES catalog_generation(id),
  compiler_fingerprint TEXT NOT NULL, input_manifest_hash TEXT NOT NULL,
  status TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE TABLE catalog_head (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  generation_id INTEGER NOT NULL REFERENCES catalog_generation(id)
);
CREATE TABLE generation_source (
  generation_id INTEGER NOT NULL REFERENCES catalog_generation(id),
  source_id TEXT NOT NULL, endpoint_scope TEXT NOT NULL, snapshot_id TEXT NOT NULL,
  freshness TEXT NOT NULL,
  PRIMARY KEY(generation_id, source_id, endpoint_scope),
  FOREIGN KEY(snapshot_id, source_id) REFERENCES source_snapshot(id, source_id)
);
CREATE TABLE catalog_entity (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN
    ('movie','series','episode','live_channel','event','radio','unknown')),
  created_at INTEGER NOT NULL
);
CREATE TABLE entity_revision (
  id TEXT PRIMARY KEY, entity_id TEXT NOT NULL REFERENCES catalog_entity(id),
  canonical_title TEXT NOT NULL, original_title TEXT, year INTEGER,
  resolution_state TEXT NOT NULL, display_json TEXT NOT NULL,
  revision_hash TEXT NOT NULL, UNIQUE(id, entity_id)
);
CREATE TABLE generation_entity (
  generation_id INTEGER NOT NULL REFERENCES catalog_generation(id),
  entity_id TEXT NOT NULL, revision_id TEXT NOT NULL,
  PRIMARY KEY(generation_id, entity_id),
  FOREIGN KEY(revision_id, entity_id) REFERENCES entity_revision(id, entity_id)
);
CREATE TABLE entity_external_id (
  generation_id INTEGER NOT NULL REFERENCES catalog_generation(id),
  authority TEXT NOT NULL, media_kind TEXT NOT NULL, external_id TEXT NOT NULL,
  entity_id TEXT NOT NULL REFERENCES catalog_entity(id),
  evidence_ref TEXT NOT NULL,
  PRIMARY KEY(generation_id, authority, media_kind, external_id)
);
CREATE TABLE edition (
  id TEXT PRIMARY KEY, timeline_id TEXT NOT NULL, label TEXT
);
CREATE TABLE generation_edition (
  generation_id INTEGER NOT NULL REFERENCES catalog_generation(id),
  edition_id TEXT NOT NULL REFERENCES edition(id), entity_id TEXT NOT NULL,
  PRIMARY KEY(generation_id, edition_id),
  UNIQUE(generation_id, edition_id, entity_id),
  FOREIGN KEY(generation_id, entity_id)
    REFERENCES generation_entity(generation_id, entity_id)
);
CREATE TABLE stream_variant (
  id TEXT PRIMARY KEY
);
CREATE TABLE source_binding_revision (
  id TEXT PRIMARY KEY, variant_id TEXT NOT NULL REFERENCES stream_variant(id),
  source_record_id TEXT NOT NULL REFERENCES source_record(id),
  content_revision TEXT NOT NULL, claims_json TEXT NOT NULL,
  UNIQUE(id, variant_id)
);
CREATE TABLE generation_variant (
  generation_id INTEGER NOT NULL REFERENCES catalog_generation(id),
  variant_id TEXT NOT NULL REFERENCES stream_variant(id), entity_id TEXT NOT NULL,
  edition_id TEXT, binding_revision_id TEXT NOT NULL,
  PRIMARY KEY(generation_id, variant_id),
  FOREIGN KEY(binding_revision_id, variant_id)
    REFERENCES source_binding_revision(id, variant_id),
  FOREIGN KEY(generation_id, edition_id, entity_id)
    REFERENCES generation_edition(generation_id, edition_id, entity_id),
  FOREIGN KEY(generation_id, entity_id)
    REFERENCES generation_entity(generation_id, entity_id)
);
CREATE TABLE metadata_fact (
  id TEXT PRIMARY KEY, entity_id TEXT REFERENCES catalog_entity(id),
  variant_id TEXT REFERENCES stream_variant(id), field TEXT NOT NULL,
  value_json TEXT NOT NULL, origin TEXT NOT NULL, evidence_ref TEXT NOT NULL,
  confidence_class TEXT NOT NULL, producer_fingerprint TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  CHECK((entity_id IS NOT NULL) != (variant_id IS NOT NULL))
);
CREATE INDEX facts_entity_field ON metadata_fact(entity_id, field);
CREATE TABLE lineage_operation (
  id TEXT PRIMARY KEY, operation TEXT NOT NULL, payload_json TEXT NOT NULL,
  evidence_ref TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE TABLE entity_redirect (
  generation_id INTEGER NOT NULL REFERENCES catalog_generation(id),
  old_id TEXT NOT NULL REFERENCES catalog_entity(id),
  new_id TEXT NOT NULL REFERENCES catalog_entity(id),
  operation_id TEXT NOT NULL REFERENCES lineage_operation(id),
  PRIMARY KEY(generation_id, old_id), CHECK(old_id != new_id)
);
CREATE TABLE user_action (
  sequence INTEGER PRIMARY KEY, entity_id TEXT NOT NULL REFERENCES catalog_entity(id),
  action TEXT NOT NULL, payload_json TEXT NOT NULL, occurred_at INTEGER NOT NULL
);
CREATE TABLE resume_state (
  entity_id TEXT NOT NULL REFERENCES catalog_entity(id), timeline_id TEXT NOT NULL,
  variant_id TEXT REFERENCES stream_variant(id), position_us INTEGER NOT NULL,
  content_revision TEXT NOT NULL, updated_at INTEGER NOT NULL,
  PRIMARY KEY(entity_id, timeline_id), CHECK(position_us >= 0)
);
CREATE TABLE catalog_outbox (
  id INTEGER PRIMARY KEY, generation_id INTEGER NOT NULL REFERENCES catalog_generation(id),
  topic TEXT NOT NULL, payload_json TEXT NOT NULL, acknowledged_at INTEGER
);
CREATE INDEX catalog_outbox_pending ON catalog_outbox(id)
  WHERE acknowledged_at IS NULL;
```

Edition and variant ownership is generation-scoped so a merge/split can reassign them without rewriting old generations or changing stable variant IDs. `StreamVariant` in §13 is the resolved domain view, not a one-table row. Edition label corrections need revision rows in production; a genuinely different timeline gets a new identity. Durable observations remain attached to variant/content revision and are interpreted through current membership.

Additional normalized tables: `series_relation` and episode numbering/airdate; generation-scoped `entity_relation`/`entity_alias`; `source_collection` and memberships; typed taxonomy term/membership tables; cannot-link decisions; selected-fact provenance; technical observations; health summaries; state projections; model/index registry; typed EPG bindings/programmes. Add composite indexes for `(generation, kind, sort_key, entity_id)`, `(generation, taxonomy_term, entity_id)`, episode parent/order and `(channel, start_utc)`. Validate relationship kinds, same-source endpoint membership and redirect cycles in the compiler and transaction tests; SQL foreign keys alone cannot express all domain rules.

Stage immutable rows in short batches through one per-profile writer coordinator. Pure compilation and network fetch never hold a write transaction. Before publish, verify all memberships and the expected head, then in one short transaction: compare-and-swap `catalog_head`, mark generation published and insert an outbox event. Retry on a stale base after revalidation. Projectors are idempotent and acknowledge outbox entries only after completing their own durable work.

Reads select a head once and constrain all joins to it. Cursor pagination carries generation, sort/filter digest and last key. Retain generations while leased; an expired cursor returns `GenerationExpired`, never mixes pages. User actions are independent of generation revisions and cannot be lost by re-publication.

**Implementation research.** SQLite permits concurrent readers but only one WAL writer, so extra writer tasks do not add write throughput; long read transactions also obstruct checkpoint progress. Keep WAL/NORMAL as the current repository policy, acknowledging its power-loss durability tradeoff. [SQLite WAL](https://sqlite.org/wal.html) and [isolation](https://sqlite.org/isolation.html) support short publication transactions. SQLx 0.8.6 already routes SQLite work through a [connection worker](https://raw.githubusercontent.com/launchbadge/sqlx/v0.8.6/sqlx-sqlite/src/connection/worker.rs); do not wrap every async query in another blocking worker. A proposed [SqliteRwPool PR](https://github.com/launchbadge/sqlx/pull/4177) is not an available baseline API. Prefer an application writer queue over adopting that proposal. Spike C3 compares staged generations with the existing long import transaction under concurrent reads and watch-progress writes.

## 16. Incremental imports

Separate four hashes: raw payload identity; semantic metadata; locator/credentials revision; compiler-input fingerprint. Token expiry should refresh a locator without forcing title matching and embeddings again. Unknown attributes remain preserved; changes in extraction/rules/model fingerprints invalidate the appropriate stage even when raw input is unchanged.

Reuse unchanged records through existing identity bindings; compile new/changed records and their affected candidate neighborhoods. Removals require a complete authoritative endpoint. Maintain dependency edges from record → fact → entity/variant → projection, so a changed year can invalidate resolution rather than just artwork. Taxonomy changes rebuild taxonomy projections, not durable identities.

Measure 40k, 100k and larger synthetic catalogs with unchanged, 1%-changed and full-refresh workloads. Bound parsing queues, stage buffers and candidate lists. Reuse prepared queries and batched writes; benchmark batch sizes instead of assuming maximum concurrency helps. Preserve current import progress responsiveness and separate fetch time, compilation time, publish pause and later enrichment time.

Concurrent imports may fetch independently but publication is serialized per profile. A job compiles against an explicit source-manifest base; if another source publishes, merge manifests and revalidate impacted candidates. Never overwrite a newer source snapshot with a stale whole-profile generation.

## 17. Metadata enrichment

Retain opt-in TMDB and local embeddings behind job interfaces. Enrichment reads canonical candidates and preserved raw aliases, emits facts with request/response provenance and schedules a small compiler patch. It does not bypass identity validation by overwriting display rows.

Field precedence is contextual: observed stream dimensions beat title hints; a trusted matched metadata release year beats parsing; a user's corrected title beats TMDB; provider-specific stream availability beats an unrelated metadata site's existence. Store competing facts and the selected-fact explanation. Do not invent confidence percentages for deterministic rules; use evidence classes and calibrate probabilistic scores separately.

Use request caching, retry-after handling, bounded queues, negative-cache expiry and endpoint/schema fingerprints. Current “unmatched forever” markers must be invalidated by relevant input/resolver changes. Artwork downloads need size/type validation, cache quotas and source attribution. Failure leaves the previous facts/catalog usable.

**Implementation research.** [TMDB lookup](https://developer.themoviedb.org/docs/finding-data) separates text search from existing-ID lookup. Use ID lookup where trusted and search only as candidate generation. Its [FAQ](https://developer.themoviedb.org/docs/faq) documents attribution and commercial-use distinctions; a user-provided key does not by itself settle Nova's distribution terms. Reject mandatory TMDB import dependencies. Spike C4 measures title/year matching and conflicts on Nova's multilingual corpus.

## 18. Catalog intelligence

Define interfaces around evidence and budgets, not model brands:

```rust
trait CatalogEmbedder: Send + Sync {
    fn fingerprint(&self) -> ModelFingerprint;
    fn embed(&self, batch: &[CatalogText], budget: WorkBudget)
        -> Result<EmbeddingBatch, IntelligenceError>;
}
trait EntityReranker: Send + Sync {
    fn rank(&self, record: &ResolvedFacts, candidates: &[Candidate])
        -> Result<Vec<RankedEvidence>, IntelligenceError>;
}
trait AmbiguityResolver: Send + Sync {
    fn propose(&self, context: &ResolutionContext)
        -> Result<ResolutionProposal, IntelligenceError>;
}
```

Run synchronous inference on bounded workers; contracts include maximum batch/text lengths, deadline/cancellation checkpoints and model/tokenizer fingerprints. An optional LLM may return only schema-validated proposals referencing supplied candidates and evidence. It cannot issue database writes, invent external IDs, fetch URLs or override cannot-link constraints. Provider metadata is untrusted model input, not instructions.

E5 is the current baseline, not the permanent production choice. Benchmark deterministic-only, deterministic+existing E5, and a licensed multilingual alternative/reranker with exact artifact revisions. A local LLM is evaluated only if residual ambiguity remains valuable enough to justify memory, latency and licensing cost. Hold provider/title families out of training/tuning evaluation. Failed inference becomes abstention and does not prevent publication.

## 19. Search architecture

Search returns canonical entity IDs and dialogue hits anchored to timeline/content/audio revision. FTS and embeddings are projections, never identity stores. Store title/original title/aliases, taxonomy and verified metadata in catalog search documents; retain provenance for boosted aliases.

Maintain a projection registry with generation, format/model fingerprint, status and high-water mark. Publish lexical search coherently with the catalog or keep an explicitly generation-pinned previous search view until ready. Semantic enrichment may lag: combine current lexical results only with same-generation valid vectors, otherwise use lexical-only. Never map old vector row numbers into new entity rows by coincidence.

Hybrid ranking uses bounded lexical and semantic candidates with a documented fusion policy, source filters, availability and duplicate-by-entity elimination. Version ranking and normalization. Cache keys include profile, generation, model/index fingerprint, locale, filters and query. Count-based invalidation is insufficient.

Dialogue results contain cue revision, timeline, variant compatibility and target timestamp. A click first finds a playable compatible variant; on unavailable content or incompatible edition, explain rather than seeking into the wrong movie cut.

**Implementation research.** [FTS5](https://sqlite.org/fts5.html) supports indexed text and explicit rebuild mechanics; external-content synchronization needs careful triggers/backfill. Keep the repository's multilingual tokenizer baseline, but evaluate Arabic normalization/recall separately. Prefer local FTS plus bounded vectors over a remote search server. Test equal-count updates, deletion, profile collision, stale generation, unknown model and index corruption. Existing SQLx workers provide async database execution; CPU ranking runs on a bounded inference/compute worker.

## 20. Frontend catalog contract

Rust generates TypeScript DTOs and tagged errors. A representative generated shape:

```ts
interface CatalogItem {
  id: EntityId;
  generation: CatalogGeneration;
  kind: "movie" | "series" | "liveChannel";
  title: string;
  subtitle: string | null;
  year: number | null;
  artwork: Artwork | null;
  genres: GenreId[];
  countries: CountryCode[];
  languages: LanguageTag[];
  qualitySummary: QualitySummary | null;
  variantCount: number;
  availability: Availability;
  favorite: boolean;
  watchState: WatchState | null;
}
interface CatalogPage {
  generation: CatalogGeneration;
  items: CatalogItem[];
  nextCursor: string | null;
  totalEntities: number;
}
```

Commands: `list_catalog`, `get_entity_details`, `list_entity_variants`, `list_series_episodes`, `set_entity_favorite`, `set_entity_watchlist`, `search_catalog`, `resolve_catalog_redirect`, `play_entity`. Inputs carry profile and scoped IDs; raw URLs and parsing conventions never leave the domain boundary for ordinary UI components.

Events: `catalog:published {profileId,generation,previousGeneration,changedKinds}`, `catalog:enrichment:progress`, `user-state:changed`. Publication emits after commit through the outbox; lost events are recovered by comparing generation on reconnect. Query keys include generation and profile. Source display names may appear in advanced version details, but provider hacks, credentials and merge scores are diagnostic-only.

## 21. Browsing UX

Movies show one card per canonical work, with editions and versions inside details. Series show stable series entities with proper season/episode relationships. Live shows individual channels or clearly labeled brand collections; collection counts mean channels, variant counts mean alternatives for one channel.

Counts, facet totals and pagination use the same generation and filters. Empty/search/error/loading states remain explicit. Continue Watching references canonical identities and compatible timelines; unavailable sources keep a recoverable card. Virtualization, lazy artwork and query prefetch remain, but frontend name heuristics and duplicate collapsing disappear after migration parity.

User-visible filters are media type, genre, language, country/region and quality. Source collections are an optional source-oriented view, not primary genres. Source removal, partial refresh and enrichment status appear unobtrusively without blocking navigation.

## 22. Detail-page architecture

One canonical entity owns title/artwork/overview, user state, availability and default Play. Edition selection is visible when cuts differ; quality/language alternatives are normally summarized and tucked into a Versions panel. Never imply all advertised languages coexist on every quality option.

The backend returns a capability matrix per candidate variant: claimed/observed audio languages, subtitles, quality, HDR, duration/edition compatibility, availability and uncertainty labels. The panel can show “French requires another version” without teaching Xtream URL syntax.

Series details fetch seasons lazily by stable series ID and preserve loading completeness. Live details show feed identity, Now/Next and alternate quality/backups, while brand-family pages list distinct channels. A stale details page re-resolves its entity/variant against the current generation before Play.

## 23. Variant selection

`VariantSelector::select(entity, intent, capabilities, context) -> VariantSelection` is a pure, testable policy returning preferred variant, ordered compatible fallbacks and structured reasons.

Apply hard filters first: correct entity/edition, user-pinned source/version, required language/role, supported output/security policy and availability. Then score evidence quality, recent startup/stall reliability, language preference, display/decoder capabilities and desired quality. Treat missing measurements as uncertainty; never equate highest claimed resolution with best experience. Use stable tie-breakers and hysteresis.

Prefer changing an audio track within the active stream over reopening a compatible alternate stream. A manual selection is sticky for the session; failures may offer fallback rather than silently contradicting it. Auto may recover to a compatible source under policy, with a subtle explanation and recorded decision. Do not make continuous cross-provider ABR decisions during a film; initial selection and failure recovery are separate from native HLS rendition adaptation.

Language/quality changes across variants depend on timeline compatibility (§28). Policy output includes rejected candidates/reasons for diagnostics but exports only user-understandable explanations. Test contradictory hints, stale health, no sample data, bandwidth changes, unsupported HDR and provider connection constraints.

## 24. Playback architecture

Introduce `domain` and `runtime` modules within `crates/playback` for types/policies and coordination; extract separate crates only if dependencies justify it. Keep the existing `mpv-player` FFI/surface code behind an engine interface. A profile-scoped `PlaybackCoordinator` owns one foreground session, output selection and resource leases. Catalog reads produce immutable playback requests; a catalog refresh cannot mutate an active stream halfway through playback.

```rust
struct PlaybackRequest {
    profile_id: ProfileId,
    entity_id: EntityId,
    edition_id: Option<EditionId>,
    variant_intent: VariantIntent,
    start: StartIntent, // Beginning, CompatibleResume, MediaTime, LiveEdge
}
struct SessionToken { id: SessionId, epoch: u64 }
enum PlaybackError {
    Unavailable, AuthenticationRequired, ResolveTimeout,
    UnsupportedMedia, DecodeFailed, OutputUnavailable,
    NetworkInterrupted, Cancelled, Internal { diagnostic_id: String },
}
trait PlaybackEngine: Send {
    fn command(&mut self, token: SessionToken, cmd: EngineCommand)
        -> Result<CommandId, PlaybackError>;
    fn capabilities(&self) -> EngineCapabilities;
}
```

The coordinator serializes commands through a bounded mailbox. Latest seek replaces queued obsolete seeks; stop/profile change preempts resolution. A dedicated engine owner handles mpv client calls/events. Render callbacks only request rendering on the surface's serialized render executor. AppKit view/window mutation stays on the main thread. Blocking model/FFmpeg work runs in bounded workers or managed subprocesses, not on the UI or session owner. Child jobs carry cancellation and the session token; late responses are discarded.

Shutdown order: reject commands, cancel resolving/caption jobs, stop accepting render callbacks, drain in-flight render work, free render context while its graphics context is valid, destroy mpv, then release the native surface. A coordinator shutdown must not wait on a worker that is synchronously waiting for the main thread. Encapsulate unsafe callbacks with explicit lifetime owners and test repeated create/close/fullscreen/sleep cycles.

**Implementation research — libmpv embedding.** The [render API contract](https://github.com/mpv-player/mpv/blob/master/include/mpv/render.h) specifies context/lifetime and threading constraints; the [client API](https://github.com/mpv-player/mpv/blob/master/include/mpv/client.h) separates asynchronous replies from playback events. [IINA's contributor architecture](https://github.com/iina/iina/blob/develop/CONTRIBUTING.md) demonstrates separating player logic, mpv control and video/window ownership. Adopt that boundary, not IINA code or a wholesale application architecture. Reject raw FFmpeg playback and a browser-engine replacement because neither solves Nova's existing integration debt cheaply. Pin headers and linked library together; an option accepted by standalone mpv is not proof the embedded render API supports it. Spike M1 must exercise cancellation and teardown under rendering load before moving ownership.

**Implementation research — VideoToolbox.** Apple's [hardware-decoding session](https://developer.apple.com/videos/play/wwdc2020/10090/) distinguishes requesting acceleration from requiring it. Keep mpv responsible for decode negotiation and observe its actual selected decoder; retain a software fallback with a bounded retry. Do not equate `hwdec=auto-safe` with guaranteed hardware decoding for every codec/profile. The historical [mpv frame-drop report](https://github.com/mpv-player/mpv/issues/5536) is a benchmark lead, not evidence of a present Nova bug. Spike M2 compares H.264/HEVC, interlaced sources, 4K and software fallback on pinned binaries; record CPU, dropped frames, decoder and power impact. Reject a second independent VideoToolbox decoding pipeline unless measurements establish a required capability.

**Implementation research — HDR/rendering.** Current Nova is an OpenGL libmpv integration. The [mpv manual](https://mpv.io/manual/stable/) and [gpu-next render API proposal](https://github.com/mpv-player/mpv/pull/16818) do not justify declaring an existing Metal/libplacebo embedding available in Nova. The proposal and maintainer discussion are experimental evidence, requiring a pinned build/prototype. Keep the working path with verified SDR behavior. Spike M3 compares current embedding with an upstream-supported candidate, testing transfer function, color space, tone mapping, external displays, fullscreen and SDR UI over HDR video. Reject a cosmetic HDR toggle until output is measured; fall back to explicitly supported tone-mapped SDR. HDR claims require device/display-specific evidence and decoded color metadata, not a filename tag.

## 25. Playback state machine

Rust publishes a revisioned snapshot with `phase`, `desiredTransport`, `observedTransport`, entity/edition/variant, position, duration, seekable ranges, output, tracks, error and capabilities. Pause intent is orthogonal to buffering: a paused session must not resume because a cache threshold was reached.

| Current phase | Event and guard | Next phase / action |
|---|---|---|
| Idle / Ended / Failed | Play accepted | Resolving; allocate new token, select variant |
| Any active phase | Different Play accepted | Stopping old token, then Resolving new request |
| Resolving | Valid resolved lease | Loading; issue engine load |
| Resolving | Timeout/auth/no candidates | Failed, or bounded candidate retry |
| Loading | File metadata loaded | Stay Loading; publish tracks/capabilities |
| Loading | First usable video frame or audio progress | Active; satisfy desired transport |
| Active | Cache starvation while play desired | Buffering; retain position and pause intent |
| Buffering | Sustained playable data | Active; resume only if play desired |
| Active / Buffering | Seek within verified range | Seeking; coalesce requests |
| Seeking | Seek completes and clock/frame advances | Active or Buffering according to observations |
| Active / Buffering | Alternate variant selected | Switching; checkpoint compatible timeline, resolve replacement |
| Loading / Active / Buffering / Seeking | Recoverable failure within retry budget | Recovering; classify, back off, refresh lease or choose compatible fallback |
| Switching / Recovering | Replacement produces usable output | Active; commit selected variant and clear transient failure |
| Any active phase | Natural VOD EOF, current token, no pending replacement | Ended; episode policy may offer next |
| Any active phase | Stop / profile close | Stopping; cancel jobs, release output/transport leases, then Idle |
| Any active phase | Fatal error or exhausted budget | Failed; release resources and expose retry/change-version |

Use monotonic deadlines for load/seek/recovery and a total session recovery budget. Cancellation is not an error banner. An mpv command reply is not first frame; `file-loaded` is not proof of visible playback; EOF, stop, redirect, replacement and error end reasons must be distinguished. A paused first frame still constitutes a successful load. Audio-only playback succeeds through audio clock/output evidence.

Every event carries session/epoch and sequence. React can request `get_playback_snapshot` after an event gap and rejects older revisions. New catalog generations do not change session epochs; variant replacement does. Tests enumerate transitions, rapid Play/Stop/Play, late old-file events, paused reconnect, seek then switch, keep-open EOF and closing while resolving. Illegal transitions produce bounded diagnostics rather than panics.

## 26. Stream Resolver

`StreamResolver` converts a variant binding into a short-lived `ResolvedStream` containing transport kind, secret locator, request policy, expiry, safe display origin, expected media role and a revocable resource lease. Secret URLs stay in Rust. Resolve credentials immediately before use; refresh expired provider tokens at most once per classified authentication failure before asking the user to repair the source.

Adapters own Xtream URL construction and provider exceptions. The resolver owns redirect limits, network deadlines, cancellation and scheme policy. Probe only when the answer affects behavior; avoid a HEAD request for every Play because providers may reject HEAD or count it as a connection. Reuse successful observed facts with TTL. Distinguish DNS/connect/TLS/HTTP/media-parse failures, with redacted details.

Choose direct engine access for compatible streams, a managed loopback proxy when header propagation, remuxing or bounded retry requires it, and a recorder/timeshift source only when explicitly active. A transport lease tracks provider connection usage across player, probe, ASR and recorder; priorities and deduplication prevent background jobs from starving playback. Loopback endpoints use unguessable session tokens, bind locally, expire with the lease and cannot become general URL-forwarding services.

**Implementation research.** [FFmpeg protocol documentation](https://ffmpeg.org/ffmpeg-protocols.html) supports explicit protocol/header/reconnect policy but does not guarantee every provider follows it. [HLS RFC 8216](https://www.rfc-editor.org/rfc/rfc8216.html) establishes relative URI/rendition/segment behavior: authentication may need propagation beyond the top playlist, but never indiscriminately across origins. Adopt a typed resolver and fixture-tested provider adapters. Reject always-remux and always-proxy defaults because they add latency, failure points and resource cost. Current direct mpv loads bypass parts of Nova's proxy; migrate deliberately instead of assuming centralized networking already exists. Spike N1 compares direct/proxy behavior for redirect chains, expiring URLs, cookies/headers, disconnects and connection-limited providers.

## 27. Live playback

Live policy prioritizes stable continuation, bounded channel-change time and honest live-edge status. Begin with conservative named profiles (`balanced`, `resilient`, optional `lowLatency`) resolved to options supported by the pinned mpv/FFmpeg build. Current 20-second readahead and 150 MiB demux cap are baseline measurements, not universal production tuning. Tune duration and byte ceilings together for high-bitrate channels.

Track startup, current buffered duration where observable, stalls and reconnects. Use hysteresis: sustained trouble can enlarge buffering at the next safe boundary; transient jitter must not oscillate settings or jump sources. Recovery classifies auth, network and decode failure, retries with jitter/backoff and a bounded total deadline, then offers another compatible variant. Persist health as expiring observations, never permanent punishment after one outage.

“Live” means current stream playback; “At live edge” requires a defensible position/range mapping. Plain MPEG-TS may provide no absolute edge. EPG wall time does not prove latency. For HLS, map program date time only when present and coherent; account for discontinuities and local clock uncertainty. Return-to-live seeks to the verified safe edge, not arbitrary duration-minus-one-second. Channel switching cancels the previous load and caption work immediately; no overlapping audio.

**Implementation research.** Apple's [HLS authoring requirements](https://developer.apple.com/documentation/http-live-streaming/hls-authoring-specification-for-apple-devices/) and [LL-HLS guidance](https://developer.apple.com/documentation/http-live-streaming/enabling-low-latency-http-live-streaming-hls) describe source requirements; a client flag cannot convert arbitrary IPTV into LL-HLS. Mature streaming systems distinguish startup, buffering and recovery policies. Adopt measured presets; reject one global minimum-cache configuration. Pinned demuxer behavior and provider discontinuities remain compatibility risks. Spike L1 uses repeatable bandwidth/jitter/loss profiles and real-provider confirmation, comparing first output, stall ratio, live latency when measurable, memory and recovery, including six-hour playback and sleep/wake.

## 28. VOD playback

VOD policy owns duration confidence, seekability, resume and edition compatibility. Save progress after meaningful advancement, on pause/stop/backgrounding and before variant change, with rate-limited database writes. Store media time against entity + edition/timeline + audio provenance; use a monotonic observation sequence so delayed writes cannot move resume backward. Preserve explicit user restarts as intentional actions.

In-stream HLS rendition changes remain engine work. Changing provider/file is a new resolve/load operation. If variants share a verified timeline, carry position, desired pause, speed, language preferences and compatible caption selection. If duration, cuts, intro padding or frame rate imply a different timeline, offer “Start this version” or a verified mapping; do not silently transfer percent progress. On failed switch, retry the previous variant at its checkpoint if its lease remains valid, otherwise show a recoverable error. Commit the new active variant only after usable output.

**Implementation research — variant switching.** HLS rendition alignment from the [HLS specification](https://www.rfc-editor.org/rfc/rfc8216.html) does not extend to two independent provider files. [Jellyfin media version conventions](https://jellyfin.org/docs/general/server/media/movies/) provide a useful separation of work and versions, not a guarantee of timeline equivalence. Adopt transactional session replacement with compatibility evidence. Reject treating arbitrary variants as ABR renditions. Spike V1 tests alternate audio in one stream, aligned encodes, a director's cut, duration mismatch, expired replacement URL and cancellation mid-switch. Expected result: no invalid resume/caption reuse and no duplicate audio; switch interruption is measured rather than promised seamless.

## 29. Series playback

An `EpisodeOrchestrator` reads canonical series/season/episode relations, current user preference and availability. It selects next by explicit sequence metadata; unknown numbering cannot masquerade as episode 1. Specials and alternate episode orders remain distinguishable. Lazy provider episode fetch updates the catalog through the same compiler boundary and exposes incomplete/loading/error states.

On natural EOF, offer a cancellable next-episode countdown only when autoplay is enabled. Seeking near the end or closing the player must not trigger it. A next episode load starts a new token, resolves its own variant and applies language preferences without assuming track indices are stable. Remember watched status separately from resume and allow manual correction. Test season boundaries, missing episodes, unknown order, duplicates, failure during preload and source refresh between episodes.

## 30. QoE

Define metrics before dashboards. `play_requested` starts startup timing; first usable output ends it. Track video first submitted frame and audio clock progress separately; a submitted frame is a proxy, not proof of photons on screen. Report resolver time, engine load time, first-output time, stalls per hour, stall duration/play-intended duration, fatal startup rate, seek recovery, variant-switch interruption and recovery success.

Attach stack version, session token, transport, codec, observed decoder, output and fixture/scenario ID. Avoid raw title, URL, provider account or transcript text. Unknown dropped-frame/live-latency values remain null, not zero. Separate cold/warm model/cache/network runs, user pause/seek from involuntary stalls, and fixture performance from real provider outcomes. Keep bounded local aggregates by default; any remote telemetry requires explicit product/privacy design.

QoE feeds a conservative decaying variant health score. It cannot override edition compatibility or manual selection. Benchmarks use distributions and failure counts, not one successful demo or averages hiding long tails.

## 31. Player diagnostics

Provide an optional diagnostics panel with session phase, selected variant, observed codec/resolution/HDR, decoder, buffer/range availability, output, recent recovery reason, caption job status and stack fingerprint. Advanced details expose timing stages and a bounded event ring with monotonic times. Controls include copy redacted report, export diagnostic bundle and clear local history.

Build export through an allowlist serializer, not regex cleanup of arbitrary debug structs. Strip credentials, full URLs, query strings, filesystem usernames, media titles and transcript text by default; an explicit separate attachment workflow can include selected content. Report stored and observed facts distinctly. Test seeded secrets through every error path, bounded log growth and report generation while playback is failed. Diagnostics must still work when metadata enrichment or a model is unavailable.

## 32. Caption architecture

Create domain modules in `crates/captions` with source adapters, cue timeline, selection policy, cache provenance and translation jobs. A unified control model does not require converting every format to plain text: retain mpv/libass rendering for rich ASS or bitmap subtitles when fidelity demands it; use the Nova text overlay for supported text/AI/dual-caption experiences. Capabilities explain combinations that cannot preserve original styling.

```rust
struct CaptionCacheKey {
    timeline_id: TimelineId,
    audio_fingerprint: AudioFingerprint,
    engine_revision: String,
    model_digest: String,
    extraction_revision: u32,
    alignment_revision: u32,
}
struct CaptionCue {
    id: CueId,
    start_ms: i64,
    end_ms: i64,
    text: String,
    language: Option<LanguageTag>,
    origin: CaptionOrigin,
    stability: CueStability, // provisional or committed
}
```

An extraction coordinator schedules playhead-adjacent windows with bounded lookahead/backfill. Cache by stable media/audio/timeline provenance, not only URL or track index. Seeking reprioritizes and cancels obsolete work; switching audio invalidates incompatible jobs. Persistent cache writes use temporary artifacts, hash verification and atomic publication. Failed/missing/corrupt models leave embedded captions and playback usable.

Keep current Whisper large-v3-turbo-q5_0 + CoreML encoder as the measured baseline candidate, FFmpeg 16 kHz mono extraction and existing stitching/DTW behavior. VAD is currently disabled for a known timing issue; re-enabling requires alignment fixtures, not a configuration cleanup. Installation verifies every model/encoder/tokenizer artifact and runtime compatibility. Do not promise realtime on all machines.

Translation takes committed source cues, source/target language, context policy and exact model/tokenizer/runtime revisions. Cache source-text hash + cue/context revision + languages + model digest; preserve source cues and never overwrite them. Current CT2 runs NLLB on CPU. Translated words do not inherit source-word timestamps as if aligned; cue-level timing is the default. Bound simultaneous ASR/translation/embedding jobs with playback priority, memory ceilings and thermal awareness.

**Implementation research.** [WebVTT](https://www.w3.org/TR/webvtt1/) provides a text cue model, while [libass](https://github.com/libass/libass) addresses richer subtitle rendering; neither justifies flattening every subtitle type. The [whisper.cpp streaming example](https://github.com/ggml-org/whisper.cpp/tree/master/examples/stream) is an experimental pattern, not proof of production live captions. [CTranslate2 hardware support](https://opennmt.net/CTranslate2/hardware_support.html) does not establish a Metal backend for Nova's translator. Adopt source-preserving cue timelines and bounded workers; reject a new general LLM dependency for basic captions. Spike C1 measures word/cue timing, stitching duplicates, translation quality, memory and cancellation across languages, accents, silence and music. NLLB's [model card](https://huggingface.co/facebook/nllb-200-distilled-600M) creates an intended-use/licensing gate before distribution (§66).

## 33. Future live captions

Extend the same caption domain with bounded rolling audio windows, capture sequence, stream epoch and media-to-monotonic clock mappings. A discontinuity/reconnect starts a new epoch; old provisional cues cannot appear over a new channel. Keep a short mutable hypothesis tail and commit older cues. Translation follows committed chunks with bounded lag; do not repeatedly translate every hypothesis.

Prefer reusing the active decoded/ingested audio path where supported and licensed. A second provider fetch consumes a connection and may observe a different timeline; it is a deliberate fallback, not the default. Spike C2 compares tap/IPC extraction versus shared ingest with ASR latency, dropped audio, drift and CPU while playing. Define an explicit delay budget and show “catching up” when exceeded. Pause/drop transcription work before compromising playback. No live-caption release claim until a long-run drift/discontinuity test passes.

## 34. Player frontend architecture

Replace the monolithic player incrementally with `PlayerShell`, `VideoSurfaceHost`, `PlayerChrome`, `TransportControls`, `VodTimeline`, `LiveStatus`, `CaptionOverlay`, `LanguagesPanel`, `VariantPanel`, `EpisodePanel`, `PlaybackErrorView` and `DiagnosticsPanel`. Domain actions are typed commands; components never call arbitrary mpv properties or construct provider URLs.

```ts
interface PlaybackSnapshot {
  sessionId: string;
  epoch: number;
  revision: number;
  phase: PlaybackPhase;
  desiredTransport: "playing" | "paused";
  media: CanonicalPlaybackMedia | null;
  timeline: VodTimelineState | LiveTimelineState | null;
  capabilities: PlaybackCapabilities;
  tracks: PlaybackTrack[];
  error: PlaybackFailure | null;
}
```

Rust owns the authoritative snapshot. A small external store selects slices for controls, captions and timeline; React owns panel visibility, hover/focus and in-progress drag only. The [React external-store contract](https://react.dev/reference/react/useSyncExternalStore) requires stable cached snapshots. Interpolate displayed progress locally between coarse authoritative ticks, rebasing on seek/stall; never broadcast 60 full IPC snapshots per second. Generation guards prevent late events reopening closed controls.

Commands carry request IDs; pending feedback lasts until the matching result or authoritative state arrives. Event loss triggers resync. Tests inject out-of-order events, rapid actions and profile teardown. Migrate controls behind one shell flag so the old player remains a coherent fallback rather than mixing competing state owners.

## 35. Premium macOS design system

Make content dominant, controls calm and status legible. Define tokens for spacing, typography, corner radius, elevation, control hit areas, focus, scrims, motion, semantic colors and caption-safe areas. Use material/glass effects only where they preserve readability and where the underlying native/web rendering supports them reliably. A consistent opaque/translucent fallback is preferable to fragile native imitation.

Apple's [Liquid Glass design session](https://developer.apple.com/videos/play/wwdc2025/219/) and [2026 design principles](https://developer.apple.com/videos/play/wwdc2026/250/) inform hierarchy and restraint; they do not require replacing Tauri. Respect reduced motion/transparency, increased contrast and inactive-window appearance. Keep focus rings visible on bright and dark footage. Animate panel/controls with short interruptible transitions; no motion that hides state or delays input.

Use scalable layouts for minimum supported window, fullscreen, ultrawide and long localized labels. Primary controls remain accessible before overflow. Surface titles and errors use wrapping/truncation with accessible full names; every loading state explains whether media, metadata or AI is pending. Record design tokens in `packages/ui`; avoid a second conflicting native design system.

## 36. Live UI

Show channel identity, Now/Next when available, a small live status and minimal transport. Display a return-to-live action only when a seekable live window exists and the viewer is behind it. Otherwise show pause capability honestly; pausing a non-timeshift source may eventually require reconnecting to live.

Channel navigation remains available without burying video: a transient channel list supports keyboard navigation, favorite filtering and current program. Switching shows selected channel immediately with a tuning state and cancellable retry; only confirmed playback changes the active/audible badge. EPG failure does not block tuning. Variant alternatives live in Quality/Versions; diagnostics and provider details stay secondary.

States to design and test: no EPG, late EPG, unknown edge, timeshift full, buffering, recovery, unavailable channel, audio-only feed, 50 alternatives and source authentication failure.

## 37. Movie UI

Show title/edition, timeline, elapsed/remaining time, play/pause, skip, audio/captions and versions. Resume prompts distinguish a compatible saved position from an unavailable previous cut. A first-time Play follows the selected edition and automatic version policy; quality preferences remain secondary to language and compatibility.

During seeking, preview the target time immediately and mark pending resolution without lying about actual playback. Optional thumbnail previews are a separately budgeted feature. Caption generation has a compact progress/status affordance and never replaces the main playback spinner. End state offers replay, details or next suggestion without treating an ended film as a transport failure.

## 38. Series UI

Add series name, season/episode identity and an episode drawer with watched/resume markers. Next-episode action names its target and supports cancellation; keep it away from caption-safe space. Lazy episode fetch shows loading/error within the drawer while the current episode continues.

Preserve drawer scroll/focus during refresh and resolve IDs before playback. Unknown numbering uses a descriptive title without fabricated S01E01. Specials remain discoverable. Autoplay preferences are explicit, and the player never changes episode merely because a stream briefly reports a short duration or an end event belongs to the previous token.

## 39. Languages/Captions UI

One panel has clearly separated Audio, Captions and Translation sections. Audio options show language plus role (commentary/audio description) and observed format when useful. Track IDs are session-local. A language that requires another stream is labeled as a version change with any restart consequence.

Captions offer Off, available embedded/external tracks and Generate captions where eligible. Translation chooses target language and optional dual display, with model availability/download/error status in context. Fifty languages require search and a scrollable keyboard-accessible list, not a huge nested menu. Never report unsupported translation as ready merely because a language label exists.

Appearance controls preview size, placement, background and contrast. Rich native subtitles explain any styling limitation. Selection preserves user preference across episodes by language/role, while observing actual per-stream availability. Pending translation keeps original captions visible rather than blanking the screen.

## 40. Quality/variant UI

Default to Auto with a short explanation of current choice. List alternatives by meaningful quality, language, edition and source label; distinguish provider claims from verified properties when uncertainty matters. Unsupported outputs are disabled with a reason; temporarily failed versions can be retried.

Same-timeline switches state that playback will continue near the current point; incompatible editions require a clear start/resume choice. Show switching and failure recovery inline, keep selected-versus-active distinct, and avoid selecting a row before output succeeds. A manual choice remains sticky until changed or an explicit recovery choice. Native HLS adaptive quality and alternate provider versions must not be presented as identical controls.

## 41. Timeline architecture

Use a discriminated timeline contract: VOD has duration confidence, seekable ranges, position and optional buffered ranges; Live has optional window start/end, optional program-time mapping, edge confidence and distance behind edge. Missing ranges mean unknown, not zero-length media. Segment discontinuities or recording offsets require timeline epochs.

The UI drag value is local preview; commit through a coalescing seek command. Rebase against a sliding live window and clamp to still-available ranges at commit. Keyboard seeks use backend capability checks. Caption cues, dialogue search hits, resume and thumbnails all reference the same timeline identity and explicit offsets. Search results from another edition cannot seek blindly into the active one.

Test zero/unknown duration, sparse seek ranges, non-zero media start, long recordings, window eviction while dragging, seek cancellation and captions across discontinuities. The timeline must remain usable with screen reader and keyboard without relying on pixel position alone.

## 42. Keyboard/pointer interactions

| Input | Default behavior / scope |
|---|---|
| Space | Play/pause when player scope owns focus; never while typing |
| Left / Right | Seek backward/forward by configured small step when seekable |
| Shift + Left / Right | Larger seek step; range-clamped |
| Up / Down | Navigate an open menu/list; volume only in explicit player control scope |
| M | Mute in player scope |
| F or fullscreen control | Request native fullscreen; synchronize observed state |
| Escape | Close topmost panel, then leave fullscreen; do not stop playback unexpectedly |
| Tab / Shift+Tab | Traverse visible controls in logical order |
| Enter / Space on button | Activate focused control using standard semantics |
| Pointer movement / video single click | Reveal chrome; avoid accidental pause while trying to find controls |
| Video double click | Fullscreen, with click arbitration; controls exempt |
| Timeline click/drag | Preview/commit seek; no seeking outside available ranges |
| System media keys | Route through the same coordinator commands as controls |

Expose discoverable shortcuts, configurable seek intervals and conflict handling with app-wide shortcuts. Menus/sliders receive keys before the player scope. Auto-hide chrome only while playing, pointer inactive, no panel open and no keyboard focus inside controls. Pause, error and assistive interaction keep relevant controls visible. Do not add hover-only functionality. Verify rapid click/double-click and drag release outside the window.

## 43. Accessibility

Use semantic buttons, labeled menus/radio groups and a proper slider; name includes the action, not only an icon. Report elapsed/remaining time through `aria-valuetext` at meaningful changes, avoiding continuous announcements. Focus returns to the invoking button when a panel closes; player open/close has predictable focus restoration. Errors are announced once and include actionable recovery.

Native video and web overlays require one coherent accessibility tree: hide decorative hosts, expose controls once and test the real WKWebView/AppKit combination with VoiceOver. Support keyboard-only use, reduced motion/transparency, increased contrast and scalable text. Captions need readable contrast and placement but do not substitute for audio-description track support. Accessibility remains testable with an unavailable stream.

**Implementation research.** The [WAI-ARIA slider pattern](https://www.w3.org/WAI/ARIA/apg/patterns/slider/) supplies interaction semantics; Apple's [accessibility display-change notification](https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayoptionsdidchangenotification) is a native integration point to verify against the chosen SDK. Adopt semantic web controls with a small native preference bridge, not custom canvas controls. DOM accessibility tests alone cannot prove VoiceOver or native focus behavior. Spike A1 covers fullscreen, panels, sliders and native surface layering on the minimum and current supported macOS versions, with manual assistive-tech verification plus automated semantic checks.

## 44. Arabic/RTL

Store original titles and localized aliases without stripping Arabic or folding distinct letters into canonical identity. Search normalization is a separate versioned projection; fixtures include diacritics, mixed scripts, Arabic/Persian digits and equivalent Unicode representations. Bidi-safe rendering uses `dir=auto` for unknown content and isolation around embedded names/numbers; never construct direction controls from untrusted provider input.

Mirror appropriate layout with logical CSS properties, not all media controls indiscriminately. Time progresses according to the documented product convention and remains consistent between visuals and arrow keys; transport icons do not change meaning unexpectedly. Localize labels/plurals and use locale-aware formatting while preserving machine timestamps. Caption shaping/font fallback and mixed Arabic/Latin lines must be tested on actual macOS rendering. [Unicode's bidirectional algorithm](https://www.unicode.org/reports/tr9/) explains why merely setting a container to RTL does not solve mixed-direction content. Arabic and French/English mixed-provider fixtures are release-gate coverage, not an optional polish pass.

## 45. AppKit/native bridge

Keep a narrow macOS bridge under `apps/desktop/src-tauri/src/platform/macos` for video-host lifecycle, window/fullscreen geometry, display changes, native accessibility preferences, media commands, output routes and optional PiP. Use typed C ABI/Objective-C bindings or a small Swift boundary with explicit ownership. Do not let React call arbitrary selectors or have Swift maintain a second playback state machine.

Bridge commands execute on the main thread and return completion/error events to the coordinator. Handles have generation/lifetime checks; callbacks cannot retain closed windows indefinitely. Surface geometry accounts for backing scale, display changes and fullscreen transitions. Native exceptions/panics cannot cross FFI. Sign and package bridge code with the app.

**Implementation research — AppKit and Tauri/macOS.** [IINA's architecture guidance](https://github.com/iina/iina/blob/develop/CONTRIBUTING.md) supports separating window/video responsibilities. Nova's pinned Tauri/wry/tao stack, not current `main` examples, defines its integration surface. The [Tauri transparent-window issue](https://github.com/tauri-apps/tauri/issues/14394) is a regression lead for compositing tests, not a demonstrated defect in Nova's exact version. Adopt a small bridge and keep native surface ownership explicit; reject a full SwiftUI rewrite and broad private-API hooks. Spike N2 checks z-order, transparent overlays, drag regions, resize, multiple displays, fullscreen, close/reopen and shutdown with the pinned stack. Public SDK availability and deployment-target checks are required for every new symbol.

## 46. PiP

Current mpv playback does not implement PiP. Model output capabilities explicitly: `systemPip`, `miniPlayer`, `externalPlayback`, `captionOverlay`. A floating Nova mini-player can reuse the existing surface/window architecture but must be labeled as such; it is not automatically macOS system PiP.

Apple exposes PiP on [AVPlayerView](https://developer.apple.com/documentation/avkit/avplayerview/allowspictureinpictureplayback). That does not make an arbitrary libmpv OpenGL surface eligible. Spike P1 evaluates a public macOS API path with required formats/captions, versus a Nova mini-player, including ownership, fullscreen, close/restore, audio continuity and sandbox/signing behavior. If a specialized AVPlayer output adapter is needed, use the same coordinator with one active audible engine and a resumable handoff. Reject private PiP APIs and claims based on iOS sample-buffer examples without macOS SDK verification. Ship only the capability actually validated.

## 47. AirPlay

Retain the existing AVFoundation route-picker integration as an output adapter, but centralize state and handoff in Rust. Check resolved format, route and receiver access before switching. Capture compatible playback position and desired pause, prepare external output, then stop/mute local output at the explicit handoff point. On failure restore local playback; do not leave both engines audible.

A loopback URL accessible to the Mac is not necessarily accessible to a receiver. Headers, authentication, codec support and source reachability require real-device tests. React-rendered AI/dual captions are not automatically transmitted with video. Expose supported captions honestly, or add a validated packaging path later. [AVPlayer external-playback documentation](https://developer.apple.com/documentation/avfoundation/avplayer/allowsexternalplayback) is a starting point, not proof every Nova stream is compatible. Spike P2 covers Apple TV/device combinations, live/VOD, route loss, sleep, seeking, credentials and caption behavior. Historical user-confirmed playback is useful evidence but does not replace a repeatable matrix.

## 48. Now Playing/media keys

Publish title, artwork where permitted, elapsed time, duration and transport state through the platform's public Now Playing interfaces. Treat live duration as unknown when appropriate. Remote commands translate into coordinator intents and return success only when accepted; disabled capabilities must not appear executable. Register once for the active application playback owner, unregister on teardown and avoid duplicate handlers after profile switches.

Use Apple's [Now Playable guidance](https://developer.apple.com/documentation/mediaplayer/becoming-a-now-playable-app) and verify symbol availability against the macOS deployment target. Test keyboard media keys, Control Center, headset controls, inactive windows, pause during buffering and another media app taking focus. Do not infer playback from UI button state. Metadata privacy preference can suppress sensitive titles while retaining transport control.

## 49. Timeshift

Build optional `live-buffer` storage behind an ingest lease shared with playback where feasible. Store bounded media segments with sequence, stream epoch, timestamp range, discontinuity information, initialization data and integrity status. Publish only complete usable segments into a local playlist/index. Enforce per-session and total disk/time quotas; readers hold leases so eviction never removes a segment currently in use. On disk pressure, evict unpinned oldest segments or shrink/disable buffering while preserving live playback.


The seekable window derives from actual retained, decodable media, including keyframe/init requirements. Pause holds a cursor, not an unbounded promise; if its range expires, inform the user and offer the oldest retained point or live. Restart recovery scans committed metadata, removes temporary fragments and validates references. Authenticated content remains inside protected profile storage with explicit retention.

**Implementation research.** [FFmpeg segment/muxer options](https://ffmpeg.org/ffmpeg-formats.html) and [Tvheadend's timeshift implementation](https://github.com/tvheadend/tvheadend/tree/master/src/timeshift) illustrate segmentation and bounded storage, but muxer deletion flags alone do not implement crash-safe reader leases or DVR ownership. Adopt a small segment store, not a full TV server. Compare HLS segment reuse with remuxed TS ingest in spike T1: timestamp continuity, keyframe seek, disk writes, reconnect, quota eviction and six-hour drift. Provider connection limits and encrypted/unsupported transports may disable timeshift. Do not ship until live playback survives a full/unwritable disk and process restart.

## 50. DVR

Create separate `RecordingJob`, `RecordingSchedule` and `RecordingAsset` domain types. Job states: scheduled → acquiring → recording → finalizing → completed, with cancelled/failed/partial outcomes. Persist transitions and monotonic progress; write to temporary paths, finalize metadata and atomically publish the asset. A crash can yield a recoverable partial recording, never a falsely completed one.

Scheduled jobs use UTC instants plus original schedule timezone/program reference and padding policy. Sleep/wake, missed starts and DST require explicit outcomes. Provider connection budgets arbitrate foreground playback and recordings; expose conflicts before scheduling. Timeshift segments can be promoted/pinned into a recording with ownership accounting, not copied blindly or deleted by the ring buffer. Recording-library entities link original channel/program metadata and carry their own stable timeline.

**Implementation research.** [Tvheadend's DVR design](https://docs.tvheadend.org/documentation/configuration/digital-video-recorder) supplies useful job/storage concepts; Nova needs only local scheduling and durable files, not a broadcast server architecture. FFmpeg is a candidate recorder/remuxer under the pinned build, not an automatic scheduler. Spike D1 compares standalone recording versus shared ingest, tests abrupt shutdown, disk full, overlapping jobs, daylight-saving changes and source reconnection, and verifies playable output. A user must understand that a sleeping/shut-down Mac may miss recording unless a separately researched public wake mechanism is supported. Recording rights and retention are product constraints; do not implement DRM circumvention.

## 51. EPG

Keep XMLTV import and Now/Next, move them behind bounded parsing and canonical channel bindings. Store source-scoped XMLTV IDs, program UTC start/end, original timezone/offset, titles/languages and provenance. Exact scoped IDs outrank names; ambiguous name matches remain unresolved. Channel variants may share a guide only with evidence; regional feeds and time-shift channels are distinct.

Refresh stages a guide generation and atomically replaces the active scope. Bad timestamps, overlapping shows, oversized XML and partial downloads retain the last valid guide with freshness status. Bound entity expansion and input size. Current-program lookup uses indexed channel/time intervals, not a full scan.

The future grid queries channel/time windows, virtualizes both axes, preserves focus and supports keyboard/VoiceOver navigation. Guide time is wall time; playback timeline mappings remain separate. DVR scheduling references a snapshot of program timing so later guide corrections can be reconciled explicitly. Test DST, midnight, missing offsets, stale guides, multilingual titles and channel rebindings.

## 52. Environment reproducibility

Retain `just` as the public command surface. Make `just setup`, `just check`, `just dev`, `just test-e2e` and `just package` reproducible entry points, with a small documented bootstrap for installing the task runner itself. Pin exact Rust, Node, pnpm and just versions; retain Cargo/pnpm lockfiles and enforce locked/frozen installation. Record Xcode build, SDK, deployment target, architecture and native compiler. A clean-machine run must not depend on an existing Homebrew prefix or shell initialization.

Build or fetch a versioned native SDK containing mpv and its actual dependency closure, FFmpeg, libplacebo/libass where used, licenses and build recipes. Select an explicit target such as Apple Silicon first; Intel/universal distribution is a separate tested matrix. Do not invent a minimum macOS version before checking the existing config, all native libraries and required public APIs together. Record the chosen minimum in the manifest and run on it.

Models, tokenizers and CoreML assets have immutable upstream revision, size, digest, license, runtime compatibility and cache format. Download to temporary files, enforce disk budgets, verify, then atomically install; an interrupted download never looks ready. Offline operation after setup should not resolve mutable `main` URLs.

Proposed commands:

| Command | Contract |
|---|---|
| `nova stack --json` | Report compiled build manifest and actual loaded native/runtime/model versions |
| `nova doctor` | Explain missing/incompatible artifacts, permissions, disk and database health with safe remediation |
| `nova verify-environment --strict` | Compare toolchain, lockfiles, SDK and model manifest; exit nonzero on required drift |
| `just test-e2e` | Start isolated fixture/app profile, run tests and clean owned processes |
| `just package` | Produce an unsigned local package from the verified SDK; signing is a separate privileged CI stage |

Separate build reproducibility, reproducible functional behavior and byte-identical release output. Signing/notarization timestamps can make final artifacts differ even with identical inputs. Preserve build inputs, provenance and pre-sign checksums. Test bootstrap in a disposable clean environment; destructive cleanup of a developer machine is unnecessary.

## 53. Stack manifest

Add `nova-stack.toml` plus a checked schema and a generated immutable `build-stack.json` embedded in the app. The checked-in manifest declares selected versions/compatibility; build output records actual resolved artifacts and hashes. Lockfiles remain authoritative for package resolution; manifest validation catches disagreement rather than maintaining a second resolver.

```toml
# Schema sketch. REQUIRED_* values intentionally fail release validation.
manifest_schema = 1

[nova]
version = "REQUIRED_RELEASE_VERSION"
channel = "alpha"
commit = "BUILD_INJECTED_FULL_COMMIT"

[toolchain]
rust = "REQUIRED_EXACT_VERSION"
node = "REQUIRED_EXACT_VERSION"
pnpm = "9.15.4"
just = "REQUIRED_EXACT_VERSION"

[framework]
tauri_rust = "2.11.2"
tauri_js = "2.11.0"
react = "19.2.6"

[macos]
architecture = "aarch64"
deployment_target = "REQUIRED_VERIFIED_TARGET"
xcode_build = "REQUIRED_BUILD"
sdk_build = "REQUIRED_BUILD"

[media]
native_sdk_revision = "REQUIRED_IMMUTABLE_REVISION"
mpv = "REQUIRED_VERSION_AND_SOURCE_REVISION"
ffmpeg = "REQUIRED_VERSION_AND_CONFIG_HASH"
libplacebo = "REQUIRED_VERSION_OR_NOT_LINKED"
libass = "REQUIRED_VERSION_OR_NOT_LINKED"

[sqlite]
runtime_version = "REQUIRED_MEASURED_VERSION"
source_id = "REQUIRED_SOURCE_ID"
compile_options_sha256 = "REQUIRED_DIGEST"

[catalog]
schema = 1
compiler = 1
normalization = 1
identity_resolver = 1
taxonomy = 1

[data]
database_schema = "REQUIRED_MIGRATION_HEAD"
search_schema = 1
caption_cache = 1
translation_cache = 1
embedding_index = 1

[ai.transcription]
runtime = "REQUIRED_WHISPER_REVISION"
model = "large-v3-turbo-q5_0"
revision = "REQUIRED_IMMUTABLE_REVISION"
sha256 = "REQUIRED_DIGEST"
artifact_manifest = "models/transcription.json"

[ai.translation]
runtime = "ct2rs-0.9.19_WITH_REQUIRED_NATIVE_REVISION"
model = "nllb-200-distilled-600M_BASELINE_PENDING_LICENSE_GATE"
revision = "REQUIRED_IMMUTABLE_REVISION"
sha256 = "REQUIRED_DIGEST"
artifact_manifest = "models/translation.json"

[ai.embedding]
model = "multilingual-e5-small"
revision = "REQUIRED_IMMUTABLE_REVISION"
sha256 = "REQUIRED_DIGEST"
dimensions = 384
```

Version integers above illustrate independently versioned contracts; they do not claim migrations already exist. Native SDK manifests additionally record every dylib's path, ID/rpaths, arch, digest and build flags. Model artifact manifests cover tokenizer, encoder and auxiliary files, not only the largest weight file. VideoToolbox is an OS capability: report OS/hardware and observed decoder, not a fabricated downloadable package version.

Ownership: build/release engineering owns toolchain/native inventory, catalog maintainers own compiler/schema compatibility, caption/search maintainers own their artifact/cache versions. CI validates all changes together. Diagnostic output can safely include versions/hashes, but never environment variables containing credentials.

## 54. Dependency management

| Risk tier | Examples | Required upgrade evidence |
|---|---|---|
| 1 | Rust, Tauri/native webview stack, mpv, FFmpeg, SQLite, model/runtime artifacts | Release notes/source changes, platform issues, targeted regression, actual macOS E2E, performance comparison, packaged smoke |
| 2 | React, TypeScript, major plugins, search libraries | Type/API review, affected unit/integration/component tests, UI smoke and relevant benchmark |
| 3 | Formatting/dev-only helpers | Tool/config compatibility, lockfile and normal CI checks |

Use grouped, small upgrade PRs with a declared reason (security, bug fix, supported capability or maintenance), affected subsystems, before/after stack manifests and rollback plan. Bots may propose updates but cannot auto-merge critical native/model upgrades. Avoid upgrading mpv, Tauri and ASR simultaneously unless compatibility requires it.

Investigate advisories against the actual binary configuration, not package names alone. A fix may require a compatible backport rather than newest major. Compare representative cold/warm runs and retain failure artifacts. Review model license/tokenizer/output changes as API changes. Update behavior/cache versions when outputs or interpretation change, even if database schema does not.

## 55. Version control

Keep Git/GitHub canonical, a protected `main`, short feature branches, reviewed PRs and release tags. Prefer trunk-based integration with feature flags over long-lived architectural branches. Jujutsu may be a local workflow, with no requirement that CI or other contributors use it.

`main` means required checks pass and the commit can produce a valid installable build with declared inputs. Signing secrets are not required for ordinary contributor builds. Require appropriate review for schema/native/security/release changes; never let an AI agent bypass branch protection or publish without authorization. Keep Conventional Commits and a human-maintained release changelog; no agent attribution unless requested.

Organize documentation around one status/roadmap source: `README.md`, accepted `ARCHITECTURE.md`, generated/linked `STACK.md`, `ROADMAP.md`, `CHANGELOG.md`, and focused `docs/{catalog,playback,frontend,testing,environment,releasing,security}.md`. ADRs preserve decisions; agent instructions point to them instead of maintaining a duplicate feature ledger. This blueprint is a proposal until adopted incrementally.

## 56. Testing pyramid

Test contracts at their cheapest reliable layer and preserve a small set of indispensable real-app proofs.

| Layer | Main responsibilities | What it cannot prove |
|---|---|---|
| Pure Rust units/property tests | Parsers, extraction, identity constraints, selector, state transitions, retry budgets | Native rendering, actual network behavior |
| SQLite integration | Generations, constraints, migration, rollback, lineage, user-state mapping, FTS consistency | Provider/API compatibility |
| Service integration | Fixture HTTP, resolver, EPG, cancellation, caption cache/jobs | Window focus or device output |
| React component | Controls, panels, focus semantics, state rendering, clean DTOs | Tauri IPC/native surface correctness |
| Catalog regression | Large multilingual identity corpus and churn metrics | Playback reliability |
| Actual macOS app E2E | Import → domain → IPC → playback → captions → persistence | Every codec/physical output/assistive-tech case |
| Device/manual certification | AirPlay, HDR displays, VoiceOver, sleep/power, signed fresh install | Broad deterministic regression coverage |

The golden flow starts with a clean isolated profile: add fixture provider → import/compile → assert canonical counts → browse movie → Auto chooses expected variant → observe real first output → seek → generate known-speech captions → translate → switch audio language → switch compatible quality/source → quit/relaunch → resume same timeline → dialogue search → seek from result. Also test an incompatible edition switch, ensuring no invalid position/caption reuse. Use known licensed/generated media with embedded timestamp/frame markers, so progress events alone cannot fake video success.

**Implementation research — CI/E2E.** Current [Tauri test documentation](https://v2.tauri.app/develop/tests/webdriver/) describes macOS testing through WebdriverIO's embedded integration; the older limitation applies to standalone `tauri-driver`, not every current approach. [WebdriverIO's Tauri documentation](https://webdriver.io/docs/desktop-testing/tauri/) provides the integration path. Adopt a pinned test-only embedded driver and actual app execution. Reject both browser-only mocks as the sole E2E proof and “macOS E2E is impossible” based on old docs. The driver must be absent from release builds. Spike E1 verifies compatibility with Nova's exact Tauri version, native mpv surface, test isolation and artifacts; supplement DOM assertions with rendered-frame/native-window evidence. An uninstrumented packaged-app smoke remains required.

Mocks remain valuable but must be labeled. Assertions use bounded semantic waits rather than fixed sleeps; every failure retains redacted trace, stack, screenshot and fixture scenario. Quarantine requires owner, expiry and replacement coverage; retries may classify flakiness but cannot turn repeated failures into a green release.

## 57. Catalog regression corpus

Create `fixtures/catalog/` with a versioned case schema, sanitized source records, expected entity/variant/edition relationships, cannot-link constraints and field-level expected facts. Include actual provider-shaped disorder without credentials or copyrighted bulk metadata. Large synthetic expansions test scale while a reviewed multilingual gold set tests semantics.

Cases include remakes/same-title years, regional channels, numbered brand channels, episode/season ambiguity, specials, remasters/director's cuts, Arabic/French/English aliases, punctuation/Unicode, duplicated IDs, tokenized URLs, missing fields, contradictory labels, source disappearance/reappearance, partial refresh and user corrections. Add metamorphic tests: input order does not change output; unchanged replay is idempotent; source reordering does not churn IDs; a weaker hint cannot override a correction; adding a conflicting record cannot transitively merge incompatible entities.

Measure false merges, false splits, classification accuracy, naming changes, unresolved rate and canonical-ID churn separately. **Zero known forbidden merges** is a hard gate; do not hide them inside average accuracy. Resolver/model upgrades generate a reviewable diff with evidence and impact on user state. Store corpus version, compiler/model versions, seed and expected lineage. No baseline regeneration merely to make tests pass.

## 58. IPTV fixture environment

Build a small local test service in `tools/iptv-fixture/`, with M3U, Xtream-like endpoints, XMLTV, files, HLS and optionally MPEG-TS. Use a checked scenario manifest and seeded clock/failure scheduler. Assets include a short speech clip with reference text/times, silence/music, multilingual audio tracks, embedded subtitles, aligned video encodes and a deliberately different cut. Generate media through the pinned FFmpeg build or store legally distributable small checksummed artifacts.

Fault controls cover slow headers/body, dropped segment, transient 404/503, permanent 401, expiring token, redirect across origins, malformed playlist, discontinuity, truncated media, changing live window, connection cap and server restart. Bind to loopback with an isolated temporary profile and ephemeral ports; record scenario IDs in diagnostics. No test requires private provider credentials.

Keep the service independent of Nova's parser logic to avoid identical bugs on both sides. Validate fixture media with an independent probe and expected timestamps. A scenario API advances state deterministically; do not depend on public streams staying online. Real-provider tests are separately consented, credential-safe compatibility checks whose network variability cannot invalidate deterministic unit expectations.

## 59. Player UI Lab

Add a development-only route/window rendering production player components against a typed simulated store. Scenarios include loading, paused, buffering, recovering, failed, audio-only, unknown duration, very long title, 50 languages, unavailable model, dual Arabic captions, tiny window, fullscreen, incompatible edition switch and live-window eviction. Use deterministic clock and artwork; show state/event controls for designers and engineers.

The lab tests presentation and interaction, not mpv. Include a distinct native integration harness for surface layering/resize/lifecycle with real media. Neither harness ships enabled in release; compile-time flags and release binary checks enforce this. Reuse canonical DTO fixtures generated from schemas so mocks do not drift from the backend.

## 60. Visual testing

Capture stable UI Lab states on fixed macOS/scale/font/theme matrices, with animations disabled or time-controlled. Compare controls, clipping, safe areas, RTL, focus and reduced-transparency appearances. Use perceptual thresholds plus human review for intentional changes; never auto-approve new baselines in the change that broke them.

Native video may not appear in a webview screenshot. Use a whole-window capture or instrumented known-frame proof for compositing, and keep dynamic video regions out of pixel-perfect DOM diffs while verifying them separately. Record OS, display scale and font versions. Test minimum supported window, common laptop size and fullscreen; add large text and long localized strings. Visual tests complement functional assertions, not substitute for them.

## 61. PR CI

Required PR jobs: formatting, Clippy/ESLint, TypeScript, Rust/unit/component tests, SQLite schema/migration checks, focused catalog gold corpus, manifest/lockfile validation, secret/dependency/license policy and a short macOS actual-app smoke. Run independent jobs in parallel; share only verified immutable caches. No signing secrets or private provider credentials are available to forked PR code.

The smoke imports the fixture, checks canonical counts, plays a known clip, seeks, closes and relaunches to verify persistence. Native/media changes expand the smoke matrix; catalog changes run churn/lineage tests; model changes run quality/compatibility fixtures. Initially add informational baseline jobs, then make them required once deterministic and owned; do not call main production-ready while essential jobs remain optional.

Archive exact stack and failure artifacts. Cancel obsolete runs safely; never cancel a migration test halfway and count it as passing. Track CI duration/flakiness and improve fixture speed rather than deleting critical integration coverage.

## 62. Nightly CI

Run the full corpus and scale benchmarks, complete golden flow, repeated seek/variant/channel switching, model download corruption/recovery, multilingual captions/translation, profile isolation, interrupted refresh, migration restore, prolonged playback, network fault matrix and resource-leak checks. Include six-hour live playback and repeated open/close cycles on a controlled runner when practical.

Physical-device schedules cover sleep/wake, display/HDR and AirPlay; these may require dedicated hardware and cannot be honestly replaced by hosted-runner mocks. Record which runs are deterministic, hardware-assisted or manual. Retain trend distributions for RSS, CPU, dropped frames and startup, compare only equivalent stacks/hardware, and file actionable regressions with owner and threshold. A failed nightly affecting a release candidate blocks promotion until triaged.

## 63. Release CI

Promote one reviewed commit through alpha/beta/stable with explicit approval at publication. Sequence: clean bootstrap → strict environment/license checks → full required tests → build/package → inspect dependency closure → sign nested code/app → notarize/staple applicable artifacts → verify signatures/Gatekeeper → install the final artifact on a clean machine → uninstrumented playback/persistence smoke → generate final checksums/SBOM/provenance/updater signatures → publish approved artifacts and channel feed.

Test upgrades from supported prior versions with copied fixture databases and user-state assertions; test interrupted and corrupt downloads. Verify release binaries lack E2E driver, debug routes and test credentials. Archive manifests, build logs, native recipes, model inventory, symbols and notarization results with retention policy.

Unsigned local packages are useful developer artifacts but not evidence of successful distribution. Test the final signed/stapled deliverable, not only the pre-sign build. A published artifact is immutable; fixes receive a new version. Release promotion does not rebuild different bytes unnoticed.

## 64. Security

Threat boundaries are provider content, remote media/demuxers, webview content, native FFI, model/binary downloads, local IPC/proxy, profile storage and update distribution. Apply input limits before expensive parsing/inference: total bytes, record count, field length, XML nesting/entities, decompression expansion, redirect count, concurrent requests and execution time. Parse untrusted names as text, never HTML or shell fragments.

Move credentials to macOS Keychain or an explicitly protected vault; bindings store secret references and resolver templates. Raw snapshots that necessarily include credentials require encrypted/protected storage, redaction on export and retention controls. Do not duplicate secret-bearing URLs in captions, search indexes, diagnostics or analytics. Source deletion invalidates leases and offers deletion of associated raw material without destroying independent canonical user state.

Enable an explicit CSP and narrow [Tauri capabilities](https://v2.tauri.app/security/capabilities/) by window/command. Restrict navigation and external URL opening; remote content cannot gain app IPC privileges. Image fetching has its own URL/size/content policy. Private-network IPTV may be legitimate: support explicit configured local sources while preventing arbitrary remote metadata from initiating unrestricted local-network requests. Revalidate redirects and forbid credential forwarding to unrelated origins.

Use typed subprocess arguments, bounded output and controlled environment; no interpolated shell commands. Prefer verified bundled executables. Rust must validate profile ownership of every ID even if the UI supplies it. Protect loopback endpoints and avoid arbitrary filesystem read/write commands. Test malicious playlist strings, traversal, secret-bearing errors, oversized metadata, redirect abuse and cross-profile IDs. Native decoder vulnerabilities require dependency response and crash containment planning; Rust alone does not make C media libraries memory-safe.

## 65. Supply-chain security

Lock Rust/JS dependencies, pin CI actions to reviewed commits, verify native archives/models by digest and retain source/build provenance. Audit dependency/license graphs including transitive dylibs and optional FFmpeg features. Artifact manifests use trusted release-controlled hashes, not hashes downloaded beside an untrusted payload without authentication.

Separate untrusted PR runners from signing/release environments; least-privilege workflow tokens, protected environments and short-lived credentials where supported. [GitHub's secure-use guidance](https://docs.github.com/en/actions/reference/security/secure-use) and [artifact attestations](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations) support commit-pinned workflows and build provenance. Attestation proves a stated build origin, not that dependencies are harmless or output reproducible.

Generate an SBOM covering app crates/packages and native/media/model artifacts. Track advisories with responsible owner, remediation SLA by exploitability and known configuration, and an emergency release path. Check licenses/hashes again during packaging; a developer machine's untracked Homebrew upgrade cannot silently become a release dependency.

## 66. Licensing

Create a release inventory with component, exact revision, build flags, license, redistribution/source obligations, notices and intended use. Review the actual mpv/FFmpeg build and complete dependency closure. [mpv's copyright guidance](https://github.com/mpv-player/mpv/blob/master/Copyright) and [FFmpeg's legal guidance](https://ffmpeg.org/legal.html) make build configuration consequential; do not assume that dynamic linking or including a notice alone satisfies every obligation. Codec patent questions are separate from source-code licenses and depend on distribution/use jurisdiction.

Record libplacebo, libass, Rust/JS dependencies, fonts/artwork and fixture media separately. If a licensing-compatible native configuration cannot provide required playback, resolve the product/distribution choice before building an alpha promise around it. A license audit is a release gate, not a claim that this architecture itself determines legal compliance.

NLLB-200 distilled 600M carries a noncommercial license and research-oriented model-card limitations; current local functionality does not establish suitability for Nova's intended public/commercial distribution. Choose a compatible licensed model or obtain appropriate rights, benchmark the replacement and retain a pluggable translator. Review Whisper/E5 weights and converted artifacts at their exact revisions too. TMDB use requires compliance with [TMDB's terms/FAQ](https://developer.themoviedb.org/docs/faq), attribution and applicable commercial arrangements; a user-provided API key does not automatically settle redistribution rights.

## 67. Packaging

Bundle a verified native runtime with relocatable dylib references, correct architecture/deployment target and all required resources. Inspect `otool` dependency/rpath output and reject unresolved `/opt/homebrew`, developer checkout or accidental system-specific dependencies. Use the [Tauri macOS bundle guidance](https://v2.tauri.app/distribute/macos-application-bundle/) as the app packaging layer, not proof native sidecars are automatically complete.

FFmpeg/Whisper subprocesses, CoreML assets and runtime libraries have explicit paths relative to bundle/resources or managed application support. Large optional models can be verified on-demand downloads with clear disk requirements and offline readiness. Never write into the signed app bundle after installation. Separate user DB/cache/download/recording locations and provide safe cleanup accounting.

Test launch from Finder, a path with spaces, a clean account without Homebrew, offline startup, quarantine/Gatekeeper and read-only app installation location. Verify crash symbols and helper cleanup. Prototype packaging in Phase 0: native dependency relocation, signing compatibility and licensing can invalidate later assumptions if deferred.

## 68. Signing/notarization

Use a release-owned Developer ID identity with protected CI access. Build an explicit nested-code inventory, sign helpers/libraries and the app in the required order with reviewed entitlements/hardened runtime, then notarize and staple supported deliverables. Verify final signatures, entitlements, architecture and Gatekeeper assessment on a clean machine. Do not use blanket entitlement exceptions or `codesign --deep` as a repair strategy for an unknown dependency graph.

**Implementation research.** Apple's [distribution-signing guidance](https://developer.apple.com/documentation/xcode/creating-distribution-signed-code-for-the-mac/) and [notarization documentation](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) govern the process; Tauri packaging complements them. Adopt an explicit native-code graph and narrowly justified entitlements. Reject “works after disabling Gatekeeper” as validation. Exact requirements depend on the pinned Xcode/SDK and linked runtimes; spike R1 signs a minimal but representative mpv + AI helper bundle early, validates clean-machine installation and captures failures. Production publication remains a separate approved action. Signing failure blocks release and must not trigger unsigned fallback distribution under the same channel.

## 69. Updater

Use the Tauri updater only after packaging and schema compatibility gates pass. Maintain separate alpha/beta/stable feeds with immutable versioned artifacts, architecture targeting, minimum supported app/schema constraints and staged promotion. The app verifies the downloaded artifact's updater signature before installation and reports actionable download/verification failures without replacing a working app.

**Implementation research.** The [Tauri updater contract](https://v2.tauri.app/plugin/updater/) signs update artifacts; ordinary feed JSON is not automatically a separately signed metadata system. Adopt HTTPS feeds, protected publication, signed artifacts and strict version/channel selection; add authenticated metadata if the threat model needs stronger rollback/freeze protection. Reject describing the default feed as cryptographically signed metadata. Spike U1 tests wrong key, altered bytes, interrupted download, stale feed, unavailable server, architecture mismatch and key-rotation strategy with the selected plugin version. Key custody/recovery is an operational prerequisite, not an app UI setting.

Before applying an update, checkpoint/backup the database with a consistent mechanism and enforce supported schema ranges on next launch. Binary rollback is safe only when the older binary can read the current schema; otherwise restore an explicitly selected compatible backup, explaining possible loss of later writes. Prefer additive/expand-contract migrations and delayed destructive cleanup. Never automatically downgrade a user database because an update failed. Keep the last known install artifact for recovery, but do not promise zero-loss downgrade after arbitrary schema changes.

## 70. Observability

Use structured `tracing` spans for import, compiler stage, generation publish, resolver, session transition, caption job and release/environment verification. Correlate with random operation/session IDs and profile-local opaque IDs. Bound log file count/size and event-ring memory. Record errors with typed codes, safe context and a diagnostic ID; user copy explains recovery without stack traces or credentials.

Persist coarse local operational counters and recent health observations with retention. Build a redacted support bundle from stack manifest, recent safe events, capability observations, database integrity summary and user-selected reproduction steps. No automatic upload by default. Crash reports require the same privacy boundary; arbitrary native logs can leak URLs and must not bypass sanitization.

Observability tests assert required events for failure paths, redaction, bounded growth and behavior when log storage itself fails. A logging failure must not stop playback. Metrics support decisions, not endless collection: each metric has a definition, unit, owner and acceptance use.

## 71. Performance

Establish baseline on named Apple Silicon hardware, OS, power mode, stack and fixtures; retain cold/warm distributions. The following are **initial proposed budgets**, subject to explicit Phase 0 calibration rather than claims about current Nova:

| Workload | Initial acceptance target |
|---|---|
| 40k-record import | No material regression from measured baseline; bounded memory and UI responsive throughout |
| 200k-record synthetic import | Completes within agreed workstation budget; peak memory grows sublinearly with raw payload via streaming/staging |
| Warm browse/filter/search | p95 domain query ≤150 ms on 40k fixture; UI interaction remains responsive |
| Warm fixture VOD startup | p95 first usable output ≤2 s, excluding deliberate faults/model download |
| Live fixture startup/recovery | Profile-specific p95 target set from segment duration; no impossible universal low-latency claim |
| Catalog publish | Short head-swap transaction, initial p95 ≤100 ms under normal reader load |
| UI | Smooth interaction at display refresh with no repeated >50 ms main-thread tasks caused by progress events |
| Repeated player lifecycle | No sustained RSS/resource growth after warmup across 100 open/close cycles |
| Caption workload | Report realtime factor and playhead coverage per hardware/language; playback priority remains protected |

Use prepared/batched writes, one coordinated writer per profile and short transactions. SQLx already has SQLite worker machinery; benchmark extra task/thread layers instead of adding them automatically. Bound parser queues, metadata fetches and model concurrency. Incremental compilation avoids embedding unchanged records; native frame/progress observations are sampled at useful rates. Query plans and FTS/index sizes belong in benchmark artifacts.

Compare correctness, CPU, RSS, disk writes, latency tails and maintenance complexity when selecting designs. A faster import with destructive false merges fails acceptance. Performance regressions require explicit documented tradeoffs; moving work off the main thread does not by itself reduce total resource cost.

## 72. Failure recovery

| Failure | Recovery and retained truth | Required proof |
|---|---|---|
| Malformed/oversized source | Quarantine record or reject snapshot according to scope policy; keep last published generation | Bad-record corpus and bounded-resource test |
| Corrupt/interrupted snapshot | Digest/complete-marker failure prevents compilation; retry fetch or replay prior valid snapshot | Kill during write and restart |
| Source disappears | Mark binding unavailable; retain entity/user state and other sources | Remove/re-add source identity test |
| Wrong merge | Append correction/cannot-link and split lineage; review ambiguous user-state assignments | Merge→watch→split regression |
| Compiler crash/stale base | Unpublished staging is GC-eligible; rebase/recompile affected scope | Concurrent refresh and kill-at-publish tests |
| SQLite busy/full/corrupt | Bounded retry for busy; stop publication on full; preserve files and offer validated restore for corruption | Fault injection and backup restore |
| Dead stream/token expiry | Classify, refresh once where valid, bounded retry/fallback; no infinite spinner | Resolver failure matrix |
| Decoder/hardware failure | One supported software/alternate retry, then actionable error | Unsupported codec and fallback fixture |
| Native crash | OS crash evidence and safe next-launch recovery; future engine isolation only if data justifies it | Repeated lifecycle/stress tests |
| Missing/corrupt/incompatible AI | Mark unavailable, verify/reinstall; preserve source captions/playback | Partial artifact/cache-version tests |
| Disk pressure | Suspend optional caches/jobs, evict reclaimable data, preserve user DB/recordings | Disk quota simulation |
| Lost/out-of-order UI events | Snapshot resync and token/revision guards | Event reorder/drop tests |
| AirPlay/PiP route failure | Restore compatible local output or clear unavailable state, one audible owner | Device handoff matrix |
| Timeshift/DVR interruption | Recover committed segments/partial asset; never label incomplete recording complete | Crash/disk-full schedule tests |
| Signing/notarization failure | Block publication and preserve diagnostic artifacts | Release rehearsal |
| Update corruption/incompatibility | Reject artifact; retain working install; refuse incompatible DB open | Upgrade/downgrade compatibility matrix |

Backups use SQLite's consistent backup mechanism or a correctly coordinated snapshot, not copying only a live `.db` while ignoring WAL. Test restore before advertising recovery. Cleanup must never delete user media or unrecognized paths; ownership markers and lease/reference accounting scope deletion. Recovery UX names what is retained and what needs user action.

## 73. Architecture Decision Records

Create the following ADRs as implementation work begins; statuses below are **proposed decisions in this blueprint**, not accepted repository ADRs. Each record contains context, dated/versioned evidence, alternatives, decision, consequences, spike result, migration and supersession links.

| ADR | Decision and alternatives | Acceptance evidence |
|---|---|---|
| 0001 Desktop/domain boundaries | Keep Tauri/Rust/React; reject rewrite absent measured blocker | Baseline app/build/package proof |
| 0002 Canonical catalog | Durable entity ledger + generation revisions; reject raw provider rows as UI model | Identity/user-state corpus |
| 0003 SourceRecord and snapshots | Preserve protected raw evidence; replay compiler offline | Snapshot replay/corruption tests |
| 0004 Entities/editions/variants | Separate identity from playable timeline/source | Incompatible-cut switch tests |
| 0005 Conservative identity/lineage | Deterministic evidence + reversible merge/split; models advisory | Zero forbidden merges, lineage proof |
| 0006 SQLite generations | Per-profile SQLite, coordinated writer, short publish CAS | C3 benchmark and crash matrix |
| 0007 Catalog intelligence | Stable model interfaces; deterministic offline core | C4 quality/cost benchmark |
| 0008 Playback authority | Rust coordinator + typed state, libmpv adapter | M1 lifecycle/state tests |
| 0009 Rendering/acceleration | Retain current path pending supported replacement | M2/M3 device benchmark |
| 0010 Resolver/recovery | Typed transport leases and bounded policy | N1/L1 fixtures and provider probe |
| 0011 Captions | Provenance-keyed timeline, format-aware rendering | C1 alignment/translation suite |
| 0012 Native bridge/outputs | Narrow public API bridge; capability-gated outputs | N2/P1/P2/A1 results |
| 0013 Native SDK/stack manifest | Versioned reproducible closure; reject system-runtime release dependency | Clean-machine build/install |
| 0014 Testing | Embedded macOS E2E + native/device proof | E1 actual-app golden flow |
| 0015 Updates/schema compatibility | Signed artifacts, explicit compatibility and safe restore | U1 upgrade matrix |
| 0016 Dependency/release policy | Deliberate tiered upgrades, protected release chain | Rehearsed signed release |
| 0017 Advanced live storage | Shared ingest leases, segment ownership and durable jobs | T1/D1 crash/quota tests |

Do not create dozens of empty ADR files merely to imply decisions are complete. The corresponding phase PR records its tested decision and updates accepted `ARCHITECTURE.md`; this table is the initial decision backlog.

## 74. Implementation research findings

Research was checked on **17 September 2026** against local source/locks and the linked upstream material. Versioned local locks establish Nova's baseline; moving upstream branches and open PRs establish leads only. Older Apple/streaming material was used for stable concepts, with current SDK/build validation still required. Community and issue reports inform tests but do not establish Nova defects without reproduction.

The important findings are practical: retain the working stack; separate canonical identity from timelines and sources; move grouping before pagination; preserve raw evidence for replay; keep SQLite write transactions short; obey libmpv render ownership; measure hardware decoding; avoid assuming Metal/HDR/PiP support; distinguish cross-file switches from ABR; use current macOS E2E options; and validate the actual signed package.

| Research area | Evidence and decision | Remaining experiment / measurable exit |
|---|---|---|
| Identity (C4) | Kodi/Jellyfin/TMDB patterns support work/version/external-ID distinctions, not title-only merging | Deterministic vs E5-assisted vs optional reranker on labeled corpus; report forbidden merges, recall, churn, latency and model cost |
| SQLite (C3) | [WAL documentation](https://sqlite.org/wal.html), [isolation](https://sqlite.org/isolation.html), [SQLx 0.8.6 worker source](https://raw.githubusercontent.com/launchbadge/sqlx/v0.8.6/sqlx-sqlite/src/connection/worker.rs): one writer, snapshot readers, existing worker machinery | 40k/200k full+incremental import, reader latency, WAL growth, checkpoints and crash recovery; compare staged generation vs current transaction |
| SQLite patch gate | Current WAL docs disclose WAL-reset fixes in 3.51.3 and backports 3.44.6/3.50.7; crate version alone does not identify linked patch status | Query actual version/source ID/compile options; select verified patched compatible build before concurrency certification |
| libmpv (M1) | Render/client headers and IINA ownership boundaries support serialized render and separate control/window roles | 100 lifecycle cycles, rapid cancellation and shutdown; no stale callbacks, deadlock or sustained leak |
| VideoToolbox (M2) | Hardware request and actual selected decode path differ | Codec/profile matrix on target hardware, CPU/drop/power comparison, software fallback |
| HDR (M3) | Current OpenGL baseline; gpu-next embedding work is not assumed production API | Color/transfer/display validation and cost comparison; supported SDR fallback required |
| Resolver (N1) | FFmpeg/protocol and HLS contracts plus provider variability favor explicit leases/policies | Redirect/auth/capacity/failure matrix; no secret leakage, bounded recovery |
| Native bridge (N2) | Tauri/native composition requires pinned-stack testing | Resize/fullscreen/multi-display/close/overlay matrix; correct ownership and focus |
| Live (L1) | HLS/LL-HLS requirements make universal low-latency presets unsound | Balanced/resilient profiles under seeded impairments and six-hour run; measured startup/stalls/recovery |
| Variant switch (V1) | Independent encodes need explicit timeline evidence | Aligned vs different-cut tests; correct resume/captions, one audible output |
| Captions (C1/C2) | Text and rich subtitle paths differ; Whisper demo is not production streaming proof | Timing/translation quality and resource matrix; live epoch/drift/cancellation proof |
| PiP/AirPlay (P1/P2) | Public AVKit output APIs have format/platform constraints | Actual supported macOS/receiver matrix; truthful capability and fallback |
| Accessibility (A1) | ARIA patterns plus AppKit preferences require actual VoiceOver validation | Complete keyboard/VoiceOver golden flow, RTL and reduced-motion verification |
| Timeshift/DVR (T1/D1) | FFmpeg/Tvheadend patterns inform segment/job ownership, not a server rewrite | Quota/crash/discontinuity/schedule tests; playable recovered partials |
| E2E (E1) | Current Tauri/WDIO docs support an embedded macOS path | Pinned prototype runs real app and verifies native output; release driver absent |
| Distribution (R1/U1) | Apple signing and Tauri artifact signatures are distinct controls | Clean signed install, notarization, tampered update rejection and schema compatibility |

For every spike, check in a short result document with exact code/build/model versions, hardware/OS, scenario hashes, commands, raw measurements, failures and decision. Repeat trials and report distributions; include implementation/maintenance complexity. A result that disproves the preferred approach changes the ADR. Open questions are tracked work, not silently converted into promised capabilities.

## 75. Risks

| Risk | Severity / likelihood before validation | Mitigation / owner role | Gate |
|---|---|---|---|
| False canonical merge destroys user meaning | Critical / medium | Catalog owner: cannot-link, evidence, lineage, reviewed corpus | Phases 1–3 |
| Timeline mismatch corrupts resume/captions | High / high | Playback/caption owners: editions and fingerprints | Phases 4/7 |
| Native teardown crash/deadlock | High / medium | Media owner: lifetime/thread ownership, M1 | Phase 4 |
| Actual SQLite build lacks required fixes | High / unknown | Storage/release owners: runtime inventory and patched SDK | Phase 0 |
| Import memory/WAL growth at scale | High / medium | Storage owner: bounded staging, writer benchmark | Phase 2 |
| NLLB/native distribution license mismatch | High / unresolved | Product/release owner with qualified review | Phase 0 feasibility; before public distribution |
| Packaged dylibs depend on developer machine | High / high until checked | Release owner: SDK closure and clean install | Phase 0/9 |
| Claimed PiP/HDR/AirPlay behavior unsupported | High / medium | Native/media owner: capability gates and devices | Before exposing promise |
| Provider limits break ASR/recording/playback | High / high for some sources | Resolver owner: shared budgets/leases | Phases 4/7/8 |
| Model changes alter quality/timing silently | High / medium | AI owner: hashes, corpus, format versions | Every model upgrade |
| E2E driver incompatible/flaky | Medium / unknown | Test owner: E1 and native harness fallback | Phase 0 |
| Credentials leak via raw snapshots/logs | Critical / medium | Security/storage owner: vault, protected snapshots, allowlist exports | Phase 1 and every release |
| Update opens incompatible database | Critical / medium | Storage/release owners: schema ranges and restore tests | Phase 9 |
| UI looks correct but is inaccessible | High / medium | Frontend/native owners: real VoiceOver and focus matrix | Phase 6 |
| Advanced live expands scope indefinitely | High / high | Product/architecture owner: optional Phase 8 gates | Release planning |

Roles identify responsibilities, not staffing commitments. Assign named owners when scheduling work. Unknown likelihood means evidence is missing; it is not permission to mark risk low.

## 76. Migration plan

Use additive schemas, adapters and feature flags, with one authoritative owner per subsystem at a time. Keep current user changes and working playback. A rollback flag may restore old UI/runtime behavior only while its data contract remains compatible; irreversible schema cleanup waits until a release window closes. Do not run two independent importers writing conflicting truth.

### Phase 0 — Baseline

- **Objective:** establish what works and can be reproduced before changing architecture.
- **Architecture changes:** introduce version/compatibility inventory, test harness boundary and proposed ADR ownership; no player rewrite.
- **Implementation work:** run existing checks, record known failures, inventory loaded native libraries/SQLite/models, pin toolchain, prototype stack manifest/doctor, bootstrap, fixture and packaging closure. Capture source/test feature status in one ledger.
- **Research required:** E1 embedded macOS E2E compatibility; R1 native signing/license feasibility; verify SQLite patch status and minimum macOS/architecture support.
- **Tests:** existing Rust/frontend suite, clean-profile import/play/relaunch smoke, dependency closure and secret scan. Separate pre-existing failures from new ones.
- **Benchmarks:** 40k baseline import, browse/search, first output, caption realtime factor, memory and lifecycle resource usage on named hardware.
- **Migration strategy:** additive tooling/docs/test code only; test-only driver feature disabled in normal release.
- **Rollback strategy:** remove experimental harness/config without touching user DB; retain measurements and known-good lockfiles.
- **Acceptance criteria:** documented passing baseline or explicitly scoped blockers; clean-machine reproducible unsigned install; real-app smoke proof; native/model/license unknowns assigned; no release claim with unresolved critical blockers.
- **Risks:** system Homebrew dependency, mutable model artifacts, broken local tooling, missing signing access and stale SQLite. None should be concealed by skipping checks.

### Phase 1 — Canonical Catalog Foundation

- **Objective:** introduce durable identity/storage without changing visible browsing behavior.
- **Architecture changes:** SourceRecord, immutable protected snapshots, entity/edition/variant ledger, bindings, generation staging and legacy-ID mapping.
- **Implementation work:** additive migrations; adapters preserve raw fields; snapshot validation/retention; credential references; canonical user-state tables; repository APIs and transactional outbox.
- **Research required:** finalize C3 storage prototype, backup method and source completeness semantics; audit every old user-state/cache reference before mapping.
- **Tests:** schema/FK tests, snapshot replay, cross-profile isolation, malformed input, crash during stage/publish, consistent backup/restore and legacy-map uniqueness.
- **Benchmarks:** snapshot size/parse memory, batch write throughput and reader latency at 40k/200k records.
- **Migration strategy:** shadow-write foundations from existing import results or replayed snapshots; old rows remain the serving path. Map old rows to bindings without auto-merging ambiguity.
- **Rollback strategy:** disable shadow compilation; old tables remain untouched; preserve newly captured snapshots and mapping for reattempt. Restore DB backup only if schema compatibility requires it.
- **Acceptance criteria:** complete scope snapshots replay offline; generation publish is atomic; IDs/user state survive refresh; secrets absent from exported fixtures/logs.
- **Risks:** lossy legacy titles, unavailable original raw input, duplicate IDs and disk growth. Mark lost evidence unknown rather than reconstructing invented originals.

### Phase 2 — Catalog Compiler

- **Objective:** produce correct deterministic canonical catalogs at provider scale.
- **Architecture changes:** explicit extraction/classification/identity/naming/taxonomy/EPG stages, lineage and incremental dependency graph.
- **Implementation work:** conservative rules, confidence/cannot-link records, source completeness handling, generation validator, compiler versioning and reviewed corpus; optional enrichment jobs outside publish transaction.
- **Research required:** C4 deterministic vs model-assisted comparison; C3 full/incremental schema benchmark; model/metadata licensing and artifact verification.
- **Tests:** multilingual regression corpus, input-order/idempotence properties, source removal, partial refresh, merge/split user state, stale-base CAS and negative identity cases.
- **Benchmarks:** 40k/200k full import plus 1% changes; peak RSS, publish latency, reader p95, ID churn and forbidden merge count.
- **Migration strategy:** shadow compile and compare with legacy views; review divergences before enabling canonical reads for internal profiles.
- **Rollback strategy:** keep serving previous generation; disable new compiler version; replay preserved snapshots with prior rules while retaining durable corrections/identity ledger.
- **Acceptance criteria:** zero known forbidden merges, stable replay IDs, explainable unresolved cases, no source-loss publication on incomplete fetch, agreed performance budget.
- **Risks:** overconfident fuzzy merge, taxonomy mistaken for identity, generation GC leaks and inconsistent source scopes.

### Phase 3 — Frontend Catalog Migration

- **Objective:** all browse/details/search consume canonical entities and versions.
- **Architecture changes:** generated DTOs and generation-aware query keys; source adapters disappear from presentation code.
- **Implementation work:** migrate Movies, Series, Live, details, counts/facets, Favorites/Watchlist/Continue Watching and search; lazy episodes enter through compiler. Remove page-level dedup only after parity.
- **Research required:** validate canonical UX against real provider-shaped catalogs; review series ordering and brand/feed distinctions with corpus evidence.
- **Tests:** clean DTO component tests, generation invalidation, redirects/splits, pagination counts, source removal and golden browse→details→Play identity handoff.
- **Benchmarks:** first useful browse, 40k filters/search p95, virtualized scroll and query count.
- **Migration strategy:** enable by media kind/profile cohort; translate old user state through audited legacy mapping, retain ambiguous state for review; version cache keys.
- **Rollback strategy:** old presentation can serve legacy projections while they remain maintained by the same canonical pipeline; do not silently revive stale independent writes. Keep a migration checkpoint.
- **Acceptance criteria:** no provider parsing in normal UI, correct one-work/multiple-version presentation, stable favorites/resume and coherent counts across pages.
- **Risks:** empty/miscounted pages, missing unknown episodes, old route IDs and stale semantic indexes. Fallback to lexical search while compatible vector index rebuilds.

### Phase 4 — Playback Platform

- **Objective:** a single typed Rust playback authority with reliable variant-aware orchestration.
- **Architecture changes:** coordinator/session state, engine adapter, resolver, selection policy, transport leases and QoE.
- **Implementation work:** wrap existing mpv path first, add token/epoch guards and bounded mailboxes, migrate commands/events, implement compatible resume and version switch, centralize native output ownership.
- **Research required:** M1 lifecycle, M2 decoder, N1 transport and V1 switching; M3 rendering only as an isolated candidate experiment.
- **Tests:** state-transition suite, rapid Play/Stop/Seek/Switch, old events, auth/redirect errors, software fallback, incompatible editions and 100 native lifecycle cycles.
- **Benchmarks:** startup/seek/switch interruption, dropped frames, CPU/RSS and shutdown time against baseline.
- **Migration strategy:** adapter uses existing renderer and player UI first; activate new coordinator under one feature flag and retain data contracts for fallback.
- **Rollback strategy:** restore old coordinator path for compatible sessions after stopping new owner; never run both. Preserve canonical identity-to-stream adapter.
- **Acceptance criteria:** one audible engine, no invalid timestamp reuse, bounded failures, no observed lifecycle leak/deadlock and real-app VOD flow passing.
- **Risks:** callback lifetime bugs, misleading first-frame instrumentation, cross-provider constraints and unknown binary behavior.

### Phase 5 — Live Reliability

- **Objective:** predictable tuning, buffering and recovery for supported live transports.
- **Architecture changes:** live-specific policy, observable seek-window/edge confidence and expiring variant health.
- **Implementation work:** measured buffer presets, bounded reconnect/token refresh, compatible fallback, channel-switch cancellation and return-to-live semantics.
- **Research required:** L1 impaired-network matrix and representative provider probes; verify all mpv/FFmpeg options against pinned SDK.
- **Tests:** HLS/TS fixtures, discontinuity, sliding playlists, timeouts, source failure, paused reconnect, rapid channel changes and sleep/wake.
- **Benchmarks:** startup tails, stall ratio, recovery success, memory over six hours and live latency only when measurable.
- **Migration strategy:** default conservative profile; opt-in new policies before widening; record decisions safely for comparison.
- **Rollback strategy:** restore prior policy/options and clear experimental health weighting without losing catalog/user state.
- **Acceptance criteria:** no infinite recovery loop, no duplicate audio, truthful edge controls and agreed six-hour/stress thresholds on supported fixtures/providers.
- **Risks:** providers with connection caps, inaccurate timestamps, unstable encoders and false low-latency expectations.

### Phase 6 — Premium Player Frontend

- **Objective:** a coherent accessible macOS player for Live, Movies and Series.
- **Architecture changes:** decomposed shell/controls, snapshot store, native bridge boundary and design tokens.
- **Implementation work:** timelines, languages/versions, episode drawer, error/recovery views, keyboard/focus, RTL, reduced-motion/transparency and UI Lab. PiP/AirPlay only expose certified capabilities.
- **Research required:** N2 native composition, A1 VoiceOver, P1/P2 output spikes and HIG review against current supported macOS.
- **Tests:** UI Lab/component/visual matrix, actual-app focus/fullscreen, keyboard-only and VoiceOver golden flow, tiny window, long titles and 50 languages.
- **Benchmarks:** main-thread work, render churn, IPC frequency, input responsiveness and surface resize behavior.
- **Migration strategy:** replace one coherent player shell behind a flag, reusing the coordinator; progressively extract existing controls before redesigning all visuals.
- **Rollback strategy:** restore prior shell connected to the same typed state; preserve user preferences and avoid reverting backend truth.
- **Acceptance criteria:** no essential hover-only action, accessible focus/order, correct RTL/mixed text, smooth controls and no unsupported output promises.
- **Risks:** native/web focus mismatch, hidden controls during assistive use and compositing regressions.

### Phase 7 — Caption Platform

- **Objective:** reliable unified selection/timing/cache behavior for captions and translation.
- **Architecture changes:** provenance-keyed cue timeline, format-aware rendering, bounded extraction/model jobs and replaceable translator.
- **Implementation work:** migrate URL/track caches through validated provenance or invalidate safely; retain source captions; verified model installer; dual captions; dialogue search timeline linkage; optional live prototype isolated.
- **Research required:** C1 quality/alignment/resource evaluation, translation license/model decision, C2 live audio capture only if scheduled.
- **Tests:** known speech timing, seek/audio/version changes, silence/music, corrupt model, partial download, cancellation, Arabic shaping, translation cache context and dialogue seek across editions.
- **Benchmarks:** realtime factor, cue coverage/lag, translation quality, memory/CPU and playback QoE while models run.
- **Migration strategy:** read old cache only when timeline/audio identity is provable; otherwise regenerate with explanation. Preserve captions already generated as separately versioned artifacts.
- **Rollback strategy:** retain previous compatible engine/cache reader or disable generated captions; embedded captions and playback remain operational.
- **Acceptance criteria:** no stale-track cues, no wrong-edition dialogue seeks, verified install recovery, agreed language quality/timing and distribution-compatible model choice before release.
- **Risks:** alignment drift, model license mismatch, memory pressure and provider extra-stream limits.

### Phase 8 — Advanced Live

- **Objective:** optionally add timeshift, DVR, full EPG and later multiview without weakening baseline playback.
- **Architecture changes:** segment store with leases, durable recording jobs, guide window queries and shared resource arbitration.
- **Implementation work:** timeshift first, then manual recording, then schedules/recording library; full EPG virtualization; multiview as a separately budgeted capability.
- **Research required:** T1/D1 storage/ingest benchmarks, public sleep/wake scheduling APIs if needed, provider capacity and recording constraints.
- **Tests:** disk full, crash/restart, partial recovery, discontinuity, eviction during seek, overlapping jobs, DST, missed wake and multi-session audio/resource policy.
- **Benchmarks:** disk write rate, seek latency, drift, simultaneous ingest CPU/RSS and decoder limits.
- **Migration strategy:** opt-in new tables/storage quotas with explicit retention; current live path remains available when feature disabled.
- **Rollback strategy:** stop new jobs safely, preserve completed/partial recordings, disable ring buffering and return to direct live; never delete user recordings as rollback.
- **Acceptance criteria:** bounded storage, recoverable playable assets, accurate job state and live playback survives optional-service failure.
- **Risks:** scope expansion, power/sleep limitations, source limits and disk exhaustion. This phase does not block an otherwise validated alpha/beta/stable scope that excludes these features.

### Phase 9 — Distribution

- **Objective:** ship a reproducible, signed, notarized and updateable artifact for the declared support matrix.
- **Architecture changes:** production native SDK, protected release workflow, channel feeds, schema compatibility and support diagnostics.
- **Implementation work:** finish bundling/licenses/notices, Developer ID signing, notarization/stapling, updater keys/feed policy, SBOM/provenance, clean install/upgrade and release runbook.
- **Research required:** complete R1/U1 on final stack, current Apple/Tauri requirements, key rotation/recovery and distribution obligations.
- **Tests:** final artifact Gatekeeper/install, no-Homebrew/offline launch, uninstrumented media smoke, wrong-signature/corrupt update, prior-version upgrades, incompatible schema refusal and restore.
- **Benchmarks:** startup/install/update size/time, disk requirements and packaged playback parity.
- **Migration strategy:** internal signed alpha → limited beta → stable after explicit gates; packaging feasibility already established in Phase 0.
- **Rollback strategy:** stop feed promotion; retain immutable previous artifacts; restore only schema-compatible binary or explicit tested backup recovery. Never silently downgrade data.
- **Acceptance criteria:** all claimed features have current evidence, no unresolved critical license/security/install blockers, release driver absent, exact stack archived and approved publication.
- **Risks:** signing key loss, native entitlement incompatibility, stale feed, model redistribution and unsupported OS combinations.

Phases describe dependency order, not a commitment to serial staffing or dates. Security, tests, documentation and packaging evidence accompany each phase. Alpha can follow a deliberately limited subset; advanced Live/HDR/PiP require their own advertised-capability gates.

## 77. Prioritized engineering backlog

| Priority | Deliverables | Done when |
|---|---|---|
| **Do immediately** | Baseline build/tests and feature evidence ledger; exact loaded native/SQLite/model inventory; stack manifest/doctor; E1 actual macOS smoke; R1 packaging/license feasibility; protected snapshot/identity schema prototype | Unknowns are measured/owned; clean install and fixture playback can be reproduced; no runtime feature claims based only on code presence |
| **Before validated alpha** | Canonical foundation/compiler and conservative corpus; clean browse DTOs; user-state migration; typed playback coordinator/resolver; golden import→play→resume flow; bounded recovery; credential-safe diagnostics | Alpha's declared media types and flows pass on supported hardware with preserved state and no forbidden merges |
| **Before beta** | Full variant/language behavior, caption provenance/translation validation, search consistency, premium accessible shell, repeatable live faults, model installer, signed package/update rehearsal | Long-run and failure matrix pass; VoiceOver/RTL and migration proof exist; beta feature list excludes unproven outputs |
| **Before stable** | Complete support matrix, release/license/security gates, final-package smoke, upgrade/restore and key runbooks, perf thresholds, source/model compatibility documentation | Every supported feature has recent evidence; critical risks resolved; release pipeline is repeatable and approved |
| **Later** | Timeshift/DVR/schedules/full EPG, multiview, live translation, HDR enhancements, native PiP if feasible, advanced fallback/intro detection | Each capability has a bounded spike, product scope, resource budget and acceptance test before implementation |

First implementation slices should be reviewable: (1) inventory/manifest + doctor, (2) fixture/E2E first-output proof, (3) snapshot/SourceRecord persistence with replay, (4) generation/identity kernel with corpus, (5) one canonical Movies vertical slice preserving favorites/resume. Resist a single enormous “new architecture” PR.

Track feature status as **implemented**, **unit/integration verified**, **actual-app verified**, **device verified**, or **release verified**, with evidence date/stack. A checked UI button is not equivalent to E2E coverage. Current ASR/translation and import functionality remain valuable, but current PiP, hardware acceleration, model licensing and full-app validation must be described at their actual evidence level.

## 78. Final recommended architecture

Nova should remain a local-first Tauri 2 application with Rust owning a per-profile SQLite canonical catalog and a libmpv-based playback platform, and React/TypeScript presenting clean domain contracts. Provider adapters preserve protected raw snapshots; a deterministic, versioned Catalog Compiler derives evidence-backed entities, editions, variants, taxonomy and guide bindings, publishes validated generations atomically, and preserves stable identity, lineage and user state. Models and external metadata contribute replaceable evidence through bounded background interfaces.

A Rust coordinator selects compatible variants, resolves secure transport leases and drives one authoritative playback session with explicit state, cancellation, recovery and QoE. The existing native renderer remains until a measured supported replacement wins. Captions and translation share provenance-aware timelines while preserving rich-format capabilities. A small public-API macOS bridge owns window/output integrations; the UI exposes only capabilities proven on the supported platform matrix.

The product is operated through pinned toolchains/native SDKs/models, a validated stack manifest, deterministic catalog/media fixtures, actual macOS E2E plus device tests, deliberate upgrades, clear ADRs and a signed/notarized release pipeline with safe schema/update behavior. Evolve by vertical slices with measurable acceptance and reversible rollout. The standard is not how many systems the diagram contains: it is whether a clean install can import a difficult provider, preserve the user's library, play the right version reliably, recover honestly from failure, and repeat that result after an upgrade.
