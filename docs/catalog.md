# Catalog migration: implemented contracts

The current serving catalog remains the existing provider-row implementation.
`nova_core::catalog` is an additive, unconsumed domain module. It does not yet
implement a Catalog Compiler, canonical database, snapshot store or identity resolver.

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

Next steps: durable SourceSnapshot lifecycle/completeness and fingerprint contracts,
then fixture-backed adapters and storage staging. Actual import/publication changes
require regression, migration and real-app E2E evidence before activation. See the
production blueprint for the proposed full architecture; do not confuse it with
the implementation described here.
