# Nova — Master Implementation Plan

> Status: living implementation plan
>
> Purpose: guide development from the current working Nova application toward the target architecture without rewriting working systems.

---

## 1. How this document is used

This document is the execution plan for Nova.

It is not a substitute for the long-form product and architecture specification. The architecture specification defines the destination and architectural invariants. This file defines how to reach that destination from the code that exists today.

Every substantial task must:

1. read `CONTRIBUTING.md`;
2. read this file;
3. read `docs/nova-implementation-status.md` if it exists;
4. inspect the relevant source, tests, migrations, and current APIs before editing;
5. read only the relevant sections of the long-form architecture specification;
6. implement one bounded milestone at a time;
7. run the relevant validation;
8. update implementation status and, when discoveries materially change dependencies, update this plan;
9. stop after the requested milestone.

Do not implement multiple future milestones merely because they are listed here.

---

# 2. Core development rule

Nova is an existing application with substantial functionality.

The goal is convergence, not a rewrite.

Before changing a subsystem:

1. inspect the current source;
2. inspect relevant tests;
3. inspect persistence and migrations when applicable;
4. determine what already works;
5. preserve working behavior;
6. implement the smallest missing contract;
7. validate failure and migration behavior;
8. avoid parallel replacement systems.

Do not casually introduce systems named or behaving like:

- `catalog-v2`;
- `search-v2`;
- `tmdb-v2`;
- `ai-v2`;
- `playback-v2`;
- or equivalent parallel replacements.

Existing working systems should be evolved.

---

# 3. Architectural invariants

These rules apply throughout all milestones.

## 3.1 Canonical media

Providers supply raw source material.

Nova owns canonical media identity.

The domain distinction is:

```text
SourceRecord != CanonicalEntity != StreamVariant
```

Provider records retain provider provenance.

Canonical entities own durable user-facing identity and state.

Stream variants represent playable sources for canonical entities.

User state must not depend on volatile provider row IDs.

Canonical identity must survive reasonable provider changes, refreshes, URL changes, metadata enrichment, duplicate consolidation, and identity correction.

---

## 3.2 Provider boundary

Provider-specific structures must stop at the provider/catalog-ingestion boundary.

Provider-specific M3U/Xtream types must not become the durable domain types used by:

- search;
- canonical user state;
- metadata;
- playback UI;
- frontend APIs.

Provider credentials, authenticated URLs, temporary URLs, cookies, and similar secrets remain Rust-side.

---

## 3.3 Ownership

Rust owns durable application truth, including:

- catalog identity;
- persistence;
- provider integration;
- playback coordination;
- search/indexing;
- jobs;
- model management;
- media processing;
- AI-derived artifact provenance.

React owns:

- presentation;
- interaction;
- navigation;
- transient UI state.

React must not become the durable source of truth for catalog, provider, playback, credential, or persistence state.

---

## 3.4 TMDB

TMDB supplies structured media knowledge.

Nova owns canonical media identity.

A TMDB ID is not a Nova canonical ID.

TMDB enrichment attaches structured knowledge to Nova canonical entities.

Low-confidence identification remains unresolved.

AI may reason over real TMDB candidates but must never invent a TMDB ID.

---

## 3.5 AI

AI is a first-class optional capability.

AI may improve:

- transcription;
- translation;
- semantic search;
- catalog ambiguity resolution;
- command interpretation;
- discovery;
- personalization;
- content understanding.

AI is not required for application correctness.

If AI or a model fails:

- the catalog must still open;
- playback must still work;
- deterministic exact search must still work;
- durable user state must remain intact.

Rule:

```text
AI proposes
Nova validates
```

Generative models must not own:

- canonical IDs;
- database migrations;
- credentials;
- snapshot publication;
- durable user-state migration;
- stream URL construction;
- retry policy;
- codec/device compatibility;
- favorites/history correctness;
- exact TMDB factual data;
- the playback state machine.

---

## 3.6 Model infrastructure

Model identity must be independent of filesystem location.

```text
Model ID != filesystem path
```

Models may live in internal storage or on external/removable model libraries.

Disconnecting an external library must degrade only the relevant AI capabilities.

It must not crash Nova, corrupt the catalog, destroy configuration, break playback, or destroy user state.

Models should be loaded lazily and explicitly unloadable.

A model cannot be unloaded while an active inference lease still uses it.

Do not assume all heavyweight models remain resident simultaneously.

---

## 3.7 Playback

Playback truth belongs in Rust.

Target layering:

```text
React
  ↓
Application / Tauri API
  ↓
PlaybackCoordinator
  ↓
StreamResolver
  ↓
VariantSelector
  ↓
Playback policy / QoE
  ↓
libmpv / FFmpeg
  ↓
VideoToolbox
```

Variant selection and recovery must be deterministic and testable.

Do not place an LLM in the playback critical path.

Playback recovery must be finite.

Stale callbacks from an obsolete playback session must never mutate the current playback session.

---

## 3.8 Jobs and resources

Background work must be bounded.

Do not create unbounded per-item task spawning.

Prefer bounded queues, controlled workers, cancellation, and backpressure.

Resource priority:

```text
1. Playback
2. Active real-time transcription / translation
3. Interactive search / navigation
4. Explicit user import / refresh
5. Metadata enrichment
6. Background AI / indexing
7. Maintenance
```

Playback must be able to throttle nonessential expensive AI work.

---

## 3.9 Persistence

Use additive deterministic migrations.

Preferred lifecycle:

```text
ADD
↓
MIGRATE
↓
VERIFY
↓
CUT OVER
↓
REMOVE OLD LATER
```

Do not destructively reinterpret user state.

Migration tests should cover:

- a fresh database;
- representative old databases;
- the complete migration chain;
- integrity checks;
- realistic scale where relevant.

---

## 3.10 Security and privacy

Never expose through logs, React, AI inputs, crash reports, or analytics:

- provider credentials;
- authenticated Xtream URLs;
- API tokens;
- signed stream URLs;
- model-storage authorization data;
- private profile information.

Sensitive URLs must be redacted.

Local AI is the default philosophy.

---

# 4. Execution gates

No milestone is considered complete solely because code was written.

Each milestone must pass the gates relevant to it.

## Gate A — Inspect

Before editing:

- inspect existing implementation;
- inspect existing tests;
- inspect migrations/schema when relevant;
- identify code that should be reused.

## Gate B — Implement

Implement the smallest coherent contract required for the milestone.

Avoid speculative future abstractions.

## Gate C — Test

Run focused tests first.

Then run the broader relevant test/build/typecheck suite.

## Gate D — Failure behavior

Exercise likely failure/cancellation/offline cases.

## Gate E — Compatibility

Verify that existing behavior used by other parts of Nova still works.

## Gate F — Document

Update `docs/nova-implementation-status.md`.

Record major architecture decisions only when they materially affect future work.

## Gate G — Stop

Do not begin the next milestone unless explicitly requested.

---

# 5. Phase 0 — Stabilize the current main version

This phase happens before architectural migration.

The objective is to establish a trusted baseline.

## M0.1 Repository health audit

Inspect the full repository and identify how the project is built and tested.

Run, as applicable:

- Rust formatting/checks;
- Rust unit/integration tests;
- TypeScript typecheck;
- frontend tests;
- linting;
- Tauri build/check;
- database migration tests;
- existing end-to-end or smoke tests.

Do not suppress failing tests merely to obtain green output.

Record the baseline.

## M0.2 Reproduce and fix current bugs

Find and fix real defects in the current main version before beginning planned architecture work.

Prioritize:

1. build/type/test failures;
2. crashes and panics;
3. database/migration corruption risks;
4. playback lifecycle failures;
5. provider import/refresh defects;
6. stale asynchronous callbacks/races;
7. search correctness failures;
8. broken user-state persistence;
9. secret leakage;
10. significant UI regressions.

For each bug:

- reproduce or establish a reliable failing test;
- identify root cause;
- make the smallest safe fix;
- add or strengthen a regression test;
- verify no adjacent behavior regressed.

Do not use the bug-fix phase as permission for major architecture replacement.

## M0.3 Baseline qualification

The stabilization phase is complete only when:

- normal development build succeeds;
- relevant test suites are green or explicitly documented as pre-existing/unsupported;
- known high-severity bugs discovered during the pass are fixed or explicitly blocked with evidence;
- migration/database startup works;
- application startup works;
- core browse/search/playback smoke behavior is viable;
- the repository is in a clean state suitable for architectural work.

Create/update:

`docs/nova-implementation-status.md`

with the stabilized baseline and remaining known defects.

---

# 6. Phase 1 — Architecture planning against the real repository

## M1.1 Integration audit

Audit the actual repository against this plan and the long-form architecture specification.

For each major subsystem determine:

- what exists;
- what is complete enough to reuse;
- what is partially implemented;
- what conflicts with the target architecture;
- where provider-specific types leak;
- where React owns durable truth;
- what persistence already exists;
- what migrations are actually required;
- what tests already cover behavior;
- what hidden coupling or lifecycle risk exists.

Audit at minimum:

- provider ingestion;
- catalog;
- fingerprints;
- snapshots;
- entity matching/reasoning;
- TMDB;
- search;
- storage/migrations;
- jobs;
- transcription;
- translation;
- embeddings/AI if present;
- model infrastructure;
- mpv playback;
- Tauri APIs;
- React DTOs/state.

## M1.2 Source-aware implementation plan

Reconcile future milestone ordering with what is already implemented.

Skip or shrink milestones that are already satisfied.

Reorder later milestones when actual dependencies require it.

Never rebuild an adequate subsystem solely because this plan names it.

---

# 7. Phase 2 — Model infrastructure

Execute only the missing pieces discovered by the audit.

## M2.1 Model library domain

Target concepts:

```text
ModelDescriptor
ModelLibrary
ModelLocation
```

Support stable logical identity and library-relative locations.

## M2.2 Model persistence

Persist libraries, descriptors, locations, availability information, and required metadata through additive migrations.

## M2.3 External/removable model libraries

Implement safe external SSD/library behavior and persistent macOS authorization as appropriate.

Offline libraries must degrade cleanly.

## M2.4 Model registry/discovery

Resolve logical models independently from absolute paths.

## M2.5 Model runtime manager

Support:

- logical model resolution;
- lazy loading;
- explicit unload;
- leases/lifetimes;
- backend adapters;
- availability;
- cancellation;
- resource-awareness hooks.

## M2.6 Resource policy

Integrate model/runtime resource decisions with playback and job priorities.

---

# 8. Phase 3 — Canonical catalog

## M3.1 Canonical domain

Evolve current catalog structures toward explicit separation between:

- SourceRecord;
- CanonicalEntity;
- StreamVariant.

## M3.2 Provider boundary

Contain provider-domain types behind the catalog-ingestion/compiler boundary.

## M3.3 Versioned fingerprints

Classify source records as approximately:

```text
unchanged
changed
new
deleted
```

Reuse expensive derived work when safe.

## M3.4 Candidate blocking

Prevent whole-library O(N²) matching.

Use deterministic blocking evidence.

## M3.5 Entity resolution

Prefer deterministic evidence.

Hard contradictions prevent automatic merges.

Bias toward avoiding false merges.

## M3.6 Identity history

Support aliases, redirects, merge history, and split history where required.

Historical references must deterministically resolve.

User state must survive corrections.

## M3.7 Snapshot publication

Compile, validate, and atomically publish snapshots.

Failed candidates must leave the previous active snapshot intact.

---

# 9. Phase 4 — Metadata and TMDB

## M4.1 Identification

Prefer trusted external IDs / TMDB Find where available.

Otherwise use appropriate movie/TV candidate search.

Record match evidence and provenance.

## M4.2 Core enrichment

Implement Tier 0 / Tier 1 identification and detail/search metadata.

## M4.3 Rich enrichment

Add richer metadata progressively.

Do not eagerly download every possible field for the full catalog.

## M4.4 Series/episode enrichment

Support seasons and episodes lazily and canonically.

## M4.5 Refresh/provenance

Track retrieval/version/fingerprint provenance so derived work can be reused or invalidated correctly.

---

# 10. Phase 5 — Search

## M5.1 Canonical search foundation

Search returns canonical entities.

Preserve exact-title reliability.

## M5.2 Metadata/people indexing

Integrate structured metadata, people, genres, keywords, and related searchable knowledge.

## M5.3 Transcript search

Search stable transcript segments.

## M5.4 Semantic retrieval

Add canonical-entity and transcript embeddings as an additive channel.

Deterministic lexical search continues working when semantic retrieval is unavailable.

## M5.5 Fusion and evaluation

Fuse exact, lexical, metadata, transcript, and semantic signals while preserving deterministic exact retrieval.

Build relevance regression tests.

---

# 11. Phase 6 — Playback

## M6.1 PlaybackCoordinator

Make Rust the authoritative playback session/state owner.

## M6.2 StreamResolver

Contain provider-specific authenticated playback preparation.

## M6.3 VariantSelector

Produce an ordered deterministic list of playable variants.

## M6.4 Recovery

Use bounded recovery:

```text
selected variant
↓
classify failure
↓
bounded transient retry
↓
alternative variant
↓
supported decoder fallback
↓
actionable error
```

## M6.5 QoE

Record useful local playback evidence.

Do not introduce remote telemetry.

## M6.6 Series/autoplay

Request next canonical episodes through the normal playback path.

## M6.7 Live integration

Keep Live TV semantics distinct from VOD assumptions.

---

# 12. Phase 7 — Jobs and resource governance

## M7.1 Bounded queues

Prevent unbounded task spawning.

## M7.2 Cancellation

Propagate cancellation safely through long-running work.

## M7.3 Priority

Apply the global resource priority ordering.

## M7.4 Backpressure

Prevent large imports/enrichment/indexing from overwhelming CPU, memory, storage, or model runtime resources.

## M7.5 Persistence/resume

Persist/resume jobs where it provides meaningful user value and matches existing architecture.

---

# 13. Phase 8 — Intelligence

## M8.1 ASR baseline

Evolve existing transcription rather than replacing it.

## M8.2 Streaming transcription

Target:

```text
playback audio
↓
bounded PCM buffer
↓
VAD/chunking
↓
ASR
↓
partial output + stable timestamped output
↓
transcript persistence
↓
FTS / embeddings
```

Only stable segments become durable transcript knowledge.

## M8.3 Translation

Translate stable timed segments.

Cache by source identity/language/model/version/options.

## M8.4 Embeddings

Primarily embed:

- CanonicalEntities;
- transcript segments.

Version embedding artifacts by model/version.

## M8.5 Dialogue/scene search

Return canonical media plus timestamp and route playback through the normal playback API.

## M8.6 Derived artifacts

Add summaries/themes/scene descriptors only after provenance, versioning, failure behavior, and resource governance are solid.

## M8.7 Command-K

Natural language becomes a typed intent.

A deterministic validator authorizes and executes the intent through existing Nova APIs.

## M8.8 Smart discovery

The LLM interprets intent.

Nova retrieves actual content from the local catalog.

Do not hallucinate catalog results.

## M8.9 Local personalization

Use local behavior signals without requiring remote behavioral tracking.

---

# 14. Phase 9 — Frontend canonicalization

Migrate one vertical slice at a time.

Preferred order:

```text
Movie browse
→ Movie detail
→ Favorite / Watchlist
→ Play

Series
→ Season / Episode

Live

Search / Dialogue

Discovery / Command-K
```

React should eventually consume stable Nova DTOs such as:

```text
CanonicalMediaSummary
CanonicalMediaDetail
CanonicalSearchResult
PlaybackState
```

Do not perform a wholesale frontend rewrite.

---

# 15. Phase 10 — Live TV and EPG boundary

Maintain existing XMLTV behavior until a different provider is deliberately selected.

Target provider-neutral boundary:

```text
EpgProvider
↓
EPG normalization
↓
ChannelMatcher
↓
CanonicalChannel
↓
ProgramSchedule
```

EPG identity ultimately maps to canonical channels.

---

# 16. Phase 11 — Production hardening

## M11.1 Security/privacy qualification

Audit logs, errors, DTOs, AI inputs, provider boundaries, crash diagnostics, and persistence for sensitive-data leakage.

## M11.2 Migration-chain qualification

Test fresh installs and representative upgrades.

## M11.3 AI failure matrix

Test at minimum:

- AI disabled;
- model missing;
- external library offline;
- corrupt model;
- load error;
- unsupported model;
- transcription cancellation;
- translation cancellation;
- malformed reasoning output;
- semantic backend unavailable;
- low-memory throttling;
- model-version change;
- cache corruption.

## M11.4 Scale qualification

Baseline:

```text
100K SourceRecords
```

Stress:

```text
250K SourceRecords
```

Longer-term soak:

```text
500K SourceRecords
```

Measure:

- import;
- incremental refresh;
- candidate generation;
- snapshot publication;
- search latency;
- semantic indexing;
- memory;
- playback startup;
- variant switching;
- AI contention.

Detect accidental O(N²) behavior.

## M11.5 macOS lifecycle

Qualify:

- launch/quit;
- sleep/wake;
- player teardown;
- stale callbacks;
- external model SSD removal/reconnection;
- active model leases;
- cancellation;
- fresh install;
- upgrade.

## M11.6 Accessibility

Qualify keyboard navigation and accessibility basics for major product flows.

## M11.7 Packaging/signing/notarization

Verify production bundle resources, native dependencies, entitlements, security-scoped access, signing, and notarization workflows.

---

# 19. Implementation status format

Maintain:

`docs/nova-implementation-status.md`

Recommended structure:

```markdown
# Nova Implementation Status

## Baseline
- commit / branch:
- build status:
- test status:
- known blockers:

## Current milestone
- ID:
- objective:
- model used:

## Completed
- ...

## Validation
- command:
- result:

## Migrations introduced
- ...

## Architecture decisions
- ...

## Known gaps
- ...

## Next recommended milestone
- ...
```

Keep this file concise.

It is a handoff document, not a second copy of the architecture specification.

---

# 20. Definition of done for any milestone

A milestone is done only when:

- current implementation was inspected first;
- the solution evolved existing code;
- requested behavior is implemented;
- regression tests exist where appropriate;
- relevant builds/tests/typechecks pass;
- failure/cancellation behavior was checked when relevant;
- secrets are not leaked;
- migrations preserve user state when applicable;
- implementation status is updated;
- future work is not silently started.

---

# 21. Recommended initial execution order

Source-aware audit completed on 2026-09-25. The phase goals remain unchanged;
the dependency adjustments below follow the executable boundaries documented in
`docs/nova-implementation-status.md` (M1 audit). They do not authorize broad
implementation or imply that tested library primitives are integrated features.

```text
0. Stabilize current main
   0.1 repository health audit
   0.2 reproduce/fix bugs
   0.3 baseline qualification

1. Source-aware architecture audit

2. Model infrastructure gaps
   logical model/library domain + additive persistence
   internal registry/discovery using existing installs
   external SSD access / library authorization
   minimum bounded admission + cancellation/lease/resource hooks (from M7)
   runtime manager
   resource policy

3. Canonical catalog gaps
   domain separation
   provider boundary + protected raw capture/staging integration
   persisted fingerprint diff/reuse planning
   blocking
   entity resolution
   identity history
   canonical snapshot validation/publication

4. TMDB / metadata

5. Search

6. Playback

7. Complete jobs / resource governance
   (minimum worker/runtime safety hooks were prerequisites above)

8. Intelligence
   ASR
   translation
   embeddings
   dialogue search
   derived artifacts
   Command-K
   discovery/personalization

9. Frontend canonicalization

10. Live / EPG boundary

11. Production hardening
    security
    migrations
    scale
    failure matrix
    lifecycle
    accessibility
    packaging/signing/notarization

12. Final architecture convergence review
```

Dependency constraints established by the audit:

- **M2.1/M2.2 → internal M2.4 → M2.3:** fixed-path model installers and logical
  model names already exist, but no persisted library registry does. Implement a
  bounded internal registry/discovery slice before removable-library access. Reuse
  current adapters; discovery must not implicitly download or load models.
- **Minimum M7.1/M7.2/M7.3/M7.4/M7.5 contracts precede runtime admission and new
  background workers:** the queue library is not used by desktop workers, and
  session cancellation flags do not join running inference. Establish bounded
  admission, cancellation ownership, leases/recovery expectations and priority
  hooks with M2.5/M2.6 rather than deferring every governance concern to Phase 7.
  Full scheduling policy and qualification can remain in that later phase.
- **Raw capture is an input; canonical publication is an output:** protected files,
  scope receipts, fingerprints and record staging already exist independently of
  the app. Integrate capture with M3.2 before diffing/resolution. M3.7's final
  canonical-generation publication still follows identity/history validation.
  Do not rebuild these primitives or call a raw receipt a serving generation.
- **Optional models are not a dependency of deterministic M3 or TMDB M4:** the
  current enrichment entry point accidentally gates TMDB on embedding installation.
  Preserve independent deterministic behavior; model-assisted evidence is optional.
- **Minimum M9 contracts accompany each canonical slice:** legacy DTOs expose
  provider URLs and React currently invents page-local variant groups. Add
  secret-free canonical read/action contracts with the first movie vertical slice;
  do not wait for all Phase 8 intelligence work to verify an end-to-end cutover.
  Broad UI changes remain in Phase 9.
- **M11.1/M11.2 are continuous gates:** secret redaction, profile/operation identity,
  upgrade preservation and old-binary compatibility must be tested with each new
  IPC/schema slice. Final production qualification remains in Phase 11.

The next architecture slice is M2.1 plus minimal M2.2/internal M2.4, with acceptance
criteria in the status document. Audit-discovered current-main data/privacy risks
need focused qualification before new integration. No milestone implementation
was performed as part of this ordering update.

---

# 22. Final target

Nova should evolve from an IPTV application with AI features into a local media intelligence platform.

It should understand:

- what each media item is;
- how provider sources relate;
- which playable variants exist;
- which variant is appropriate;
- structured metadata and people;
- what is spoken;
- where dialogue occurs;
- what the user is searching for;
- what the user tends to enjoy.

That intelligence must emerge throughout the product without compromising deterministic catalog correctness, playback reliability, privacy, or graceful operation when optional AI functionality is unavailable.
