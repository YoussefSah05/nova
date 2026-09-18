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
