# Nova 100K+ Scale & Media Intelligence Architecture

Accepted 2026-09-19. Extends [the production blueprint](nova-production-architecture-blueprint.md). Full source requirements: [accepted request](implementation/media-intelligence-requirements.md). Current implementation evidence: [progress](implementation/progress.md).

## 1. Executive architecture decision

SQLite owns the canonical media graph. Stream provider evidence into immutable snapshots, compile a new generation beside the active one, validate it, then atomically change the active-generation pointer. Browse before enrichment. Reason only about unresolved candidate groups; raw row count never defines the LLM or research workload. Existing serving tables remain authoritative until migration gates pass; isolated new crates are not represented as app integration.

## 2. Workload assumptions

1k is developer smoke, 10k integration, 100k normal production, 250k stress and 500k experimental soak. Record source rows, canonical entities, variants and transcript chunks separately. Use 18 GiB Apple Silicon as the initial measurement host, not a fleet minimum. Test empty, partial and malformed libraries as well as large successful imports.

## 3. SourceRecord architecture

Use streaming M3U and bounded JSON-array framing into preserved SourceRecords. Bound record bytes before allocation, channels by capacity and workers by resource policy. Keep protected transport locators separate from metadata. Never spawn one task per row. The current HTTP M3U parser already streams through a bounded channel; Xtream framing and protected-stage integration remain migration work.

## 4. Snapshot architecture

Source → import session → endpoint receipts → snapshot records. Keep the old generation readable during import. Only complete endpoint receipts authorize removal; partial fetches retain prior membership. Validate zero-count collapses and rejected-record ratios. Commit published generation, bindings and user-state projection atomically; crash before commit leaves the old catalog.

## 5. Incremental compilation

Hash relevant metadata separately from protected locators. Match source-scoped stable upstream keys and previous bindings before title-based candidates. Reuse unchanged compiler results when all compiler/policy versions match. Recompute changed/new records and retire removed bindings only under complete snapshots. Locator-only changes do not change canonical identity. Refresh fixture: 98% byte-identical, 1% metadata edits, 1% replacements (one removal and addition each).

## 6. SQLite persistence design

Plan import_session, source_snapshot, source_record, catalog_generation, entity, edition, stream_variant, source_binding, fact_candidate, alias, cannot_link and job tables. Composite indexes start with profile/source/generation and then blocking/filter fields. One controlled writer batches prepared statements in transactions; benchmark 100/500/1000-row batches. Use WAL, synchronous NORMAL and bounded reader pools. Use checked SQL and generated offline metadata. Shadow tables precede any serving-read switch.

## 7. Canonical catalog design

Entity identity is distinct from edition/timeline, stream variant and source occurrence. One movie can own multiple languages/qualities without merging different cuts, remakes or sequels. Attach watch state to entity plus edition/timeline as appropriate. Raw names remain evidence. Do not promote current page-local UI deduplication or brand-family bundles into canonical identity.

## 8. Candidate blocking

Index movie candidates by kind/title keys/year, episodes by resolved series/season/episode, and live channels by scoped EPG identifiers plus country/network. Retrieve a capped set (normally 3–20; current reasoner cap 16). Oversized blocks subdivide or abstain. Fuzzy/semantic/reranking runs only inside these blocks; never all-pairs similarity. Record candidate-count distribution and comparison count.

## 9. Catalog Intelligence Agent

Expose capability-specific reads for known entities, aliases, prior snapshots, provider context and EPG; bounded metadata/research/asset probes; structured evidence staging. No shell, arbitrary file path, SQL execution or authoritative writes. Typed proposals pass schema, scope, cannot-link and evidence policy. Retrieved instructions remain untrusted data. Independent budgets cover each tool and the whole resolution.

## 10. Knowledge base

Verified identity bindings and multilingual aliases reference evidence and canonical IDs. Alias keys include kind, source/region and discriminators; an alias is not globally unique. Reuse only fresh compatible knowledge and retain negative/cannot-link decisions. User corrections are versioned evidence, not destructive rewrites. Inference-only results cannot automatically become verified reusable aliases.

## 11. Research cache

Persist query/resolution keys, proposed facts, exact source references, retrieval/expiry, resolver/tool/model revisions and validation outcome. Stable identity facts outlive dynamic stream facts. Separate exact-request inference cache (implemented protected derivations) from entity-level verified research cache (pending). Cache negative answers with finite expiry; changed context invalidates them.

## 12. Web research architecture

Run only after bindings, knowledge, deterministic extraction, local candidates and metadata/EPG are insufficient. Coalesce aliases for one entity research problem. Initial ceiling remains three targeted queries per unresolved entity plus global session/network budgets; queued excess is deferred. Require opt-in network policy, bounded bodies/redirects, cancellation and retry-after. Offline mode never blocks browsing.

## 13. Naming intelligence

Preserve raw Unicode text and meaningful numbering. Separate known technical tags into variant hints only when their interpretation is unambiguous. Contextual reasoning proposes a canonical name with candidate evidence; no generic token-stripping rule may decide identity. Version normalization/naming policy and retain localized/original names.

## 14. Language intelligence

Store channel primary language independently from observed stream audio tracks. Provider tags, EPG, known entity and observations are separate assertions. Normalize against a versioned ISO registry; country alone does not establish spoken language. Unknown is preferable to fabricated language certainty.

## 15. Quality intelligence

Parse explicit dimensions deterministically; preserve provider HD/FHD/UHD labels as hints. Playback observations include width/height, codec, FPS, HDR and timestamp. A 4K claim with observed 1080p remains two assertions; observed technical evidence wins for that variant and measurement scope.

## 16. Content classification

Use versioned enums for entity kind and taxonomy IDs for News, Sports, Kids, Documentary, Music, General, Regional, Event, Radio and Other. Model strings cannot create taxonomy nodes. Treat a 24/7 stream, a sports event and a movie as potentially different identities even when their titles overlap.

## 17. Logo intelligence

Resolve a verified entity before acquiring artwork. Try known managed asset, trusted metadata, then bounded research. Validate content type, decode size, dimensions, hash and identity association; reject poisoned or mismatched assets. Store protected origin/provenance and publish only Nova-managed asset references. Lazy decode visible cards.

## 18. Confidence engine

Use evidence authority, freshness, subject association and conflict policy, not model confidence alone. Keep Resolved, ProvisionallyResolved, NeedsResearch, Unknown and Invalid states. Equal-strength contradictions abstain. Existing inference evidence remains provisional; a supported fact is not permission to merge entities. Penalize false merges heavily in acceptance.

## 19. AI model abstraction

CatalogReasoner, SemanticMatcher and IntelligenceProvider describe capabilities, not model brands. Stack/model manifests pin digest, revision, license, quantization and runtime compatibility. Qwen 4B/v2 is the measured development reference, not a production default. Model upgrades require historical Nova eval comparisons and resource measurements.

## 20. Nova Media Graph

The graph is relational domain knowledge: entities connect to editions, variants, people, genres, aliases, EPG, transcripts, chapters, assets, facts and user state. SQLite remains authoritative; no graph database is required. Search, embedding and recommendation indexes are disposable projections with generation/version guards.

## 21. Intelligence Runtime

One owner coordinates model lifecycle, scheduler, governor, persistent jobs, caches and providers. Feature code requests capabilities rather than launching independent model processes. Enforce one shared memory and concurrency budget. Existing per-instance inference permits are a boundary primitive, not the central governor.

## 22. ModelManager

Download optional capability packages to temporary files, verify pinned hashes and compatibility, then atomically install. Track absent/installing/verified/loaded/failed states and reserved memory. Evict optional loaded models under pressure, never active playback requirements. Bind inference requests to an owned, attested runtime process; prevent arbitrary service substitution.

## 23. IntelligenceScheduler

Priorities: realtime captions/translation; interactive search and user requests; background enrichment/research/artwork; idle indexing/maintenance. Use bounded queues with aging for starvation prevention. Yield/cancel background batches at checkpoints. Coalesce entity work and discard stale input generations before applying results.

## 24. ResourceGovernor

Observe playback/caption activity, memory pressure, power/battery, thermal and network state when available. Playback starts by pausing heavy background inference/indexing, then resumes incrementally when safe. Unknown telemetry uses conservative concurrency. Measure playback degradation before relaxing budgets; no unmeasured GPU utilization claim.

## 25. Persistent job architecture

Persist job type, entity/batch scope, priority, attempts, input/model versions, status and lease. Pending/Running/Completed/Failed/Deferred/Stale are durable. On restart requeue expired leases idempotently. Completion verifies current generation before evidence staging. No job-per-row expansion; enqueue unresolved entity groups and bounded index batches.

## 26. Frontend scale architecture

Lightweight query pages, bounded cache lifetime, virtualization and lazy assets. This increment replaces accumulating infinite pages with explicit 200-row pages and releases inactive pages. Advance by fetched database rows, not post-grouping card count. Offset paging is a measured bridge; stable generation-bound keyset cursors belong to the canonical query migration. Keep raw URLs and large evidence outside future card DTOs.

## 27. Search architecture

FTS and alias retrieval over canonical entities, semantic reranking only when installed, transcript search separately scoped to media/timeline. Every query is bounded and filter-aware. Lexical browsing must work with models absent. Preserve profile/generation/model/index versions in search caches. Raw variants appear only in explicit variant queries.

## 28. Embedding architecture

Embed canonical entity documents once, incrementally by document/model digest. Use a disk-backed rebuildable index with controlled dimensions/quantization and bounded batches. Separate catalog vectors from millions of transcript chunks. Never ship vectors to React; report recall, index bytes and rebuild cost before choosing an index implementation.

## 29. Playback Intelligence

Record bounded per-variant startup, success/failure, buffering, reconnects and technical observations with recency. Do not retain credentials in diagnostics. Feed observed properties back as scoped evidence through policy, not title rewrites. Aggregate health with expiry so a transient outage is not permanent condemnation.

## 30. Variant Intelligence

Rank eligible variants by language preference, device/codec/HDR support, observed quality, network budget and recent reliability. A highest advertised resolution is not automatically best. Explain selection succinctly and allow manual override. Preserve semantic edition and timeline constraints.

## 31. Intelligent fallback

On qualified failure, try a bounded number of compatible variants with backoff. Live can restart at live edge; VOD preserves position only when edition/timeline compatibility is established. Never silently jump to a different episode/cut. Emit understandable status and a retry action after exhaustion.

## 32. Content Intelligence

Chapters, scene search, thumbnails, summaries, intro/credits and richer captions are Phase I extensions after catalog/playback stability. Use transcript and sampled-frame caches. Each feature has its own versioned jobs, timeline anchors, quality evals and storage budget; none blocks basic library readiness.

## 33. Transcript-aware timeline

Query only hover/playhead-adjacent transcript and thumbnail windows. Return timestamp/snippet/chapter for the active content/audio/timeline version. Seek uses the same timeline identity as transcription. Cancel obsolete requests on scrub or media change; missing data falls back to timestamp-only.

## 34. Conversational search

An optional command palette translates intent to a validated filter/search AST. Allowlisted operations cover kind, language, dates, watch state and transcript concepts; never model-generated SQL. Show applied filters and support undo. No mandatory chat interface or network dependency.

## 35. Recommendation system

Local explainable ranking combines content similarity, explicit preferences, history, completion, favorites and recency. Store feature/projection versions and allow history reset. Do not train a cloud user profile. Evaluate relevance/diversity and expose reasons such as preferred language or unfinished series.

## 36. Live recommendation system

Rank Live Now using current EPG, preferred channels, time-of-day affinity and recent health. Stale/missing EPG falls back to favorites and recent use. Preserve time zones and programme validity; inferred affinity never changes channel identity.

## 37. Smart chapters

Combine transcript transitions, sampled scene boundaries and audio events into versioned chapter proposals anchored to media timeline. Validate temporal order/duration, merge redundant boundaries and expose editing later. Store Nova metadata; never rewrite media files.

## 38. Smart thumbnails

Sample sparsely; deterministically reject black/blurred/duplicate frames before optional visual scoring. Bound decode workers and disk storage. Cache by content/timeline/frame policy and invalidate on changed edition. Run at idle priority; placeholder remains valid during playback.

## 39. Intro/credits detection

Use repeated audio/visual patterns across related episodes plus position/duration and transcript signals. Require confidence and edition scope before showing Skip actions. False-positive skips are heavily penalized. Recap/intro/credits are independent labels, not a single guessed interval.

## 40. Caption intelligence

ASR can consume a scoped glossary of verified names and media context. Retain original output and correction provenance/timing. Do not let an LLM invent dialogue or change timestamps freely. Evaluate named-entity accuracy and hallucinations across Arabic/French/English and noisy audio.

## 41. Translation intelligence

Use bounded preceding-caption context, verified media glossary and optional speaker context. Keep source/translation aligned by segment IDs and audio/timeline version. Cache includes backend/model/glossary revision. Test names, negation, speaker changes and latency under real playback.

## 42. Local-first/offline behavior

Ordinary browsing, cached knowledge/artwork, lexical search, installed-model inference, transcripts and translations work offline. Missing optional capabilities show an actionable unavailable state. Background network jobs defer without discarding the usable catalog. Reconnection retries use bounded budgets.

## 43. Security

Provider/web/model text is data, never instructions. Vet context before inference; isolate credentials and protected URLs. Scope tools to explicit entities and sources; block arbitrary filesystem/database actions. Enforce size, type, redirect and decode limits. No raw metadata in telemetry. Debug validation data uses an isolated absolute path; release ignores that override.

## 44. 100k benchmark fixture

tools/catalog-scale/provider.mjs streams deterministic M3U without a full response buffer. Families yield live/movie/series variants with Arabic/French/English names, three quality labels, unknown titles, missing artwork and controlled replacements. Media URLs are intentionally unplayable. Revision 1 is exactly 98% unchanged/1% metadata edits/1% replacement. It is a synthetic baseline, not a representative quality corpus.

## 45. 250k stress fixture

Use the same versioned generator at count=250000 with bounded HTTP backpressure. Run normal and revised imports, then browse/filter/page and restart. Keep 500k explicit experimental, not an automatic CI requirement. Add malformed transport and conflicting-identity adversarial corpora separately; generator success does not certify identity quality.

## 46. Performance measurements

Record input count/hash/revision, build/toolchain/host, parse/write/publish/total time, process and WebContent RSS separately, DB/WAL/index bytes, entity/variant counts, AI/cache/web counters, false merges/churn and query p50/p95. Never treat cargo build RSS as app RSS. Establish cold/warm repeated baselines before hard budgets; initially flag >25% time or memory regression on like-for-like runs for investigation, not automatic correctness failure.

## 47. AI quality evaluation

Version ugly-IPTV, search, logo, recommendations, captions, translation and tool-use datasets independently. Keep reviewed holdouts separate from development tuning. Track false merges/splits, naming, language, classification, variant grouping and provenance. Existing 24 synthetic cases are development evidence only. Require zero known forbidden-link regressions before changing defaults.

## 48. Hardware capability tiers

Basic: lexical browsing/playback with optional small inference or deferred jobs. Standard: one bounded catalog or ASR/translation workload under governor. High performance: more idle batching only after memory/coexistence tests. Probe capabilities rather than hardcode a chip model. Tier never disables basic browse/play because optional models are absent.

## 49. Failure handling

Fetch timeout/partial: retain prior endpoint. Parse invalid: quarantine bounded evidence. DB/full disk: rollback generation and report. Model missing/invalid: deterministic browse remains. Research offline: defer. Index failure: rebuild or lexical fallback. Artwork failure: placeholder. Playback failure: compatible bounded fallback. Crash: recover jobs and incomplete staging without publishing it. Key failure: do not replace a key or silently discard encrypted evidence.

## 50. Implementation phases

Phases below are acceptance-gated vertical slices. Complete each visible change with native review steps and test evidence. Architecture coverage does not mean feature implementation. The first active slice is A plus bounded page/accurate progress work from E; B–I remain pending unless explicitly recorded in progress.md.

### Phase A — Scale baseline

- Objective / architecture: Deterministic provider, current parser/native baseline, bounded browse and honest import stages.
- Workload / benchmark: 1k/10k smoke; 100k baseline; 250k stress; 500k optional.
- Tests / E2E: Exact counts/refresh ratios; native import, page navigation, search/filter, restart and offline browse.
- Acceptance / failure gates: Measure time/RSS/DB/query latency; no fictitious completion percentages or full-catalog React arrays.
- Migration / rollback: Synthetic profiles only; rollback fixture/UI changes, retain real user databases.

### Phase B — Streaming ingestion

- Objective / architecture: Wire protected SourceRecords, endpoint receipts, bounded workers and a controlled writer to shadow snapshots.
- Workload / benchmark: 100k and 250k; interrupted/empty/partial endpoints.
- Tests / E2E: Inject parser stalls, malformed records, full disk, cancellation and crash; native previous library stays usable.
- Acceptance / failure gates: Complete acquisition plus structural validation before atomic publication; no half-state; bounded queues.
- Migration / rollback: Shadow writes then opt-in verified activation; rollback read pointer, preserve raw evidence.

### Phase C — Incremental catalog

- Objective / architecture: Canonical entities/variants, indexed blocking, binding reuse and keyset query API.
- Workload / benchmark: 100k baseline plus 98/1/1 refresh; 250k same pattern.
- Tests / E2E: Remakes, sequels, numbered live channels, user state, removal and timeline tests; native refresh/search/details.
- Acceptance / failure gates: Deep processing count equals changed/new plus explicitly invalidated dependencies; no all-pairs matching.
- Migration / rollback: Dual read comparison with stable ID mapping; switch only after migration tests; retain prior generation.

### Phase D — Catalog Intelligence

- Objective / architecture: Vetted context, knowledge/alias store, research cache, bounded tools, provenance/conflict decisions.
- Workload / benchmark: Unresolved entity tail, never one call per source row.
- Tests / E2E: Injection, poisoned assets, missing models, cached/uncached/offline resolution; native provisional/conflict presentation.
- Acceptance / failure gates: No forbidden merges; bounded calls; repeat imports reduce expensive work; browsing does not await enrichment.
- Migration / rollback: Evidence-only shadow proposals before serving; rollback resolver version without deleting original evidence.

### Phase E — Frontend scale migration

- Objective / architecture: Canonical card DTOs, generation cursors, virtualization, lazy managed artwork, readiness/enrichment status.
- Workload / benchmark: 100k/250k queries and sustained navigation.
- Tests / E2E: Native resize/scroll/page/filter/detail/back, accessibility, no-results/error/retry and profile switches.
- Acceptance / failure gates: Bounded retained pages and mounted cards; measure query p95 and input responsiveness; no secrets in card DTOs.
- Migration / rollback: Migrate one route at a time against shadow parity; roll back route binding, not catalog data.

### Phase F — Intelligence Runtime

- Objective / architecture: ModelManager, priority scheduler, governor, durable jobs and process ownership.
- Workload / benchmark: Playback/captions plus background enrichment; restart under memory/power pressure.
- Tests / E2E: Missing/tampered models, interrupted download, sleep/restart, stale lease, pause/resume; native status controls.
- Acceptance / failure gates: Playback priority enforced; optional failures never block browse/play; jobs resume idempotently.
- Migration / rollback: Adopt existing workers behind central APIs one at a time; disable background scheduling for rollback.

### Phase G — Playback Intelligence

- Objective / architecture: Observed facts, health aggregation, variant ranking and compatible fallback.
- Workload / benchmark: Real live/VOD sources with network faults and multiple variants.
- Tests / E2E: Startup/buffering/decoder failures; native manual/auto selection and timestamp preservation.
- Acceptance / failure gates: No switch across incompatible episode/edition; bounded retries; measure reliability and playback stalls.
- Migration / rollback: Observe-only before automatic selection; rollback ranking/fallback policy independently.

### Phase H — Search / Discovery Intelligence

- Objective / architecture: Canonical FTS/semantic indexes, validated intent AST, local recommendations and Live Now ranking.
- Workload / benchmark: 100k entities and separately scaled transcript chunks.
- Tests / E2E: Multilingual relevance, absent models, filter correctness, stale indexes, profile isolation; native command palette.
- Acceptance / failure gates: Measured p50/p95/recall; explainable recommendations; lexical fallback always available.
- Migration / rollback: Build projections beside serving index; atomic index swap; rollback projection version.

### Phase I — Content Intelligence

- Objective / architecture: Transcript timeline, chapters/thumbnails, glossary translation, spoiler-safe summaries and skip candidates.
- Workload / benchmark: Long VOD and multi-episode series, sampled frames and millions of timed chunks.
- Tests / E2E: Timeline alignment, hallucination, glossary, spoiler boundaries and false skip tests; native feature-specific review.
- Acceptance / failure gates: Each optional capability passes its own quality/latency/storage budgets; none blocks core release.
- Migration / rollback: Versioned metadata only; disable capability and rebuild projection for rollback.

## Chief-engineer review contract

Each increment ends with a runnable native app, exact fixture/profile instructions, observed results and a list of untested behavior. User review is a feature gate, not a substitute for automated checks. Synthetic unplayable fixtures validate library behavior only. Real provider/playback/caption qualification requires those actual inputs and is reported separately. Timing budgets are provisional until repeated release-build measurements establish a host-specific baseline.

Nova converts large, chaotic libraries into a progressively improving canonical media graph. Stream evidence, index candidates, batch persistence, reuse verified knowledge, enrich incrementally and prioritize playback. Expensive intelligence should touch only the unresolved tail; repeated imports should become cheaper.
