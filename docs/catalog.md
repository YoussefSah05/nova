# Catalog migration: implemented contracts

The current serving catalog remains the existing provider-row implementation.
`nova_core::catalog` is an additive, unconsumed domain module. It does not yet
implement a Catalog Compiler, canonical database, durable snapshot store or identity resolver.

Implemented contracts:

- Distinct opaque IDs for sources, snapshots, records, entities, editions, variants
  and bindings. Constructors and serde reject empty/oversized/unsafe identifiers.
  IDs are supplied by their future authoritative owner, never derived from titles.
- `SourceRecordDraft` contains untrusted provider text. `SourceRecord::try_from`
  enforces byte/count bounds without cleaning or normalizing the original fields.
- Provider keys carry a namespace and remain source-scoped evidence. Two occurrences
  can share a provider key; duplicate policy belongs to the future compiler.
- `StreamReference` is an opaque protected-locator reference, not an upstream URL.
  Adapters must not put credentials in IDs. Raw metadata may still contain secrets;
  it requires protected persistence and cannot be exported to diagnostics/IPC.
- SourceRecord has redacted Debug, immutable getters and no serialization/IPC derive.
  UpstreamKey also redacts Debug. Public draft fields are construction inputs only.
- Content kinds include Unknown; episode numbering defaults to None, with season
  zero supported for specials. A Season kind is a container, not a playable stream.
- StreamVariant expresses separate entity/edition/binding IDs as a resolved view.
  It does not grant a permanent database owner; membership is generation-scoped.

Bounds are initial source-payload limits: 16 KiB per text field, 64 attributes,
128 KiB total text payload, 128 ASCII bytes per opaque ID. Empty raw titles are
preserved so the compiler can quarantine them with evidence. Empty upstream
namespaces/values and attribute names are invalid. Parsers must additionally bound
incoming bytes before allocation; constructing a huge draft and rejecting it later
does not protect the network boundary from memory exhaustion.

Validation: `cargo test -p nova-core --locked` covers preserved multilingual text,
ID serde validation, redaction, oversized input, upstream collisions, separate
entity/variant identity, unknown numbering and compile-time ID incompatibility.
There are no data migrations, runtime feature flags or compatibility adapters yet.

## Snapshot capture and fingerprints

`catalog::snapshot` now provides bounded capture and immutable endpoint receipts:

- `SnapshotCapture` hashes replay bytes incrementally and counts accepted records.
  It rejects records belonging to a different source/snapshot. Limits are explicit,
  nonzero caller-supplied byte and record budgets.
- Successful sealing records Complete or Partial; Failed and Cancelled produce no
  receipt. Every terminal state rejects further input. A quota error is terminal
  so truncated input cannot later be accidentally sealed as complete.
- `select_snapshot` replaces a scope only with a complete candidate. Partial
  refreshes retain the previous complete scope with a stale marker. Partial first
  import is an explicit status, not a deletion-authoritative complete result.
- Receipt completion does not certify catalog validity or durable payload storage.
  Compiler validation/publication states will be separate. Empty complete endpoints
  still require destructive-collapse policy checks before publishing deletions.

`catalog::fingerprint` defines `sha256-framed-v1`. Exact replay payload bytes use
standard SHA-256. Metadata frames the domain tag, kind, optional upstream namespace
and value, raw title, optional group/logo/EPG ID, then sorted attributes/count.
Optional values include a presence byte, distinguishing missing from empty. Every
variable field has a u64 big-endian byte length. Locator hashes frame their own
domain tag, canonical locator bytes and credential revision.

Metadata excludes capture IDs and opaque locator references. It remains conservative:
all unknown metadata attributes participate, so adapters must keep token/credential
data in the transport boundary to avoid unnecessary semantic invalidation. Fingerprints
are change/integrity keys, not canonical identities or encryption. Debug hides digest
values; explicit hex export is intended for protected persistence/integrity checks.

`sha2` 0.10.9 was already in Cargo.lock and workspace dependencies; core now uses it.
No external package version changed. Independent Python hashlib vectors validate the
Rust metadata/locator encoding, in addition to the standard SHA-256 payload vector.
No Python dependency or virtual environment is required by the Rust tests.

`nova_storage::snapshots` now provides a Unix-only encrypted file-store prototype.
It retains raw bytes and receipt metadata together, verifies both on replay, and
commits immutable files without replacement. Process-interruption tests cover the
staging/commit boundary. See [ADR 0004](adr/0004-protected-snapshot-files.md) for the
format, trust boundary, 16 MiB ceiling and cancellation semantics. The store is
not connected to imports and does not persist normalized SourceRecords yet.

`ProfileKeyVault` separates new-key provisioning from read-only loading; its
macOS adapter uses the data-protection Keychain without overwriting existing keys.
The native provisioning probe failed with MissingEntitlement, so this remains
unconnected to imports. See [ADR 0005](adr/0005-profile-snapshot-keys.md).

The v2 `snapshots::stream` prototype handles larger payloads through 64 KiB frames
and enforces byte/file quotas under an OS lock. V1 storage is unchanged. Replay
output stays provisional until the final receipt succeeds; callers must stage it.
Unknown/orphan files consume quota, and full stores reject new work. No automatic
deletion is implemented. See [ADR 0006](adr/0006-streaming-snapshot-quotas.md).

`nova-catalog-ingest` now pairs bounded SourceRecords with byte-exact Xtream object
evidence and stages them in encrypted archives. It preserves duplicate upstream
occurrences, rejects duplicate occurrence IDs and binds replay to the expected raw
snapshot receipt. This is a pure per-object adapter, not a replacement network
importer. See [ADR 0007](adr/0007-protected-record-staging.md) for the derived-artifact
count semantics, provisional callbacks, quotas and provenance limits.

Next steps: bounded array framing and offsets, raw/staged evidence coordination,
episode/M3U adapters, signed-app key lifecycle and reference-aware retention.
Actual import/publication changes
require regression, migration and real-app E2E evidence before activation. See the
production blueprint for the proposed full architecture; do not confuse it with
the implementation described here.
# Catalog intelligence update — 2026-09-18

Intelligent understanding is now a required subsystem in the production plan.
The unconsumed `nova_core::catalog::intelligence` module validates bounded,
record/snapshot-scoped proposals against caller-registered evidence, retains
conflicts and separates inferred hints from observed properties. It performs no
inference, research, persistence, identity merge or app import changes. See
[ADR 0008](adr/0008-catalog-intelligence-evidence.md) and the
[code baseline](implementation/intelligence-baseline.md). Existing catalog
foundations and their remaining integration gates are documented below.

The additive `nova-catalog-reasoner` crate now runs the evaluated v2 protocol against
a bounded local inference endpoint. It rejects forbidden links and explicit metadata
contradictions, then bridges accepted suggestions into expiring inference evidence.
It has passed a synthetic real-Qwen transport probe. It remains unconnected to the
desktop importer; context vetting, protected derivation/cache storage and runtime
ownership are still required. See [ADR 0009](adr/0009-local-catalog-reasoner.md).

The reasoner's Unix-only `protected` module now persists derivations in the existing
encrypted, quota-limited store and supports cache-first reuse by a caller-supplied
artifact reference. It retains exact context/protocol/output and original expiry,
and returns new results only after a protected commit. Context vetting, a durable
artifact index, retention, runtime ownership and desktop integration remain pending.
See [ADR 0010](adr/0010-protected-catalog-derivations.md).

### Entity resolution (P09)

`nova_core::catalog::resolution` decides which canonical entities are the same
work. It is pure and deterministic: the same evidence yields the same plan in any
input order. The rules are biased against false merges, since a false merge
corrupts user state and a missed one only shows a duplicate card.

- **Evidence** (`EntityEvidence`): titles (normalized by `clean::dedup_key`, which
  keeps distinguishing numbers), years, and trusted IDs namespaced by authority and
  media kind (`tmdb:movie`). Storage builds it from the entity row, its bound
  provider rows and its durable aliases. It is never built from model output.
- **Blocking**: only entities sharing a trusted ID, series coordinate or normalized
  title are compared (P08 `blocking`). Blocks above 64 members abstain.
- **Hard contradictions** always reject: different media kind (movie vs series),
  conflicting IDs in one namespace, different series, different season/episode
  numbers, years more than one apart, or a recorded cannot-link.
- **Merge rules**: a shared trusted ID corroborated by a shared title or known
  years; identical complete episode coordinates in the same series; or an identical
  normalized title *and* an identical year. A missing year, a year one apart, an
  uncorroborated ID or internally inconsistent evidence abstains.
- **No transitive false merges**: an entity whose partners contradict each other is
  ambiguous and merges with none of them. Clusters join only while every
  cross-pair is contradiction-free and at most 32 members.
- **Survivor**: the oldest entity, then the lowest ID. `RULESET_VERSION` is recorded
  on every merge and re-resolution runs once when it changes.

Only the resolver can construct a `MergeDecision` (private fields, enforced by
compile-fail doctests), and storage merges nothing else. Abstentions produce
bounded `AmbiguityCase`s (at most 8 real candidates). `assess_ai_verdict`
validates a model's `abstain` / `same_as` / `distinct_from` answer against exactly
those candidates and returns an advisory `AiSuggestion`, which has no path to an
identity change.

Storage (migration 0023, `CanonicalRepository` identity methods) applies each
decision atomically in the import transaction:

- variants move to the survivor;
- favorite/watchlist are ORed, and the most recent resume/completion wins;
- the absorbed title/year/aliases become survivor aliases, and cannot-links follow
  the merge;
- the absorbed entity is tombstoned (`state = 'merged'`, never deleted) and
  redirected, with chains compressed to one hop;
- an append-only `entity_lineage` row (triggers reject UPDATE/DELETE) records the
  rule, ruleset and both sides' prior state, secret-free.

`canonical_entity_id` resolves historical IDs with a bounded (16-hop),
cycle-checked walk. Corrupt redirects are errors and are never followed.
Provider-claimed TMDB IDs (Xtream `tmdb`) are stored in
`vod_items.provider_tmdb_id`, separate from the lexical display-only TMDB
enrichment, and persisted as `external` aliases so they keep blocking false
merges after their row disappears. `record_cannot_link` records a user's "these
are different" correction; it has no UI yet.

### Compile → validate → atomic publish (P10)

Every import builds one **candidate** generation per source in a single
transaction, then validates it before it can replace the active one.

- **Build:** a refresh first measures the active catalog, then snapshots user
  state and clears the source's rows. It inserts the received entries, restores
  user state, and compiles and resolves canonical entities. Other connections keep
  reading the active generation (WAL) until commit.
- **Validate** (`nova_core::catalog::publish`, measured by
  `PublicationRepository::{measure_tx, candidate_report_tx}`):
  - *Integrity:* persisted rows equal received rows per kind, no movie row lacks
    a canonical variant, no variant belongs to a merged entity, and every merged
    entity redirects to an active one.
  - *Destructive collapse* (refresh only): a kind with at least 50 items that
    would keep under 10% is rejected. An endpoint answering `[]` or a truncated
    list without an error is not allowed to delete a library.
- **Publish:** `touch_refreshed_tx` and a `published` row in `catalog_publications`
  (migration 0024) are written in the same transaction and commit atomically. The
  source's active generation is its latest published row.

Any failure (storage error, endpoint error or stall on refresh, no entries,
rejected candidate) rolls the transaction back explicitly. After that, it is
logged as a `failed`/`rejected` row with a secret-free `reason_code` and summary.
Failures never change the active generation or `last_refreshed_at`. A candidate
abandoned mid-build (app killed during refresh) is discarded by SQLite on reopen.

### TMDB matching (P11)

TMDB metadata is **external evidence on a Nova entity, not its identity**. A
match stores the TMDB ID as `external_id` in `entity_external_matches` (migration
0025), keyed by the canonical entity and a per-kind namespace (`tmdb:movie`),
together with its provenance. Matches never create, merge or rename entities. A
search-derived TMDB ID is not written as an identity alias, so P09's resolver
never merges on it; only provider-supplied IDs are identity evidence.

- **Trusted ID first:** when the entity has exactly one provider-supplied TMDB ID
  (a P09 `external` alias), `/3/movie/{id}` is looked up. It is accepted only if
  corroborated: a known year within ±1, or, when undated, a similar title
  (localized or original). A contradicted, uncorroborated or unknown (404) ID
  falls back to search. Several distinct provider IDs are not used at all.
- **Search otherwise:** movie+year → movie → multi (filtered to movies, so a TV
  ID is never recorded as a movie), stopping at the first decisive step. The top
  candidate must score ≥ 0.6 and beat any different title by ≥ 0.05. Otherwise
  the title is **unresolved** (`low_confidence` / `ambiguous` / `no_candidates`),
  with no metadata. The former "weak match attaches art only" band is gone.
- **Provenance** (`evidence_json`, secret-free): matcher version, query and year,
  response-cache keys consulted, trusted-ID check, top-5 candidates with scores,
  thresholds and outcome. Also stored: `method`, `confidence_bp`,
  `matcher_version` and `checked_at`.
- **Progressive:** work is ordered favourites/watchlist → in progress →
  recently added and committed every 25 titles. Home refreshes as batches land.
  Decided titles leave the queue. Unresolved ones are retried after 7 days, and
  everything is re-matched when `MATCHER_VERSION` changes. Matches survive
  refreshes: the importer mirrors them onto the new rows after resolution, with
  no new TMDB calls.
- **Legacy mirror:** matched metadata is also copied to `vod_items` TMDB columns for
  surfaces that still read rows. An unresolved decision clears them.
- **Series** have no canonical entity yet: the same rules apply, with provenance
  in new `series_meta` columns.

Auth: TMDB v3 is still the current API for movie/TV data. The credential field
accepts TMDB's default **API Read Access Token** (sent as `Authorization: Bearer`)
or a legacy v3 API key, detected by shape. Request errors no longer include URLs,
so a legacy key can't leak into logs.

### Source-title parsing (naming v1, step 1)

`nova_core::clean::parse_source_title(raw, TitleKind)` is the single
deterministic parser for provider titles (`PARSER_VERSION = 1`). It returns a
`ParsedTitle`: a display title, never empty, plus the clues it removed, as
separate fields: `country_prefix`, `year`, `season`, `episode` (absolute numbering
has no season), `quality_hints` and `language_hints`. The raw name itself is
untouched.

The M3U and Xtream converters delegate to it (their private prefix/year/episode
helpers are gone), `polish_title` is its kind-neutral title, and `dedup_key`
shares its prefix rules. `search_title`/`search_series_title` are views over it.

- **Prefixes:** `FR|`, `FR |`, `|AR|`, `SRS-FR|`, `FR| US|`, `VOD | NF-FR |`, and
  `UK:` / `FR -` for known country or language codes only (so `Mission:` and
  `Up:` survive).
- **Years:** a bracketed 1900–2099 year, or a trailing bare year up to 2030 for
  movies. `Blade Runner 2049`, `1917` and `2001: A Space Odyssey` keep their
  numbers.
- **Numbering:** `S01E02`, `S01 E02`, `1x07`, and season/episode words in
  English, French, Spanish, German, Italian, Turkish, Dutch and Arabic
  (`الموسم 2 الحلقة 5`, `Folge 1234`). Missing numbering stays `None`.
- **Brackets:** tag-only groups become hints, and distinguishing groups stay in
  the title (`The Office (US)`, `(Director's Cut)`).
- **Scene dot-names** are split (`BREAKING.BAD` → `BREAKING BAD`); acronyms stay
  (`S.W.A.T.`, `H.264`).
- Capitalization is not changed. ALL-CAPS provider titles stay as-is until a
  confident TMDB name replaces them (step 5).

### Raw source evidence (naming v1, step 2)

Migration 0026 keeps provider evidence recoverable next to each row. Values fill
in on the next import or refresh.

- **Raw title:** `channels.raw_name` and `series_episodes.raw_name` are new.
  `vod_items.name` already held it.
- **Raw group label:** `vod_items.raw_group` and `series_episodes.raw_group` are
  new. `channels.raw_group` already existed.
- **`parser_version`:** the `PARSER_VERSION` that produced the parsed columns
  (`NULL` before this migration).
- **`quality_hints` / `language_hints`:** comma-separated tags the parser
  removed (`4K,HEVC` / `MULTI`). They are variant hints, never identity.
- **`series_episodes.numbering`:** `season_episode | absolute | unknown`. The
  numeric columns stay `NOT NULL` (the Xtream catalog sentinel keeps 0/0), so
  unknown numbering is stored as placeholders plus this flag.

Import fills a `SourceEvidence` value on `Channel`/`VodItem`/`SeriesEpisode`. It
is insert-only (`#[serde(skip)]`), never sent over IPC, and not loaded on reads.
`SeriesEpisode.numbering` is exposed. The UI's `episodeLabel` shows `S03E07`,
`E1234` for absolute numbering, or only the episode title when numbering is
unknown, never a fabricated S01E01.
