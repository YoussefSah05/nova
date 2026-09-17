# ADR 0003: immutable endpoint receipts and versioned fingerprints

Status: implemented locally; not integrated into main or the production importer.
Date: 2026-09-17.

## Problem and evidence

A partial provider response must not erase a previously complete endpoint scope.
Capture completion is different from successful catalog validation/publication.
Stable persisted fingerprints also cannot depend on unspecified Rust hash behavior.

The [Rust DefaultHasher contract](https://doc.rust-lang.org/std/hash/struct.DefaultHasher.html)
explicitly warns that hashes are not stable across releases. The pinned
[sha2 0.10.9 API](https://docs.rs/sha2/0.10.9/sha2/) supports incremental SHA-256.
Use the already-resolved workspace sha2 dependency; do not upgrade to a newer API.

## Decision

`SnapshotCapture` tracks Receiving → Complete/Partial/Failed/Cancelled. Terminal
captures cannot accept bytes/records or seal again. Byte/record quota violations
and foreign-record ownership errors fail closed. Successful sealing returns an
immutable SourceSnapshot receipt containing source/snapshot/scope, parser revision,
counts and payload digest. It does not write files or publish a catalog.

Compiler run states remain separate from immutable source evidence: recompiling
retained bytes must not mutate whether the original endpoint response was complete.
This refines the blueprint's combined lifecycle into capture and later compilation
responsibilities. Publication/validation will have their own tested state machine.

Scope selection requires matching source and endpoint plus a complete previous
receipt. Complete candidates supersede it; partial candidates retain it as stale.
With no previous complete input, partial evidence is explicitly PartialFirstImport,
not an authoritative full replacement. A failed/cancelled capture cannot produce
a new receipt at all. The importer must keep its previous input on those errors.

Use standard SHA-256 for exact retained payload bytes. Use domain-separated framed
v1 SHA-256 for record metadata and locator/credential revisions. Fields use u64
big-endian byte lengths; options have presence tags; attribute order is BTreeMap
order. No Unicode normalization occurs. Record/snapshot IDs and stream references
are excluded from metadata, while unknown attributes are conservatively included.
Hash inputs and output digests are not ordinary diagnostics. Hashing is not encryption.

## Alternatives, limits and validation

Reject concatenated strings (ambiguous boundaries), Debug/JSON serialization as an
implicit hash format, title hashes as identity, and silently publishing partial
refreshes as complete. A whole retained payload in memory would defeat streaming;
capture keeps fixed-size hash state and counters only.

The adapter must report EOF/completeness truthfully, supply retained bytes exactly
once, bound transport parsing and separate transport secrets from metadata. The
future staging writer must enforce record uniqueness and durable receipt/payload
agreement. A count alone cannot prove those properties. Profile isolation and
destructive-collapse checks remain mandatory at publication. Snapshot IDs must be
uniquely allocated; this in-memory helper does not maintain an identity registry.

Tests cover independent fixed hash vectors, chunking, field boundaries, options,
attribute order, all metadata fields, source/scope ownership, terminal transitions,
empty versus partial inputs and fail-closed quotas. No real provider, profile DB or
runtime path changes. Rollback removes unconsumed modules without a data migration.
