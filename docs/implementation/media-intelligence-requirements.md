# Media Intelligence requirements — accepted 2026-09-19

You are acting as the **principal systems architect, senior Rust engineer, senior macOS media engineer, and AI platform architect for Nova**.

Nova is a production-grade macOS IPTV application for:

* Live TV
* Movies
* TV Series

Nova must support **very large provider libraries**, with **100,000+ raw source entries as a normal production workload**, and must be architected to remain stable at larger scales such as **250,000+ source records** without fundamental redesign.

At the same time, Nova is evolving from a conventional IPTV client into an **intelligent local media platform**.

The objective is not to add random “AI features.”

The objective is to create a coherent **Nova Media Intelligence architecture** in which catalog understanding, search, playback, captions, recommendations, stream selection, metadata enrichment, and user-facing intelligence all contribute to and consume a common media knowledge system.

The guiding principles are:

> **Providers supply raw source material. Nova owns the canonical media catalog.**

> **The number of raw provider rows must be decoupled from the cost of AI processing.**

> **100,000 raw records must not mean 100,000 LLM calls, 100,000 web searches, or 100,000 frontend objects.**

> **AI should be used where ambiguity and reasoning exist; deterministic algorithms should handle deterministic problems.**

> **The system should become cheaper and smarter over time by learning from previous resolutions, cached evidence, playback observations, and user behavior.**

> **Playback always has priority over background intelligence.**

> **Intelligence must be local-first, incremental, resumable, resource-aware, evidence-driven, and model-agnostic.**

> **AI proposes; Nova validates.**

> **The canonical catalog is authoritative. Search indexes, embeddings, recommendations, and caches are rebuildable projections.**

---

# 1. Scale target

Design Nova around explicit workload classes.

Use at least:

```text
1,000 records
developer smoke workload

10,000 records
ordinary integration workload

100,000 records
normal production-scale target

250,000 records
stress-test target

500,000 records
architectural soak/experimental target
```

The architecture must comfortably support 100k+ source records and should not require fundamental redesign at 250k.

Do not assume every playlist has the same size.

A user may have:

```text
5,000
30,000
120,000
220,000+
```

records.

Nova must adapt naturally.

---

# 2. Distinguish raw records from canonical catalog size

Raw IPTV entries are not equivalent to catalog entities.

For example:

```text
120,000 SourceRecords
        ↓
Catalog Compiler
        ↓
55,000 canonical entities
120,000 StreamVariants
```

A provider may contain:

```text
Dune Part Two 4K EN
Dune Part Two 1080p FR
Dune Part Two UHD MULTI
```

These should become:

```text
1 MovieEntity

Dune: Part Two
2024

with 3 StreamVariants
```

Therefore:

* source ingestion works on SourceRecords
* browsing works on canonical entities
* variant selection works on StreamVariants
* search primarily indexes canonical entities
* user state primarily attaches to canonical entities
* expensive AI should operate on canonical candidates, not blindly on all source rows

---

# 3. Core scaling principle

The large-scale pipeline should look approximately like:

```text
100k–250k+ SourceRecords
        │
        ▼
Streaming ingestion
        │
        ▼
Fingerprinting
        │
        ▼
Known source binding?
        │
        ├── yes → reuse
        │
        ▼
Known alias / knowledge match?
        │
        ├── yes → reuse
        │
        ▼
Deterministic extraction
        │
        ▼
Candidate resolution
        │
        ▼
Local intelligence only if needed
        │
        ▼
Web research only if still unresolved
        │
        ▼
Canonical entity / variant
```

The important rule is:

> **Only the difficult tail enters expensive intelligence.**

---

# 4. Avoid O(N²) catalog logic

Never compare every catalog item against every other item.

At:

```text
100,000 records
```

all-pairs comparison is unacceptable.

Use candidate blocking.

Examples:

Movies:

```text
kind
+ normalized title
+ year
```

Episodes:

```text
series identity
+ season
+ episode
```

Live:

```text
normalized channel identity
+ country
+ network
+ EPG ID
```

Only then perform:

* fuzzy matching
* semantic matching
* reranking
* LLM reasoning

on a very small candidate set.

Target something like:

```text
1 SourceRecord
→ 3–20 candidates
```

not:

```text
1 SourceRecord
→ 100,000 comparisons
```

---

# 5. Streaming ingestion and bounded memory

Do not load the entire provider payload into memory unless clearly justified.

Design:

```text
provider
  ↓
stream parser
  ↓
bounded queue
  ↓
workers
  ↓
batched persistence
```

Use:

* streaming M3U parsing
* bounded Tokio channels
* bounded worker pools
* backpressure
* prepared SQL
* batched writes
* transactions
* controlled SQLite writer

Do not create:

```text
100,000 Tokio tasks
```

for a 100k playlist.

Use controlled concurrency.

---

# 6. Controlled SQLite write architecture

CPU-heavy catalog analysis may be parallel.

SQLite writes should be controlled.

Prefer:

```text
worker pool
    │
    ▼
resolved batches
    │
    ▼
DB writer
    │
    ▼
transaction
```

over many unrelated tasks competing for SQLite writes.

Benchmark batch sizes and transaction strategy.

Do not assume more writers means more speed.

---

# 7. Import snapshots

Every provider refresh should create a versioned source snapshot.

Conceptually:

```text
Source
  ↓
ImportSession
  ↓
SourceSnapshot
  ↓
SourceRecord[]
```

Current catalog remains active while the new snapshot compiles.

```text
CURRENT CATALOG
      │
      │ remains usable
      ▼

NEW SNAPSHOT
      │
      ▼
compile
      │
      ▼
validate
      │
      ▼
atomic publish
```

The frontend should never observe half-imported state.

---

# 8. Incremental refresh is mandatory

Fingerprint source rows.

For example:

```text
fingerprint(
  provider_id,
  title,
  category,
  stream reference,
  important metadata
)
```

Refresh result:

```text
119,207 current records

116,900 unchanged
640 changed
1,667 new
881 removed
```

Nova should process deeply only:

```text
changed + new
```

Unchanged records reuse previous compiled results.

Removed records retire or tombstone their SourceBindings.

This means AI workload scales with **change**, not total catalog size.

---

# 9. Two definitions of import completion

Nova must distinguish:

## Ready to Browse

The catalog is structurally valid and usable.

Requirements:

* source acquisition complete
* deterministic parsing complete
* basic canonical identities available
* catalog published
* frontend can browse/search

## Fully Enriched

Background intelligence continues:

* uncertain identities
* missing metadata
* logos
* semantic indexing
* recommendations
* internet research
* artwork
* extra classifications

Never block browsing on full enrichment.

---

# 10. Import UX for large libraries

Do not show one vague progress bar.

Show meaningful stages.

Example:

```text
Adding Library

Fetching provider catalog              ✓

Reading 118,421 entries                 ✓

Organizing Movies / Series / Live       ✓

Grouping qualities and languages        ✓

Publishing library                      ✓

Ready to browse

Improving metadata and artwork...
8,420 remaining
```

The user can browse while enrichment continues.

---

# 11. Nova Media Intelligence philosophy

Create a coherent system called:

```text
Nova Media Intelligence
```

It should include three main intelligence domains:

```text
Nova Media Intelligence

├── Catalog Intelligence
├── Content Intelligence
└── Playback Intelligence
```

with a shared:

```text
Nova Media Graph
```

---

# 12. Nova Media Graph

The graph is the conceptual knowledge layer connecting everything Nova learns.

Example Movie:

```text
MovieEntity
│
├── canonical identity
├── localized names
├── external IDs
├── year
├── genres
├── people
├── artwork
├── StreamVariants
├── transcript
├── chapters
├── scenes
├── concepts
├── recommendations
└── user state
```

Example Live channel:

```text
LiveChannel
│
├── canonical identity
├── aliases
├── network
├── country
├── languages
├── category
├── logo
├── StreamVariants
├── EPG
├── stream health
└── user affinity
```

The graph should not necessarily mean introducing a graph database.

SQLite can remain the authoritative persistence layer.

“Media Graph” refers to the **domain relationships and knowledge model**.

---

# 13. Catalog Intelligence

Catalog Intelligence turns chaotic provider data into clean Nova entities.

Responsibilities:

* standardized names
* content type
* language
* country
* region
* genre hints
* stream quality
* codec hints
* identity resolution
* variant grouping
* logo resolution
* missing metadata research
* alias discovery
* canonical entity matching
* confidence/provenance

---

# 14. Catalog Intelligence Agent

Create a bounded agent such as:

```text
CatalogIntelligenceAgent
```

The agent can use tools like:

```text
search_known_entities()
search_aliases()
inspect_previous_snapshot()
inspect_provider_context()
inspect_epg()
lookup_metadata_service()
research_web()
inspect_logo()
inspect_stream()
query_catalog()
store_evidence()
```

The agent should never return unstructured prose as the final catalog result.

It returns strict structured proposals.

---

# 15. Context-aware catalog reasoning

Do not reason from one raw title alone.

Input context may include:

```text
raw title
provider group
provider region
neighboring items
EPG ID
logo URL
stream filename
provider category
previous matches
existing aliases
known channel/entity database
```

Example:

```text
title:
TF1 FHD

group:
FR | GENERALISTES

epg_id:
TF1.fr

neighbors:
France 2
France 3
M6
```

This context materially improves identity resolution.

---

# 16. Standardized naming

Nova should produce clean canonical names readable on:

* macOS
* TVs
* phones
* compact card layouts

Example:

```text
UK | SKY SPORTS PREMIER LEAGUE FHD
→ Sky Sports Premier League
```

```text
FR - TF1 UHD HEVC
→ TF1
```

```text
AR:MBC2 HD
→ MBC 2
```

Remove technical/provider noise.

Examples of removable stream metadata:

```text
FHD
UHD
4K
HEVC
H265
50FPS
BACKUP
SERVER 2
VIP
```

but only when those tokens do not change identity.

Preserve meaningful distinctions:

```text
BBC One
BBC Two

MBC 1
MBC 2

beIN Sports 1
beIN Sports 2
```

---

# 17. Canonical name versus variant metadata

Never use variant characteristics as media identity.

Bad:

```text
Dune Part Two 4K FR HEVC
```

Canonical entity:

```text
Dune: Part Two
2024
```

Variant:

```text
UHD
HEVC
French
```

Same rule for:

* Movies
* Episodes
* Live channels

---

# 18. Language inference

Detect language using evidence such as:

* provider group
* title tags
* known entity
* EPG
* country
* network
* metadata service
* web research
* stream/audio inspection

Differentiate:

```text
channel primary language
```

from:

```text
available stream audio languages
```

when possible.

Use standard language identifiers internally.

---

# 19. Stream-quality inference

Infer:

```text
SD
HD
FHD
UHD / 4K
```

from:

* provider title
* group
* filename
* Xtream metadata
* stream metadata
* observed playback

Provider claims are hints.

Observed playback metadata is stronger.

Example:

```text
provider:
4K

libmpv observes:
1920×1080
```

Nova should learn:

```text
actual resolution = FHD
```

while retaining the original provider claim as provenance.

---

# 20. Content-type inference

Infer controlled Nova types/categories.

Live examples:

```text
General Entertainment
News
Sports
Movies
Series
Kids
Documentary
Music
Lifestyle
Religious
Shopping
Regional
International
24/7
Event
Radio
Other
```

VOD:

```text
Movie
Series
Episode
Special
Documentary
Event
Other
```

Do not permit the model to create arbitrary category strings.

---

# 21. Logo intelligence

If a channel logo is:

* missing
* invalid
* low-quality
* clearly wrong

Nova can invoke a Logo Resolver.

Pipeline:

```text
channel identity
      ↓
known logo?
      ↓ no
trusted metadata source
      ↓
web research if needed
      ↓
candidate logos
      ↓
identity validation
      ↓
cache best asset
```

Frontend should normally load a Nova-managed cached asset, not repeatedly hotlink random third-party hosts.

Store:

```text
source URL
source domain
retrieval time
hash
dimensions
confidence
```

---

# 22. Web research is a fallback, not default

Use internet research when:

* identity remains uncertain
* logo is missing
* metadata is incomplete
* aliases are unknown
* local knowledge is insufficient

Resolution ladder:

```text
cache
  ↓ miss

known entity / alias
  ↓ miss

deterministic resolution
  ↓ uncertain

local AI
  ↓ uncertain

metadata services / XMLTV
  ↓ uncertain

targeted web research
```

Do not perform web research for every source row.

---

# 23. Entity-level web research

Research real entities, not stream rows.

Example raw rows:

```text
TF1
TF1 HD
TF1 FHD
TF1 UHD
FR TF1
TF1 Backup
```

should collapse into one research problem:

```text
TF1
```

Once resolved, all related aliases/variants reuse the evidence.

---

# 24. Persistent research cache

Create a persistent cache.

Conceptually:

```text
CatalogResearchCache

resolution_key
entity_candidate
facts
sources
retrieved_at
expires_at
resolver_version
confidence
```

Stable information should be cached for long periods.

Dynamic facts may expire sooner.

---

# 25. Local knowledge base

Successful resolutions become reusable knowledge.

Conceptually:

```text
KnownEntity

entity_id
canonical_name
aliases
type
country
languages
network
external_ids
logos
metadata
provenance
```

This allows future imports to become progressively cheaper.

---

# 26. Intelligence should get cheaper over time

First time:

```text
FR | TF1 FHD
```

may require:

```text
local reasoning
+ metadata lookup
```

Later:

```text
TF1 UHD HEVC
```

should resolve through:

```text
KnownEntity / alias match
```

with no LLM and no internet.

This is a major design objective.

---

# 27. AI should not replace deterministic computation

Never use an LLM for obvious structured facts.

Examples that should remain deterministic:

```text
3840×2160 → UHD

S03E07 → Season 3 Episode 7

hash comparison

date parsing

ISO normalization

exact ID matching

SQLite lookup

sorting

pagination

filtering
```

Use AI where interpretation is actually necessary.

Examples:

```text
Which real channel does this ambiguous provider name refer to?

Are these two noisy multilingual movie names the same entity?

What is the likely canonical name given contradictory evidence?
```

---

# 28. AI proposes; Nova validates

Architecture:

```text
AI / Agent
    ↓
structured proposal
    ↓
schema validation
    ↓
policy validation
    ↓
evidence aggregation
    ↓
confidence decision
    ↓
canonical catalog
```

Models must never directly mutate authoritative catalog state without validation.

---

# 29. Confidence and provenance

Every intelligence-derived fact should support:

```text
value
confidence
source
resolver/model version
timestamp
```

Potential sources:

```text
provider
parser
known entity
XMLTV
TMDB
web research
AI inference
player observation
user correction
```

Policy determines which fact wins.

---

# 30. Conflict handling

Do not silently hide contradictions.

Example:

```text
provider says:
US

known entity says:
CA
```

Store both evidence candidates.

The resolver decides based on confidence/source priority.

If uncertainty remains:

```text
ProvisionallyResolved
```

or:

```text
NeedsResearch
```

rather than inventing certainty.

---

# 31. Intelligence states

Support at least:

```text
Resolved
ProvisionallyResolved
NeedsResearch
Unknown
Invalid
```

This allows the catalog to remain usable without pretending all metadata is perfect.

---

# 32. Model-agnostic architecture

Do not hard-code model names into domain contracts.

Use interfaces such as:

```rust
trait CatalogReasoner {
    fn resolve(
        &self,
        request: CatalogResolutionRequest,
    ) -> CatalogResolution;
}

trait SemanticMatcher {
    fn candidates(
        &self,
        item: &CatalogInput,
    ) -> Vec<EntityCandidate>;
}

trait IntelligenceProvider {
    fn generate(
        &self,
        request: IntelligenceRequest,
    ) -> IntelligenceResponse;
}
```

Actual models belong in the Stack Manifest.

---

# 33. Nova Intelligence Runtime

Create a central runtime for local AI workloads.

Conceptually:

```text
Nova Intelligence Runtime
        │
        ├── ModelManager
        ├── IntelligenceScheduler
        ├── ResourceGovernor
        ├── JobQueue
        ├── Cache
        └── Provider adapters
```

Do not let every feature independently load its own models and consume system resources without coordination.

---

# 34. IntelligenceScheduler

All intelligence workloads should have priorities.

Example:

```text
REALTIME

playback-critical captions
live translation
stream fallback intelligence

INTERACTIVE

user search
catalog query agent
summary request

BACKGROUND

catalog enrichment
web research
recommendations
chapters
artwork

IDLE

large embedding jobs
thumbnail analysis
maintenance
reindexing
```

Playback-sensitive work always wins.

---

# 35. Playback has priority

If a user is watching 4K content:

```text
Playback               HIGH
AI captions            HIGH
Translation            HIGH
Interactive search     HIGH/MEDIUM

Catalog enrichment     LOW
Logo analysis          LOW
Embedding rebuild      LOW
Thumbnail generation   LOW
```

Background tasks should slow down or pause.

AI must never make playback stutter merely because enrichment is running.

---

# 36. ResourceGovernor

Introduce a resource-management layer aware of:

```text
memory pressure
CPU
GPU/accelerator usage
active playback
battery
power state
thermal pressure where available
network availability
```

Behavior examples:

```text
playback active
→ reduce background workers

memory pressure
→ unload optional model

battery
→ delay low-priority enrichment

plugged in + idle
→ increase enrichment throughput
```

---

# 37. ModelManager

Manage model lifecycle centrally.

Responsibilities:

* install/download
* verify checksum
* version
* load
* unload
* memory budget
* model capability
* compatibility
* fallback
* cache

Do not assume all models fit in memory simultaneously.

---

# 38. Modular AI capabilities

Model/capability bundles may include:

```text
Catalog Intelligence
Semantic Search
Transcription
Translation
Vision / Scene Analysis
Advanced Summarization
```

Capabilities should degrade gracefully if unavailable.

Nova should still browse and play content without optional advanced models.

---

# 39. Resumable AI jobs

Long background work must survive:

* application quit
* sleep
* network loss
* crash
* restart

Persist jobs.

Conceptually:

```text
IntelligenceJob

id
entity_id
job_type
priority
status
attempts
input_version
model_version
created_at
updated_at
```

States:

```text
Pending
Running
Completed
Failed
Deferred
Stale
```

On restart, interrupted jobs are safely resumed/requeued.

---

# 40. Do not create one AI job per source row unnecessarily

Prefer entity-level and batch-level processing.

Example:

```text
100,000 SourceRecords

↓ grouping / candidate resolution

45,000 canonical entities

↓ only uncertain entities

3,000 AI candidates

↓ only unresolved tail

300 web research tasks
```

Exact numbers vary by provider.

The architecture principle does not.

---

# 41. Search architecture

Canonical catalog:

```text
Canonical Catalog
       │
       ├── FTS index
       ├── alias index
       ├── semantic entity index
       ├── transcript index
       └── recommendation features
```

Do not make raw StreamVariants appear as separate primary search entities unless the user explicitly searches variants.

---

# 42. Semantic embeddings at scale

Prefer embeddings for canonical entities rather than every raw source row.

For example:

```text
120k SourceRecords
→ 50k canonical entities
→ ~50k entity embeddings
```

Use:

* incremental embedding generation
* configurable dimensions
* quantization where useful
* disk-backed/local index
* rebuildable index format

Do not load enormous vector sets unnecessarily into frontend memory.

---

# 43. Separate transcript scale from catalog scale

Catalog:

```text
100k entities
```

and transcript data:

```text
millions of timed chunks
```

are different workloads.

Use separate logical storage/indexing layers.

Conceptually:

```text
Catalog DB

Transcript DB / index

Semantic index

Artwork cache

Thumbnail cache

Model cache

Timeshift/DVR storage
```

Do not create one monolithic storage bucket for everything.

---

# 44. Frontend scaling

Do not send the entire catalog into React.

Use:

```text
React
  ↓
Catalog Query API
  ↓
SQLite
```

Queries:

```text
kind = Movie
genre = Action
country = FR
sort = ...
cursor = ...
limit = 100
```

Frontend receives lightweight page/card models.

---

# 45. Virtualization

At 100k+:

```text
database rows
≠
mounted React components
```

Only visible and near-visible cards should render.

Target:

```text
tens of mounted cards
```

not:

```text
tens of thousands
```

Use virtualization and cursor/pagination.

---

# 46. Lazy artwork

Do not decode tens of thousands of posters/logos at startup.

Pipeline:

```text
visible item
   ↓
request asset
   ↓
memory cache
   ↓
disk cache
   ↓
network only if necessary
```

Keep heavy artwork outside ordinary catalog query payloads.

---

# 47. Playback Intelligence

Nova should learn from playback.

For every StreamVariant, track:

```text
startup latency
success/failure
reconnect rate
buffering
observed resolution
codec
FPS
HDR
bitrate
audio format
recent availability
```

This turns playback into a source of catalog truth.

---

# 48. Variant Intelligence

Auto mode should not simply select the stream claiming the highest resolution.

Consider:

```text
resolution
codec
HDR
device support
network
startup latency
recent reliability
rebuffering
language
user preference
```

Select the best likely playback experience.

---

# 49. Intelligent fallback

If the preferred variant fails:

```text
Variant A
   ↓ failure
VariantSelector
   ↓
Variant B
```

For Live this may be almost seamless.

For VOD preserve timestamp when switching where possible.

---

# 50. Stream diagnosis

Playback Intelligence can classify likely failures:

```text
provider unavailable
expired URL
network interruption
decoder problem
unsupported audio
unstable source
```

User-facing behavior should remain understandable.

Example:

```text
This source stopped responding.
Trying another available source…
```

instead of exposing raw engine errors.

---

# 51. Content Intelligence

After catalog and playback intelligence are stable, use content understanding.

Potential features:

* transcript-aware search
* semantic scene search
* chapter generation
* smart thumbnails
* intro detection
* recap detection
* credits detection
* summaries
* catch-up summaries
* named-entity-aware captions
* context-aware translation
* sports highlights

Do not make all of these release-blocking.

---

# 52. Transcript-aware timeline

Leverage existing transcription.

Timeline hover can show:

```text
thumbnail
timestamp
transcript snippet
chapter title
```

Example:

```text
01:17:31
"We need to leave now."
```

Clicking jumps to that moment.

---

# 53. Natural-language media search

Support future requests such as:

```text
"Where does he mention Paris?"

"Show French thrillers from the 2010s I haven't watched."

"Arabic news channels."

"My unwatched 4K science fiction movies."

"Continue the comedy series I watched last weekend."
```

The system should convert these into structured catalog/search operations.

Do not require a giant chat interface.

`⌘K` can become an intelligent command/search interface.

---

# 54. Recommendations

Keep recommendations local and explainable.

Possible rails:

```text
Because you watched Severance

French crime dramas

Unfinished episodes

Recently added Arabic movies

4K titles in your preferred language

Sports channels you usually watch at this time
```

Use:

```text
content similarity
watch history
completion
language
genre
recency
time/context
favorites
search behavior
```

No cloud user profile is required.

---

# 55. Live recommendations

Use:

```text
EPG
channel affinity
time of day
content type
watch history
```

to rank:

```text
Live Now
```

This is particularly valuable for IPTV.

---

# 56. Smart chapters

Combine:

```text
scene boundaries
+
transcript
+
audio events
```

to derive chapter candidates.

Store them as Nova metadata.

Do not modify media files.

---

# 57. Smart thumbnails

Use sampled frames, not every frame.

Pipeline:

```text
sample frames
   ↓
cheap rejection
   ↓
candidate frames
   ↓
visual scoring
   ↓
best preview
```

Reject:

* black frames
* heavy blur
* duplicate frames
* title cards where inappropriate

---

# 58. Intro / recap / credits

Potential signals:

```text
visual similarity across episodes
audio fingerprint
transcript patterns
episode position
duration consistency
```

Use these to create:

```text
Skip Intro
Skip Recap
Skip Credits
```

later.

---

# 59. Caption intelligence

Existing AI captions should evolve.

Use media context for transcription correction.

For example:

```text
Movie:
Dune: Part Two

Known entities:
Arrakis
Atreides
Harkonnen
Fremen
```

This may improve named-entity consistency.

---

# 60. Translation intelligence

Translation can use:

```text
previous caption context
known names
series/movie glossary
speaker context if known
```

Maintain a scoped:

```text
MediaGlossary
```

to keep names/terms consistent.

---

# 61. Catch Me Up

For series:

```text
Catch Me Up
```

should summarize only episodes the user has already completed.

Never spoil future episodes unless explicitly requested.

---

# 62. AI evaluation

AI quality requires its own evaluation framework.

Maintain versioned datasets for:

```text
canonical naming
identity resolution
false merge rate
language detection
content classification
logo resolution
search relevance
recommendations
translation
caption correction
tool use
```

Measure scores historically.

Do not upgrade a model just because it is newer.

Upgrade when it improves Nova's measured workload.

---

# 63. Catalog evaluation

Build a representative ugly-IPTV corpus with:

* Arabic
* French
* English
* multilingual mixes
* numbered channels
* sequels
* remakes
* episodes
* 24/7 channels
* sports
* multiple qualities
* multiple languages
* missing logos
* wrong logos
* conflicting metadata

Measure:

```text
classification accuracy
canonical naming accuracy
identity accuracy
false merges
false splits
language accuracy
variant grouping accuracy
logo correctness
```

False merges should be heavily penalized.

---

# 64. 100k generated benchmark provider

Create a deterministic provider fixture containing at least:

```text
100,000 SourceRecords
```

with:

* Movies
* Series
* Episodes
* Live
* variants
* duplicates
* malformed titles
* multiple languages
* multiple qualities
* missing metadata
* missing logos
* invalid entries

Also create:

```text
250,000-record stress fixture
```

for nightly/stress testing.

---

# 65. Scale metrics

Track for each benchmark:

```text
record count
parse duration
compile duration
identity duration
DB write duration
publish duration
peak memory
DB size
semantic index size
AI jobs created
AI jobs avoided through cache
web requests
canonical entities
variants
false merges
identity churn
search p50/p95
incremental refresh duration
```

---

# 66. Incremental benchmark

Do not benchmark only first import.

Simulate:

```text
100k baseline

then:

98% unchanged
1% modified
1% new/removed
```

Measure incremental refresh separately.

The objective is:

> refresh cost should correlate primarily with changed data, not total library size.

---

# 67. Performance regressions

Track benchmark history.

Flag major regressions such as:

```text
peak memory:
600 MB → 2.8 GB

100k import:
12 s → 48 s

search p95:
40 ms → 600 ms
```

Do not claim optimization without measurement.

---

# 68. User hardware variability

Do not assume high-end Apple Silicon.

Nova should support capability tiers.

Conceptually:

```text
Basic
Standard
High Performance
```

The application should adapt:

```text
smaller model
reduced worker count
deferred enrichment
less aggressive vision work
```

when resources are limited.

Optional intelligence features should never make basic playback unavailable.

---

# 69. AI model storage

Do not bundle every model into Nova.app.

Use managed model packages.

Track:

```text
model ID
revision
size
hash
runtime
capability
minimum resources
install state
```

Store them in the Stack Manifest / model manifest.

---

# 70. Offline behavior

After initial enrichment, the catalog should remain useful offline.

Cached:

* identities
* aliases
* metadata
* logos
* search indexes
* recommendations
* transcripts
* translations

should continue to work locally.

Internet research enhances Nova but should not become a runtime dependency for ordinary browsing.

---

# 71. Security

Catalog intelligence and web research must treat all provider/network data as untrusted.

Protect against:

* malformed provider metadata
* malicious URLs
* oversized fields
* unsafe redirects
* credential leakage
* arbitrary file writes
* poisoned logo/media downloads
* prompt injection from untrusted web/provider text

Do not allow provider metadata or web pages to issue instructions to the catalog agent.

Treat external text as **data**, not commands.

---

# 72. Agent boundaries

Catalog agents should have explicit tool permissions.

For example:

```text
read catalog
read metadata
read EPG
perform bounded web search
write structured evidence
```

They should not have arbitrary filesystem/shell/database privileges.

---

# 73. Version everything

Track:

```text
catalog schema
compiler version
normalization version
identity version
taxonomy version
agent prompt version
tool schema version
knowledge-base schema
embedding model
reasoning model
transcription model
translation model
recommendation model
semantic index format
```

This allows reproducibility and debugging.

---

# 74. Required architecture deliverable

Produce a document titled:

# Nova 100K+ Scale & Media Intelligence Architecture

Include:

## 1. Executive architecture decision

Explain how Nova should support 100k+ source entries while remaining local-first and responsive.

## 2. Workload assumptions

Define normal/stress/soak targets.

## 3. SourceRecord architecture

Describe streaming ingestion.

## 4. Snapshot architecture

Describe provider refresh behavior.

## 5. Incremental compilation

Fingerprinting and reuse.

## 6. SQLite persistence design

Tables, indexes, batching, transactions, writer architecture.

## 7. Canonical catalog design

Entities versus variants.

## 8. Candidate blocking

Explain how to avoid O(N²).

## 9. Catalog Intelligence Agent

Tools, inputs, outputs, boundaries.

## 10. Knowledge base

Aliases, canonical identities, provenance.

## 11. Research cache

Internet metadata caching and invalidation.

## 12. Web research architecture

When it runs and how it stays bounded.

## 13. Naming intelligence

Standardized names.

## 14. Language intelligence

Detection and provenance.

## 15. Quality intelligence

Provider claims versus observed playback.

## 16. Content classification

Controlled taxonomy.

## 17. Logo intelligence

Search, validation, caching.

## 18. Confidence engine

Resolution thresholds and uncertainty.

## 19. AI model abstraction

Model-independent interfaces.

## 20. Nova Media Graph

Domain relationships.

## 21. Intelligence Runtime

Central runtime architecture.

## 22. ModelManager

Lifecycle and memory.

## 23. IntelligenceScheduler

Priorities.

## 24. ResourceGovernor

Memory/CPU/playback/battery-aware scheduling.

## 25. Persistent job architecture

Resumability.

## 26. Frontend scale architecture

Pagination, virtualization, query API.

## 27. Search architecture

FTS + semantic + transcript.

## 28. Embedding architecture

Entity-level indexing and incremental updates.

## 29. Playback Intelligence

Observed stream facts.

## 30. Variant Intelligence

Quality/language/reliability-aware selection.

## 31. Intelligent fallback

Live and VOD.

## 32. Content Intelligence

Future features.

## 33. Transcript-aware timeline

Architecture.

## 34. Conversational search

Architecture.

## 35. Recommendation system

Local explainable ranking.

## 36. Live recommendation system

EPG/context-aware ranking.

## 37. Smart chapters

Future architecture.

## 38. Smart thumbnails

Future architecture.

## 39. Intro/credits detection

Future architecture.

## 40. Caption intelligence

Context-aware ASR corrections.

## 41. Translation intelligence

Context/glossary architecture.

## 42. Local-first/offline behavior

Requirements.

## 43. Security

Agent/tool/network boundaries.

## 44. 100k benchmark fixture

Exact design.

## 45. 250k stress fixture

Exact design.

## 46. Performance measurements

Metrics and regression gates.

## 47. AI quality evaluation

Datasets and scoring.

## 48. Hardware capability tiers

Graceful scaling.

## 49. Failure handling

Subsystem by subsystem.

## 50. Implementation phases

Use something like:

### Phase A — Scale baseline

* 100k generated fixture
* current performance baseline
* memory baseline
* search baseline

### Phase B — Streaming ingestion

* SourceRecord
* snapshots
* bounded pipeline
* DB writer
* atomic publish

### Phase C — Incremental catalog

* fingerprints
* reuse
* canonical entities
* variants
* candidate blocking

### Phase D — Catalog Intelligence

* agent
* knowledge base
* provenance
* confidence
* research cache
* web tools

### Phase E — Frontend scale migration

* query API
* pagination
* virtualization
* lazy artwork

### Phase F — Intelligence Runtime

* ModelManager
* Scheduler
* ResourceGovernor
* persistent jobs

### Phase G — Playback Intelligence

* observed stream metadata
* reliability
* variant selection
* fallback

### Phase H — Search / Discovery Intelligence

* semantic search
* natural language
* recommendations
* Live Now ranking

### Phase I — Content Intelligence

* smart chapters
* thumbnails
* summaries
* intro/credits
* richer captions

For every phase include:

* objective
* architecture changes
* expected workload
* benchmarks
* tests
* E2E requirements
* failure modes
* migration strategy
* rollback
* acceptance criteria

---

# 75. Required acceptance targets

Propose measurable acceptance criteria.

Examples:

```text
100k import completes without UI lockup

catalog becomes browsable before full enrichment

frontend never loads all 100k entities into React state

incremental refresh reprocesses only changed/new records

no O(N²) duplicate matching

AI/web research runs only for unresolved entities

background enrichment pauses/reduces during playback

application remains usable if all optional AI models are unavailable

catalog remains useful offline after enrichment

intelligence jobs resume safely after restart
```

Also propose realistic performance budgets based on benchmarks rather than arbitrary assumptions.

---

# 76. Final architecture philosophy

End with a concise implementation philosophy.

It should reflect:

> **Nova is not an IPTV player with AI bolted on.**

It is:

> **A local media platform that converts large, chaotic provider libraries into a progressively improving canonical media graph, then uses that knowledge to improve browsing, search, playback, language handling, reliability, recommendations, captions, and future content intelligence.**

And:

> **The catalog may contain 100,000+ raw records, but expensive intelligence should only touch the small fraction of records that actually need reasoning.**

And:

> **Stream instead of load. Index instead of scan. Retrieve candidates instead of compare-all. Batch instead of write-one. Cache instead of recompute. Enrich incrementally instead of blocking. Research unknown entities instead of every source row. Virtualize instead of render-all. Prioritize playback over background work.**

The final architecture should make Nova more intelligent as the user keeps using it, while making repeated imports cheaper—not more expensive.
