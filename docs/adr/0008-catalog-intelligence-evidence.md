# ADR 0008: Catalog intelligence evidence boundary

Status: accepted for an additive, unconnected first increment, 2026-09-18.

## Decision

Catalog intelligence is required by the revised implementation request. Model
selection is replaceable; no additional model has been selected or installed.
The first executable boundary is `nova_core::catalog::intelligence::assess`.
It does not clean titles, run inference, research the web, merge entities or publish.

The versioned protocol accepts a bounded JSON proposal tied to a source occurrence
and snapshot. Typed Serde deserialization rejects unknown fields and enum values.
There are no URL, external-ID, source-authority or database-write fields. Bounds:
32 KiB response, 64 candidates/evidence assertions, eight citations per candidate,
256-byte names, 16 sorted unique language codes. Country/language membership is
validated against caller-owned registries; a production registry is still needed.

Nova's trusted adapter registers each fact assertion, its authority, retrieval and
expiry timestamps and an opaque protected provenance reference. This registration
API is not exposed to a model. The caller must independently verify fact extraction,
subject association and source authority. Referencing a web page does not establish
that it supports a claim. This increment does not perform that verification itself.

Model candidates must exactly match cited assertions. Every registered assertion
for a proposed field must be represented, so a model cannot hide a contradiction.
The highest fresh authority wins; equal-authority conflicting values abstain.
Lower-priority and stale alternatives remain available. Inference/provider facts
remain provisional. Model confidence is retained as an uncalibrated integer score
in basis points and never decides authority or entity identity. Missing fields are
unknown; malformed responses fail atomically without partially accepted fields.

Observed quality and audio language require playback evidence and remain separate
from provider quality hints and channel languages. `Supported` is a field assessment,
not a resolved canonical entity or permission to merge. Identity/cannot-link policy
and naming semantics still require their own validation. Raw Unicode names survive;
no new regex-based cleaning rules were added.

## Research and alternatives

Reviewed [Serde container attributes](https://serde.rs/container-attrs.html) for
strict unknown-field handling, [JSON Schema object rules](https://json-schema.org/understanding-json-schema/reference/object)
for the need to constrain additional properties, and the
[W3C provenance overview](https://www.w3.org/TR/prov-overview/) for separating facts,
derivation and responsible agents. The implementation uses typed Serde validation;
it does not claim an exported JSON Schema or PROV conformance.

Rejected free-form cleanup output, model-assigned source trust, global confidence
thresholds and embedding-only identity merges. An inference proposal may remain
provisional pending trusted corroboration; this boundary intentionally cannot
decide whether a novel interpretation is true.

## Next increments and validation gates

1. Reviewed messy multilingual corpus and identity-negative cases; benchmark
   replaceable CatalogReasoner/SemanticMatcher backends and pin chosen artifacts.
2. Context builder with credential isolation, cache-first orchestration and bounded
   workers; persistent per-fact research cache and verified knowledge/alias bindings.
3. Research adapters, deadlines, retry/concurrency/global budgets and verified artwork.
4. Complete raw framing/adapter integration, canonical identity/variant compiler,
   atomic publication and shadow comparison before migrating frontend reads.

Tests cover forged/omitted citations, authority conflicts, expiry, context mismatch,
strict schema, bounds, multilingual preservation and observed-vs-hinted properties.
No model-quality, real-provider, app E2E or large-catalog performance claim follows
from these tests. Rollback removes the unconsumed module; no schema/data migration.
