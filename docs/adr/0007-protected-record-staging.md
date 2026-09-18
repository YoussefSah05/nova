# ADR 0007: protected record archives and a pure Xtream object adapter

Status: local prototype; not connected to production imports or publication.
Date: 2026-09-18.

## Decision and scope

Add `nova-catalog-ingest`, depending on domain contracts and protected storage.
The desktop has no dependency on it. Keep provider-specific extraction out of core
and leave the existing M3U/Xtream network importers unchanged. This increment covers
Xtream live/VOD/series **objects**, not HTTP arrays, episode details or M3U parsing.

EvidenceRecord pairs one bounded SourceRecord with the original JSON-object bytes.
It has no Debug/Serialize implementation and is not an IPC DTO. Caller-owned IDs
identify distinct occurrences; an upstream ID never becomes a canonical ID. The
caller also supplies an opaque StreamReference. This does not create a resolver or
prove that a protected locator exists, is authorized or playable.

Use an explicit endpoint scope for kind and upstream namespace. Series objects
remain series containers without invented episode-zero data; channel numbers and
multilingual titles remain untouched. Missing/null provider IDs remain unknown.
Valid string IDs retain their spelling; numeric IDs must be nonnegative integers.
Malformed known fields fail with bounded typed errors that do not echo provider text.

Preserve at most 128 KiB of raw object bytes and 256 top-level fields. Keep Serde's
recursion limit enabled. A [custom map visitor](https://serde.rs/deserialize-map.html)
rejects duplicate top-level keys rather than silently replacing them. Unknown nested
fields remain in raw evidence; they are not promoted to normalized metadata. Raw
bytes retain whitespace, numeric spellings, escapes and unknown fields exactly for
accepted objects. Parsed Value trees are not used to reconstruct those bytes.

The normalized attribute cache currently allowlists category_id, container_extension,
year and rating scalars, encoded with their JSON scalar types. Unknown fields and
transport fields such as direct_source remain only in protected raw evidence. The
existing metadata fingerprint therefore does not reflect unknown raw fields; the
raw payload digest does. Treat normalized metadata as a versioned extraction cache,
not complete provider semantics or a canonical identity decision.

## Encrypted record staging

RecordStage writes bounded line-oriented records through the existing encrypted v2
stream writer. A header binds archive/adapter revisions, original source/snapshot/
scope/parser revision, raw byte count/digest and raw completeness. Each record carries
all SourceRecord fields and original object bytes. A mandatory footer carries the
staged occurrence count. The entire archive is authenticated by the storage receipt.

The archive has a distinct artifact ID and reserved `record-stage-v1` scope. It is a
derived byte artifact, not another provider response. Its transport receipt has zero
provider records; its encrypted footer owns the staged-record count. Its Complete
status means the archive finished, while raw input completeness stays in the header.
A complete archive derived from partial input must never be treated as a complete
provider scope. Replay requires the expected original receipt and rejects a mismatch.

Only one record and its encoding are buffered at a time, plus a bounded occurrence-ID
set. Limits: 2 MiB per encoded line, configured archive bytes up to 4 GiB, configured
record count up to 100,000. The ID set is count-bounded, not constant memory. Large
catalogs above this cap require a separately reviewed uniqueness/index strategy.
The caller controls callback memory; no RSS or throughput benchmark is claimed.

Repeated upstream IDs are allowed and preserved. Repeated SourceRecordId values,
foreign source/snapshot ownership, record/byte quotas and storage errors terminate
the stage. An explicit stream-writer abort closes/removes its own unpublished staging
file before releasing the lock; failure never publishes an artifact. Cleanup errors
can leave an orphan charged by the store, as with v2 process interruption.

Replay bounds lines before deserialization, revalidates domain field limits and raw
object structure, checks header/order/ownership/uniqueness/footer, then requires the
final encrypted receipt. Callbacks see **provisional evidence**: callers must not
publish it until replay returns Ok. A late failure invalidates all callback output.
No catalog database transaction or publication gate is implemented here.

## Provenance and integration limits

The raw receipt is a caller-supplied binding assertion; it does not by itself prove
that every attached object is a slice of the referenced raw response. The network
array framer must provide and verify byte offsets before production activation.
The whole raw response must remain separately retained for replay, including array
structure and response-level evidence. This archive does not replace that source.

Raw replay and derived staging need separate configured store roots if performed
concurrently, because a v2 writer holds an exclusive lock over its store. Their
quotas are separate; a future coordinator must enforce the aggregate profile budget,
reference retention and publication across both stores. Neither input receipt alone
nor a staged archive certifies durable raw retention or canonical compiler success.

No network calls, real provider, credentials, profile DB, UI, grouping, streaming URL
construction, locator persistence, automatic eviction or existing import changes
are part of this slice. The import-validation skill's real-provider probe and app
check remain mandatory before activating a replacement import path. Native Keychain
provisioning is still blocked by the previously recorded signing-entitlement gate.

## Validation and next

Tests were written first and failed against the missing API. Seven integration tests
cover exact raw retention/unknown fields, explicit kinds, malformed/ambiguous inputs,
five synthetic VOD occurrences including duplicate upstream ID 201, encrypted reopen
with identical metadata fingerprints/locators/IDs, foreign/duplicate occurrence
rejection, terminal limits, wrong raw receipts, tiny quotas and empty archives.
Two unit tests cover line/order/footer framing and bounded accumulation.

The small fixture array uses borrowed serde_json RawValue only in tests, with the
raw_value feature on the dev dependency. Production code does not collect an entire
provider array. Cargo.lock adds only the local crate; no external package versions
change. Existing v1/v2 storage tests remain part of regression validation.

Next: bounded streaming array framing with offsets, raw capture and record staging
coordination, lazy episode and M3U adapters, then isolated-profile app validation and
publication. Rollback removes the unconsumed crate and explicit abort addition;
no schema or user-data migration is needed.
