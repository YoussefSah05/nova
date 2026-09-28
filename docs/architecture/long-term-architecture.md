# Nova — Long-Term Architecture and Engineering Process

> **Purpose**
>
> This document describes where Nova's architecture should converge, and the process used to get there: rebuild accurate context from the actual repository, reconcile the codebase against the intended architecture, detect milestones that are already complete/partial/superseded, identify anything that was skipped, and then continue implementation from the first real dependency gap.
>
> **Important:** the repository is the source of truth for what exists today. This document is the source of truth for where Nova should converge.

---

# 0. Working principles

Do **not** assume the roadmap has been executed in order.

Some milestones may already be:

- complete;
- partially complete;
- implemented under different names;
- implemented more strongly than expected;
- started but not integrated;
- superseded by later work;
- duplicated by an accidental parallel subsystem.

The first task of any milestone is therefore **reconciliation**, not blind implementation.

Before broad changes:

1. read `CONTRIBUTING.md`, `docs/archive/master-plan.md` (historical), and `docs/nova-implementation-status.md`;
2. inspect recent git history for architectural work already completed;
3. locate relevant dependencies with code search and the crate map;
4. trace their callers and dependents;
5. inspect actual Rust/Tauri/React/SQLite source, tests, migrations, DTOs, and runtime contracts;
6. build a milestone reconciliation matrix;
7. identify the first real missing/partial/risky dependency;
8. only then begin implementation.

Do not restart completed milestones.

Do not create parallel replacements such as:

- `catalog-v2`;
- `search-v2`;
- `tmdb-v2`;
- `playback-v2`;
- `ai-v2`;
- `jobs-v2`.

The goal is **convergence**, not rewrite.

---

# 1. Product vision

Nova is a production-grade macOS IPTV and personal-media application that should feel closer to a modern streaming service than a playlist viewer.

Nova should understand:

- what each media item is;
- how provider sources relate;
- which sources are duplicates;
- which playable variants exist;
- which variant is best;
- what the content is called in a clean user-facing way;
- how it should be classified;
- who appears in it;
- what is spoken;
- where dialogue occurs;
- what the user is searching for;
- what the user tends to enjoy;
- which intelligence capabilities are currently available locally.

The product should hide provider disorder behind a clean canonical catalog.

---

# 2. Technology and ownership

Nova evolves around:

- Rust;
- Tauri;
- React;
- TypeScript;
- SQLite;
- libmpv;
- FFmpeg where appropriate;
- VideoToolbox where appropriate;
- local AI inference;
- local ASR;
- local translation;
- local embeddings.

## Rust owns durable truth

Rust owns:

- catalog identity;
- persistence;
- provider integration;
- catalog compilation;
- metadata integration;
- playback coordination;
- search/indexing;
- jobs;
- model management;
- media processing;
- resource governance;
- AI-derived artifact provenance;
- security-sensitive state.

## React owns presentation

React owns:

- presentation;
- navigation;
- interaction;
- transient UI state.

React must not own authoritative:

- canonical media identity;
- provider state;
- playback state machine;
- credentials;
- authenticated URLs;
- durable persistence;
- migration logic.

---

# 3. Canonical media architecture

The central domain distinction is:

```text
SourceRecord != CanonicalEntity != StreamVariant
```

## SourceRecord

Represents provider input.

Examples:

- M3U entry;
- Xtream movie;
- Xtream series;
- provider episode;
- live channel;
- provider metadata;
- provider category/group.

A SourceRecord preserves raw source evidence and provider provenance.

It is not durable user-facing identity.

## CanonicalEntity

Represents Nova-owned durable media identity.

Examples:

- Movie;
- Series;
- Season;
- Episode;
- Live Channel.

Durable user state belongs here:

- favorites;
- watchlist;
- playback history;
- resume position;
- completion;
- canonical search relationships;
- personalization signals.

## StreamVariant

Represents one playable provider source for a canonical entity.

Examples:

- Provider A 1080p;
- Provider B 4K;
- HEVC;
- H.264 fallback;
- HDR;
- SDR;
- French audio;
- English audio.

A CanonicalEntity can have multiple StreamVariants.

---

# 4. Provider boundary

Provider-specific structures must stop at ingestion/catalog compilation.

Target:

```text
Provider
   ↓
provider-specific record
   ↓
SourceRecord
   ↓
Catalog Compiler
   ↓
CanonicalEntity
+
StreamVariant
```

Provider-specific structures should not leak into:

- canonical search;
- favorites/history;
- metadata ownership;
- React DTOs;
- playback UI;
- canonical relationships.

Provider categories/groups are provider organization and provenance.

They are not automatically Nova taxonomy.

Credentials, cookies, temporary URLs, signed URLs, authenticated Xtream URLs, and similar secrets remain Rust-side.

---

# 5. Production data-cleaning pipeline

The data-cleaning pipeline is a core product subsystem.

Target:

```text
Raw provider record
      ↓
Immutable SourceRecord + provenance
      ↓
Deterministic normalization / parsing
      ↓
Structured source evidence
      ↓
Versioned fingerprint
      ↓
Media classification
      ↓
Candidate blocking
      ↓
Deterministic entity resolution
      ↓
Optional bounded ambiguity reasoning
      ↓
CanonicalEntity
      +
StreamVariant(s)
      ↓
TMDB identification
      ↓
Canonical production naming
      ↓
Structured metadata / taxonomy
      ↓
Search aliases / indexes
      ↓
Validation
      ↓
Atomic catalog publication
```

Users should never have to understand IPTV/provider naming conventions.

---

# 6. Naming model

Do not collapse all names into one destructive field.

Conceptually distinguish:

```text
raw_title
normalized_title
parsed_title
default_title
original_title
localized_titles
alternate_titles
provider_aliases
```

## raw_title

Exact provider value.

Example:

```text
FR | ★ OPPENHEIMER [4K] MULTI (2023)
```

Never destroy this evidence.

## normalized_title

Machine-comparison form.

Example:

```text
oppenheimer
```

Used for matching, blocking, search aliases, and derived logic.

It is not automatically the user-facing title.

## parsed_title

Best deterministic title extracted from provider input.

Example:

```text
Oppenheimer
```

Used as matching evidence and unresolved fallback.

## default_title

Trusted structured Nova-facing title after confident identification.

For confidently identified movies/TV/episodes, this will normally come from TMDB.

## original_title

Original production title from structured metadata.

## localized_titles

Presentation titles keyed by locale.

Localized presentation must not change identity.

## alternate_titles

Trusted alternate names, translations, aliases.

## provider_aliases

Raw/cleaned provider forms retained for search and diagnostics.

---

# 7. Deterministic title parsing

Provider-name parsing should remain deterministic Rust logic where possible.

Typical noise:

- `FR |`, `UK |`, `US |`;
- `[4K]`, `FHD`, `1080p`, `720p`;
- codec markers;
- `MULTI`, `VF`, `VOSTFR`;
- language markers;
- years;
- `S01E04`;
- `Season 2 Episode 7`;
- provider branding;
- punctuation/separators;
- bracketed release information;
- quality suffixes.

Target structured output:

```text
raw_title
parsed_title
normalized_title
media_type_hint
year_hint
season_hint
episode_hint
language_hint
quality_hint
codec_hint
provider provenance
```

Parser behavior should be:

- deterministic;
- conservative;
- non-destructive;
- tested;
- versionable when parser changes affect derived artifacts.

Do not let quality/language/provider noise become the canonical display title.

---

# 8. Media classification

Nova should classify records into:

- Movie;
- Series;
- Season;
- Episode;
- Live Channel;
- Unknown / unresolved.

Use deterministic evidence first:

- provider endpoint/type;
- season/episode coordinates;
- trusted external IDs;
- title structure;
- source metadata;
- known series relationships.

Provider folders/categories alone must not become authoritative media type.

Unknown is valid.

Do not force ambiguous records into confident classifications.

---

# 9. Fingerprinting and incremental refresh

Each SourceRecord should have a versioned fingerprint.

Refresh should classify records approximately as:

```text
unchanged
changed
new
deleted
```

Unchanged records should reuse expensive work when safe:

- TMDB match;
- artwork;
- reasoning artifacts;
- embeddings;
- transcripts;
- translations;
- summaries/derived artifacts.

Parser/fingerprint semantic changes should be version-aware.

Do not recompute intelligence on every provider refresh.

---

# 10. Candidate blocking

Entity resolution must not compare every record with every other record.

No whole-library O(N²).

Blocking evidence may include:

- media type;
- normalized title;
- title fingerprints;
- year;
- trusted external IDs;
- season/episode coordinates;
- language;
- series relationship.

Only plausible groups proceed to deeper comparison.

---

# 11. Entity resolution

Entity resolution should prefer deterministic evidence.

Useful evidence:

- media type;
- normalized title;
- parsed/original title;
- year;
- trusted external IDs;
- season/episode coordinates;
- duration;
- provider hints;
- aliases;
- language.

Hard contradictions stop automatic merge:

- movie vs series;
- conflicting trusted external IDs;
- incompatible episode coordinates;
- clearly incompatible years.

Conservative behavior is preferred.

Two temporary duplicates are better than one incorrect merge.

---

# 12. Optional ambiguity intelligence

AI is not the first step.

Target:

```text
deterministic parsing
      ↓
candidate blocking
      ↓
deterministic scoring / contradictions
      ↓
still ambiguous?
      ↓
bounded real candidate set
      ↓
optional intelligent decision
      ↓
structured proposal + confidence
      ↓
Nova validation
      ↓
accept OR unresolved
```

A typed decision model may be evaluated for bounded tasks such as:

- choose real candidate or `none`;
- classify Movie / Series / Episode / Live / Unknown;
- Accept / NeedsMoreEvidence / Reject;
- Compatible / Contradictory / Uncertain.

A local LLM may be used for harder bounded ambiguity.

Neither a typed classifier nor an LLM may:

- invent TMDB IDs;
- invent canonical identity;
- invent authoritative names;
- override hard contradictions;
- mutate durable identity directly;
- become required for catalog compilation.

Rule:

```text
AI proposes
Nova validates
```

Remote decision services must remain optional and explicitly evaluated for privacy, cost, latency, and offline behavior.

---

# 13. Canonical identity history

Canonical identity must survive provider changes.

Support:

- aliases;
- redirects;
- merge history;
- split history where needed.

Historical references must deterministically resolve to the current identity.

Favorites, watchlist, history, completion, and resume must survive identity correction.

---

# 14. TMDB strategy

TMDB is Nova's primary structured knowledge source for movies and TV.

Use it for:

- titles;
- original titles;
- alternate titles;
- translations/localized titles;
- overview;
- tagline;
- release date;
- runtime;
- genres;
- keywords;
- companies;
- countries;
- languages;
- cast;
- crew;
- external IDs;
- certifications;
- posters;
- backdrops;
- logos;
- videos;
- collections;
- recommendations;
- similar titles;
- creators;
- networks;
- aggregate credits;
- seasons;
- episodes;
- content ratings.

Provider metadata is mainly for:

- source identification;
- provenance;
- playback.

Invariant:

```text
Nova ID != TMDB ID
```

Correct:

```text
Provider
→ SourceRecord
→ CanonicalEntity
→ TMDB match
→ structured knowledge
```

---

# 15. TMDB identification

Preferred flow:

```text
trusted external ID?
      │
     yes
      ↓
TMDB Find

otherwise

parsed / normalized metadata
      ↓
movie / TV search
      ↓
real candidates
      ↓
deterministic scoring
      ↓
optional bounded ambiguity decision
      ↓
confident match OR unresolved
```

Persist match provenance:

- canonical entity;
- TMDB ID;
- media type;
- method;
- confidence;
- evidence;
- retrieval timestamp/version.

Low-confidence matches remain unresolved.

AI must never invent a TMDB ID.

---

# 16. Production display-name policy

For confidently identified movies/series/seasons/episodes:

```text
CanonicalEntity
      ↓
confident TMDB match
      ↓
best structured title for UI locale
      ↓
Nova display title
```

Retain separately:

- Nova canonical ID;
- TMDB ID;
- default title;
- original title;
- localized titles;
- alternate titles;
- raw provider names;
- parsed provider names;
- normalized aliases.

For unresolved entities:

1. use deterministic parsed title when safe;
2. fall back to a cleaned provider title;
3. preserve raw title separately;
4. do not fabricate metadata.

Never silently store an AI-generated title as authoritative metadata.

---

# 17. Localization

A canonical entity may expose:

```text
default_title
original_title
localized_titles[locale]
alternate_titles[]
```

UI title policy:

```text
preferred locale title
      ↓ fallback
default structured title
      ↓ fallback
parsed provider title
      ↓ fallback
raw provider title
```

Search should index:

- localized title;
- default title;
- original title;
- alternate titles;
- cleaned provider aliases.

---

# 18. Nova taxonomy

TMDB supplies structured knowledge.

Nova may maintain product-specific taxonomy where it adds value.

Provider groups are not authoritative taxonomy.

Useful relationships:

- media type;
- genres;
- keywords;
- people;
- collections;
- language;
- country;
- certification;
- company/network;
- series/season/episode hierarchy;
- smart collections;
- local personalization signals.

Factual metadata and AI-derived classifications must remain distinguishable.

---

# 19. Progressive enrichment

Do not block the whole catalog downloading everything.

## Tier 0 — Identification

Enough to know what content is.

## Tier 1 — Core product metadata

Enough for:

- browse cards;
- detail pages;
- search;
- core artwork.

## Tier 2 — Rich metadata

Credits, keywords, collections, companies, richer artwork.

## Tier 3 — Deep lazy metadata

Complete people, deep season data, episode artwork, full collections.

Metadata belongs to CanonicalEntities, not provider duplicates.

---

# 20. Catalog snapshots

Users must never browse a partially rebuilt catalog.

Target:

```text
active snapshot
      ↓
compile new snapshot
      ↓
validate
      ↓
atomic publication
```

Failure leaves the previous active snapshot usable.

---

# 21. Provenance

Important derived information should record provenance.

Useful fields:

- source;
- source ID;
- retrieval timestamp;
- model ID;
- model version;
- parser/derivation version;
- input fingerprint;
- confidence.

Especially for:

- TMDB matches;
- catalog reasoning;
- transcripts;
- translations;
- embeddings;
- summaries;
- smart collections;
- recommendations.

---

# 22. Search architecture

Search returns CanonicalEntities.

Possible channels:

```text
Exact title
FTS
Metadata
People
Genres / keywords
Transcript
Semantic entity embeddings
Semantic transcript embeddings
```

Flow:

```text
query
 │
 ├─ exact / lexical
 ├─ metadata
 ├─ transcript
 └─ semantic
      ↓
     fusion
      ↓
canonical results
```

Exact-title reliability remains essential.

Semantic search is additive.

Deterministic search continues when semantic/AI infrastructure is unavailable.

---

# 23. Dialogue and scene search

Stable transcripts become searchable knowledge.

Example:

```text
"the scene where they explain the simulation"
```

Result:

```text
Movie X
01:12:34
```

Selecting the result uses normal playback and seeks to the timestamp.

Searchable transcript intelligence may support:

- remembered dialogue;
- concepts;
- characters;
- themes;
- scene descriptions;
- important moments.

Do not hallucinate timestamps absent from transcript evidence.

---

# 24. Playback architecture

Playback truth belongs in Rust.

Target:

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

## PlaybackCoordinator owns

- play;
- stop;
- pause/resume;
- seek;
- playback session;
- canonical entity;
- selected variant;
- resume;
- lifecycle;
- end-of-media;
- next episode;
- live/VOD distinction;
- errors;
- recovery.

React receives state.

React does not own playback truth.

## StreamResolver

Handles:

- provider auth;
- URL construction;
- headers;
- cookies;
- temporary URLs;
- proxy/session information.

Secrets remain Rust-side.

## VariantSelector

Deterministically ranks variants based on:

- preferred language;
- resolution;
- HDR/SDR;
- codec;
- hardware support;
- user preference;
- current network;
- historical QoE;
- provider availability;
- recent errors.

No LLM in playback critical path.

---

# 25. Playback recovery

Recovery is finite.

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

No infinite retry loops.

Old-session callbacks may never mutate newer sessions.

---

# 26. Playback QoE

Store safe local evidence:

- startup latency;
- buffering;
- premature termination;
- decoder failure;
- network failure;
- retry count;
- variant used;
- timestamp.

QoE can improve future deterministic variant selection.

No remote telemetry is required.

Never persist secret URLs/credentials in QoE.

---

# 27. Series

Canonical hierarchy:

```text
Series
  ↓
Season
  ↓
Episode
```

Support:

- lazy season loading;
- lazy episode enrichment;
- episode resume;
- autoplay next;
- canonical episode identity;
- multiple episode variants.

Autoplay requests the next canonical episode through the normal playback path.

---

# 28. Live TV and EPG

Live TV is not VOD.

Important concepts:

- canonical channel;
- live variants;
- fast switching;
- EPG;
- now/next;
- live edge;
- unreliable streams;
- channel bundles.

Provider-neutral EPG boundary:

```text
EpgProvider
   ↓
normalization
   ↓
ChannelMatcher
   ↓
CanonicalChannel
   ↓
ProgramSchedule
```

Preserve existing XMLTV until another provider is deliberately selected.

---

# 29. Jobs

Background work includes:

- catalog import;
- catalog refresh;
- TMDB enrichment;
- artwork;
- embeddings;
- transcription;
- translation;
- indexing;
- maintenance.

Jobs should be:

- bounded;
- cancellable;
- prioritized;
- observable;
- persistent/resumable where useful.

Avoid:

```text
100,000 records
→ 100,000 simultaneous tasks
```

Prefer:

```text
bounded queue
→ controlled workers
→ backpressure
```

---

# 30. Global resource governor

Priority:

```text
1. Playback
2. Active real-time transcription / translation
3. Interactive search / navigation
4. Explicit user import / refresh
5. Metadata enrichment
6. Background AI / indexing
7. Maintenance
```

Integrate existing:

- playback activity;
- job scheduler;
- model runtime leases;
- model loading/unloading;
- cancellation;
- concurrency limits.

Do not create a second scheduler if one already exists.

No LLM scheduling decisions.

Playback pressure may throttle/pause/cancel nonessential background intelligence.

---

# 31. AI / intelligence architecture

AI is first-class but optional for correctness.

Target:

```text
IntelligenceService
        │
        ▼
ModelRuntimeManager
        │
 ┌──────┼────────────┬──────────────┬─────────────┬─────────────┐
 ▼      ▼            ▼              ▼             ▼             ▼
LLM     ASR      Translation     Embeddings     Reranker     Classifier
```

Use specialized models for specialized capabilities.

Do not force everything through one general LLM.

If all AI is unavailable:

- Nova still launches;
- catalog opens;
- playback works;
- exact/FTS search works;
- durable user state remains correct.

---

# 32. Model storage architecture

Invariant:

```text
Model ID != filesystem path
```

Conceptual structures:

```text
ModelDescriptor
ModelLibrary
ModelLocation
```

## ModelDescriptor

May include:

- logical ID;
- family;
- capability;
- backend;
- format;
- quantization;
- context limit;
- memory class;
- version;
- license;
- checksum.

## ModelLibrary

May include:

- library ID;
- display name;
- internal/external;
- storage authorization;
- status;
- last seen.

## ModelLocation

Maps:

```text
model ID
+ library ID
+ relative path
```

Do not use absolute `/Volumes/...` paths as permanent model identity.

---

# 33. External model libraries

Expected:

```text
SSD online
→ model available

SSD offline
→ model library offline
→ dependent AI feature unavailable
→ Nova continues working
```

Disconnecting must not:

- crash Nova;
- corrupt catalog;
- delete configuration;
- break playback;
- destroy user state.

Reconnection restores availability.

Use proper macOS persistent security-scoped storage access when required.

---

# 34. Model runtime manager

Should provide:

- logical model resolution;
- lazy loading;
- explicit unload;
- model leases/lifetimes;
- backend adapters;
- availability;
- cancellation;
- resource awareness.

Critical invariant:

```text
active inference lease
→ model cannot unload
```

Do not assume all heavyweight models remain resident.

---

# 35. AI model-role policy

Use capability roles, not hardcoded brands.

## Small/fast reasoning model

Use for:

- Command-K parsing;
- typed intent;
- lightweight catalog reasoning;
- lightweight classification.

## Larger reasoning model

Use only for harder bounded ambiguity.

Load on demand.

## ASR model

Dedicated speech-recognition model.

Optimize for:

- timestamp accuracy;
- multilingual quality;
- latency;
- memory.

## Translation model

Dedicated translation model.

May use pair-specific fast models plus broader fallback.

## Embedding model

Dedicated semantic retrieval model.

Version embeddings by model/version.

## Optional reranker

Use only if search evaluation shows meaningful improvement.

## Optional typed classifier

May be evaluated for bounded classification/decision tasks.

Must remain optional and replaceable.

---

# 36. Model replacement policy

Do not change production models because something newer appears.

Required process:

```text
current model
    ↓
benchmark
    ↓
candidate model
    ↓
same benchmark
    ↓
compare
    ↓
change default only if evidence supports it
```

Compare:

- quality;
- latency;
- memory;
- model load time;
- playback impact;
- multilingual behavior;
- license;
- distribution practicality;
- storage size;
- energy/thermal behavior where meaningful.

---

# 37. Transcription

Target:

```text
Playback audio
      ↓
audio extraction/tap
      ↓
bounded PCM buffering
      ↓
VAD/chunking
      ↓
ASR
 ┌────┴────┐
partial   stable timestamped
             ↓
        transcript storage
             ↓
         FTS + embeddings
```

Partial output is ephemeral.

Stable segments become durable.

Changing/stopping media cancels obsolete transcription.

ASR failure does not break playback.

---

# 38. Translation

Target:

```text
stable timed source segment
      ↓
TranslationEngine
      ↓
translated timed segment
      ↓
cache
      ↓
subtitle renderer
```

Cache identity includes:

- source segment/hash;
- source language;
- target language;
- model;
- model version;
- decoding options.

Do not repeatedly translate identical stable segments.

Translation failure must not break source captions or playback.

---

# 39. Embeddings

Primary embedding owners:

- CanonicalEntities;
- transcript segments.

Do not generate redundant embeddings for provider duplicates.

Embeddings are versioned by model/version.

Model change triggers safe reindexing.

Exact/FTS search continues during semantic unavailability/reindex.

---

# 40. Natural-language Command-K

Architecture:

```text
user request
      ↓
local LLM
      ↓
typed Nova intent
      ↓
deterministic validator
      ↓
Nova application API
```

Initial intent families may include:

- Search;
- Filtered Discovery;
- Navigate;
- Continue Media;
- Play Known Entity;
- Caption/Translation Settings.

The model never receives unrestricted application authority.

Ambiguity returns a typed ambiguity state instead of guessing.

---

# 41. Smart discovery

May combine:

- TMDB metadata;
- entity embeddings;
- transcript embeddings;
- local catalog;
- history;
- favorites;
- completion;
- people;
- genres;
- keywords;
- collections.

Examples:

```text
slow-burn science fiction
rainy-night thrillers
comfort sitcom episodes
things like Arrival
```

The model interprets intent.

Nova retrieves actual local catalog items.

Never hallucinate unavailable content.

---

# 42. Local personalization

Keep personalization local.

Possible signals:

- watch history;
- favorites;
- watchlist;
- completed content;
- repeat viewing;
- genre/person affinity;
- semantic similarity;
- preferred languages;
- unfinished/finished tendencies.

Do not require remote behavioral tracking.

---

# 43. Derived content understanding

Potential artifacts:

- summaries;
- themes;
- important moments;
- scene descriptors;
- chapter suggestions;
- character/subject mentions;
- topic tags;
- transcript summaries.

Derived artifacts must be versioned and distinguishable from factual structured metadata.

Use existing provenance/artifact infrastructure if present.

Do not build a parallel artifact system without need.

---

# 44. AI jobs

AI work should use the same bounded job architecture.

Possible jobs:

- transcription;
- translation;
- embedding generation;
- re-embedding;
- semantic indexing;
- summaries;
- content understanding;
- catalog ambiguity reasoning;
- model verification/download if supported.

AI jobs should support:

- priority;
- cancellation;
- retries where appropriate;
- observability;
- persistence/resume where useful;
- resource-governor integration.

No unbounded per-record task spawning.

---

# 45. AI failure behavior

Audit and implement capability-level failure handling.

Cases:

## Model missing

Feature unavailable.

## External model SSD offline

Dependent feature unavailable until reconnect.

Configuration remains intact.

## Corrupt model

Mark unusable; do not crash.

## Load error

Expose typed safe diagnostic state.

## Insufficient memory

Resource governor denies/throttles lower-priority work before playback suffers.

## Semantic backend unavailable

Fallback to deterministic search.

## ASR failure

Playback continues.

## Translation failure

Source captions continue.

## Malformed LLM output

Typed validation rejects it.

## AI disabled

All deterministic functionality remains available.

---

# 46. AI settings UX

Eventually provide coherent settings for:

- model libraries;
- internal/external storage;
- model availability;
- preferred model per capability where supported;
- loading/unloading policy;
- transcription defaults;
- translation defaults;
- semantic search enable/disable;
- optional AI features;
- storage usage;
- diagnostics.

Do not expose security-scoped authorization tokens.

---

# 47. AI privacy

Users should be able to understand whether a capability is:

```text
Local
```

or:

```text
Remote / optional external service
```

Remote decision/classification services must be optional.

Local-only operation remains supported.

Provider credentials and authenticated playback URLs must never be sent to external AI services.

---

# 48. Search and AI evaluation

Each AI capability needs measurable acceptance criteria.

## ASR

Measure:

- transcription quality;
- timestamp quality;
- latency;
- cancellation;
- memory;
- multilingual behavior.

## Translation

Measure:

- quality;
- latency;
- cache behavior;
- language coverage;
- timing preservation.

## Embeddings/search

Measure:

- relevance;
- recall;
- exact-search preservation;
- latency;
- reindex cost;
- memory/storage.

## Command-K

Measure:

- intent accuracy;
- malformed-output rejection;
- ambiguity handling;
- authorization correctness;
- no hallucinated entity IDs.

## Smart discovery

Measure:

- all results exist in catalog;
- intent relevance;
- deterministic filters respected;
- fallback behavior.

## Catalog ambiguity reasoning

Measure:

- false merge rate;
- abstention quality;
- candidate selection accuracy;
- hard-contradiction compliance.

---

# 49. Frontend architecture

React should consume stable Nova DTOs such as:

```text
CanonicalMediaSummary
CanonicalMediaDetail
CanonicalSearchResult
PlaybackState
```

React should not depend directly on:

```text
XtreamMovie
M3UItem
raw provider category IDs
provider credentials
authenticated URLs
```

Migrate one vertical slice at a time.

Preferred:

```text
Movie Browse
→ Detail
→ Favorite / Watchlist
→ Play

Series
→ Season / Episode

Live

Search / Dialogue

Discovery / Command-K
```

Do not wholesale rewrite the frontend.

---

# 50. Nova UX principles

The UI should express canonical intelligence, not provider disorder.

## Clean presentation

Cards/detail pages should show:

- clean localized title;
- artwork;
- year;
- genres;
- runtime where known;
- meaningful metadata;
- progress/resume;
- completion;
- availability/quality indicators only when useful.

Do not show `FR |`, `[4K]`, `MULTI`, provider folders, or raw provider IDs as the primary title.

## Hide provider complexity

Normally show one canonical item, not one card per provider.

Advanced source/quality controls may expose variants intentionally.

## Honest unresolved state

If identity is unresolved:

- use deterministic cleaned title;
- do not fabricate metadata;
- avoid incorrect merge;
- allow later correction;
- keep aliases searchable.

## Progressive enrichment

Browse must remain usable before deep metadata completes.

Prefer progressive loading/skeletons over blocking the library.

## Search-first usability

Support:

- exact title;
- aliases;
- people;
- genres;
- keywords;
- transcript search;
- natural-language semantic discovery when available;
- direct playback from transcript timestamps.

## Playback-first responsiveness

Playback outranks background intelligence.

The UI must not become sluggish because embeddings/enrichment are running.

## Resume / Continue Watching

Resume belongs to canonical identity.

Provider refresh or variant switching must not lose progress.

## Series UX

Show:

- canonical seasons;
- real episode titles when known;
- progress;
- resume;
- next episode;
- autoplay;
- artwork where available.

## Live UX

Show:

- canonical channels;
- now/next;
- EPG context;
- fast switching;
- safe fallback/recovery.

## AI capability states

If a model/library is unavailable:

- browsing works;
- playback works;
- exact search works;
- user state works.

UI should show capability-specific states, not generic failure.

Examples:

```text
Semantic search unavailable — exact search still works.
Translation model library is offline.
Transcription model unavailable.
```

## Diagnostics UX

Nova should safely explain:

- why records merged;
- why they stayed separate;
- why TMDB match was chosen;
- why a stream variant was selected;
- why playback fell back;
- why a model is unavailable;
- why an AI artifact regenerated.

Diagnostics must not leak secrets.

## Native macOS polish

Target:

- keyboard navigation;
- good focus behavior;
- Command-K;
- accessibility labels;
- native lifecycle behavior;
- sleep/wake resilience;
- security-scoped storage;
- signing/notarization readiness.

---

# 51. Suggested product surfaces

Do not rebuild navigation just to match this list if the existing UX is already better, but Nova should support clean paths for:

- Home / Continue Watching;
- Movies;
- Series;
- Live;
- Search;
- Watchlist / Favorites;
- Intelligent Discovery;
- Settings;
- Models / Storage;
- Playback;
- diagnostics where useful.

---

# 52. Error-state UX

Errors should be actionable and scoped.

## Provider/catalog failure

Explain whether refresh failed and whether the existing catalog remains usable.

## Playback failure

Explain whether:

- current source failed;
- fallback was attempted;
- no viable source remains.

Do not expose secret URLs.

## AI/model failure

Explain which optional capability is unavailable and what still works.

## Metadata failure

Keep the canonical item usable even if enrichment failed.

---

# 53. Security and privacy

Sensitive information must not leak through:

- logs;
- React;
- AI prompts;
- crash reports;
- analytics;
- diagnostics.

Sensitive data includes:

- provider credentials;
- authenticated Xtream URLs;
- signed stream URLs;
- API tokens;
- model-storage authorization;
- private profile information.

Local AI is the default philosophy.

Remote optional services remain bounded and non-essential.

---

# 54. Diagnostics and observability

Important decisions should retain safe structured evidence.

Useful diagnostics:

- merge evidence;
- contradiction evidence;
- TMDB match evidence;
- variant scoring;
- playback failure classification;
- model availability;
- artifact invalidation reason;
- job/resource priority.

Diagnostics should be testable and secret-safe.

---

# 55. Persistence and migrations

Use additive deterministic migrations.

Preferred:

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

Test:

- fresh DB;
- representative old DB;
- full migration chain;
- identity/user-state preservation;
- integrity;
- realistic scale where relevant.

---

# 56. Testing

## Unit tests

Examples:

- title normalization;
- provider-name parsing;
- media classification;
- fingerprints;
- candidate blocking;
- contradiction handling;
- variant scoring;
- model registry;
- resource policy;
- search fusion;
- translation cache keys.

## Integration tests

Examples:

- SQLite;
- migrations;
- provider import;
- catalog compiler;
- naming/classification;
- TMDB;
- search;
- EPG;
- playback coordinator;
- jobs;
- model libraries.

## End-to-end

Examples:

- import/refresh;
- identity preservation;
- favorites/watchlist;
- resume;
- autoplay;
- ASR;
- translation;
- semantic search;
- dialogue search;
- provider failure;
- bad streams;
- failover;
- sleep/wake;
- external model SSD disconnect;
- fresh install;
- accessibility;
- signing/notarization.

---

# 57. Scale targets

Baseline:

```text
100K SourceRecords
```

Stress:

```text
250K
```

Longer-term soak:

```text
500K
```

Measure:

- import time;
- incremental refresh;
- normalization/parsing cost;
- candidate generation;
- entity resolution;
- snapshot publication;
- search latency;
- semantic indexing;
- memory;
- playback startup;
- variant switching;
- AI contention.

No unbounded O(N²).

---

# 58. AI failure matrix

Test at minimum:

- AI disabled;
- model missing;
- external model library offline;
- corrupt model;
- model load error;
- unsupported model;
- transcription cancellation;
- translation cancellation;
- malformed reasoning output;
- semantic backend unavailable;
- low-memory throttling;
- model version change;
- cache corruption.

Invariant:

```text
AI failure may reduce intelligent functionality
but must not corrupt canonical identity
or prevent normal browsing, exact search, or playback.
```

---

# 59. Milestone reconciliation

Before implementation, classify each milestone:

- COMPLETE;
- PARTIAL;
- MISSING;
- SUPERSEDED;
- RISKY / NEEDS REWORK.

Do not infer status from filenames alone.

Verify code/tests/migrations/runtime contracts.

## Core architecture

```text
M00 Current-main stabilization
M01 Source-aware architecture audit
M02 Model library domain/persistence
M03 External model storage
M04 Model runtime manager / leases
M05 Resource governor
M06 Canonical domain / provider boundary
M07 Naming / normalization / classification pipeline
M08 Fingerprints / candidate blocking
M09 Entity resolution / identity history
M10 Atomic catalog snapshots
M11 TMDB identification / progressive enrichment
M12 Canonical production naming / localization
M13 Canonical search
M14 PlaybackCoordinator / StreamResolver / VariantSelector
M15 Playback recovery / QoE
M16 Jobs / bounded queues / backpressure
M17 Streaming transcription
M18 Timed translation
M19 Embeddings / dialogue search
M20 Command-K typed intents
M21 Smart discovery / personalization
M22 Frontend canonicalization
M23 Series / autoplay
M24 Live / EPG boundary
M25 Security / privacy hardening
M26 Migration-chain qualification
M27 Scale qualification
M28 AI failure matrix
M29 macOS lifecycle qualification
M30 Accessibility / release / signing / notarization
M31 Final architecture convergence review
```

## Explicit AI capability milestones

```text
AI01 Model domain / libraries
AI02 External/removable model storage
AI03 Model registry/discovery
AI04 Runtime manager / leases
AI05 Resource-governor integration
AI06 ASR baseline
AI07 Streaming transcription
AI08 Translation baseline
AI09 Real-time translation
AI10 Entity embeddings
AI11 Transcript embeddings
AI12 Semantic search
AI13 Dialogue search
AI14 Optional reranker
AI15 Catalog ambiguity reasoning
AI16 Command-K typed intents
AI17 Smart discovery
AI18 Local personalization
AI19 Derived content-understanding artifacts
AI20 AI settings UX
AI21 AI failure UX
AI22 AI benchmark/evaluation harness
AI23 AI provenance/versioning
AI24 AI privacy/offline qualification
```

Some rows overlap intentionally with core milestones.

Do not duplicate implementations to satisfy names.

---

# 60. Reconciliation questions

Explicitly answer:

1. Which milestones are already fully implemented?
2. Which are partial?
3. Which are implemented under different names?
4. Which were skipped accidentally?
5. Which have both legacy and new implementations coexisting?
6. Which provider-specific types still leak into canonical/frontend layers?
7. Which title fields currently drive user-facing naming?
8. Does TMDB naming/localization work correctly?
9. Are raw/parsed/normalized/default/localized titles distinct enough?
10. Are aliases searchable?
11. Are canonical IDs independent from provider/TMDB IDs?
12. Are user-state migrations safe?
13. Does exact search work without AI?
14. Does playback truth live in Rust?
15. Are stale playback callbacks prevented?
16. Are job queues bounded?
17. Can AI jobs interfere with playback?
18. Which AI capabilities bypass ModelRuntimeManager?
19. Which model paths are hardcoded?
20. Which AI outputs lack provenance/versioning?
21. Which inference paths are not cancellable?
22. Which models remain resident unnecessarily?
23. Which AI feature states are missing in the UI?
24. Are provider credentials/authenticated URLs ever passed to AI?
25. Can smart discovery return media absent from the catalog?
26. Are transcript timestamps correctly linked to playback?
27. Do unresolved catalog items degrade honestly?
28. Are there accidental `*-v2` parallel systems?
29. Are frontend flows still provider-centric?
30. What is the first dependency-correct gap to fix?

---

# 61. Reconciliation deliverable

Before implementation, update/create:

`docs/nova-implementation-status.md`

Recommended structure:

```markdown
# Nova Implementation Status

## Repository baseline
- branch / commit:
- build:
- tests:
- known blockers:

## Milestone reconciliation
| Milestone | Status | Evidence | Remaining gap |
|---|---|---|---|

## Architecture already implemented
- ...

## Partial systems to evolve
- ...

## Missing systems
- ...

## Risky legacy coupling
- ...

## AI capability status
- ...

## UX/frontend status
- ...

## Current milestone
- ...

## Validation
- ...

## Next dependency
- ...
```

Keep it concise.

---

# 62. How to choose the next milestone

After reconciliation:

1. do not simply choose the lowest-numbered missing milestone;
2. identify the earliest dependency gap blocking correctness;
3. prefer finishing a partial prerequisite over starting a new feature;
4. skip milestones already satisfied;
5. avoid frontend migration before relevant backend/domain contracts stabilize;
6. avoid semantic/AI polish while deterministic identity/search/playback is wrong;
7. prioritize production correctness and UX coherence over feature count.

Examples:

- incomplete model leases before streaming ASR;
- unsafe canonical identity before personalization;
- broken exact search before semantic search;
- provider naming leakage before visual polish;
- AI starvation of playback before adding more AI;
- transcript persistence/indexing before content summaries.

---

# 63. Investigation and review

Substantial milestones may split the investigation into independent, read-only areas.

Suggested roles:

- architecture/current-state;
- persistence/migration;
- tests/failure;
- frontend/UX;
- lifecycle/security;
- AI runtime.

Split only when the questions are genuinely independent.

Prefer one implementation owner for overlapping production code.

---

# 64. Per-milestone execution

## A. Investigate

Use code search and source inspection.

## B. Produce implementation contract

State:

- existing code to reuse;
- exact missing behavior;
- likely files/modules;
- persistence/migration impact;
- concurrency/lifecycle impact;
- AI/resource impact;
- UX impact;
- tests;
- explicit non-goals.

## C. Implement

Implement the smallest coherent contract.

Do not start later milestones.

## D. Validate

Run focused tests, then relevant broader checks.

## E. Independent review

Use a reviewer who did not author the change.

Review:

- diff;
- architecture invariants;
- callers/dependents;
- migrations;
- failure paths;
- security;
- UX regressions;
- tests.

## F. Fix confirmed findings

Verify each review finding independently.

Do not change correct code to satisfy speculative comments.

## G. Update status and stop

Update `docs/nova-implementation-status.md`.

Stop before the next milestone unless explicitly instructed.

---

# 65. Definition of done

A milestone is done only when:

- current implementation was inspected first;
- existing systems were evolved rather than bypassed;
- requested contract exists;
- relevant tests exist;
- builds/typechecks/tests pass;
- failure/cancellation behavior is covered where relevant;
- user state is preserved;
- secrets do not leak;
- migrations are safe;
- UX remains coherent;
- AI failure degrades gracefully;
- implementation status is updated;
- later work has not been silently started.

---

# 66. Milestone kickoff checklist

Use this process at the start of each milestone.

## Step 1 — Build repository context

Read:

- `CONTRIBUTING.md`;
- `docs/archive/master-plan.md` (historical milestone IDs);
- `docs/nova-implementation-status.md`;
- relevant architecture docs.

Inspect actual:

- Rust workspace;
- Tauri layer;
- React/TypeScript frontend;
- SQLite schema/migrations;
- provider ingestion;
- catalog compiler;
- naming/normalization/classification;
- fingerprints;
- entity resolution;
- identity redirects/history;
- snapshots;
- TMDB;
- search;
- jobs;
- resource governor;
- model infrastructure;
- playback;
- transcription;
- translation;
- embeddings;
- dialogue search;
- Command-K;
- smart discovery;
- personalization;
- AI settings/status UX;
- current frontend DTOs;
- tests.

## Step 2 — Reconcile the roadmap

Produce the full milestone reconciliation matrix.

Do not implement anything until the matrix exists.

Explicitly identify:

- completed milestones;
- partial milestones;
- skipped work;
- risky legacy coupling;
- duplicate/parallel implementations;
- missing AI integration;
- missing UX integration.

## Step 3 — Audit the naming/classification path specifically

Verify:

```text
raw SourceRecord preserved
      ↓
deterministic normalization
      ↓
structured parsing
      ↓
classification
      ↓
versioned fingerprint
      ↓
candidate blocking
      ↓
entity resolution
      ↓
TMDB identification
      ↓
localized production naming
      ↓
taxonomy
      ↓
aliases/search
      ↓
validated snapshot
```

Report exactly where user-facing names come from today.

## Step 4 — Audit intelligence specifically

Verify:

- model libraries;
- runtime manager/leases;
- resource governance;
- ASR;
- translation;
- embeddings;
- semantic search;
- dialogue search;
- Command-K;
- smart discovery;
- personalization;
- derived content understanding;
- provenance/versioning;
- failure UX;
- privacy/offline behavior.

## Step 5 — Audit UX/frontend specifically

Verify:

- canonical browse;
- clean titles;
- detail pages;
- resume/Continue Watching;
- search;
- series;
- live;
- player;
- watchlist/favorites;
- AI capability states;
- accessibility;
- macOS behavior.

## Step 6 — Select the first real gap

Choose the earliest dependency-correct milestone that is:

- missing;
- partial;
- risky;
- or blocking production correctness/UX.

Explain briefly why it comes next.

## Step 7 — Produce implementation contract

Before editing, state:

- code/modules to reuse;
- exact missing behavior;
- files likely affected;
- data/migration impact;
- concurrency/lifecycle impact;
- AI/resource impact;
- UX impact;
- tests;
- non-goals.

## Step 8 — Implement only that milestone

Prefer the smallest coherent change.

Do not create parallel replacement systems.

## Step 9 — Validate and review

Run relevant tests/checks.

Get an independent review.

Fix confirmed findings only.

## Step 10 — Update status and stop

Update `docs/nova-implementation-status.md`.

Then report:

1. reconciliation summary;
2. what was already complete;
3. what was partial;
4. what was skipped;
5. what milestone you implemented;
6. validation results;
7. next dependency.

Do not automatically begin another milestone unless explicitly instructed.
