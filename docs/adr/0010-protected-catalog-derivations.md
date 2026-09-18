# ADR 0010: Protected catalog derivations and exact-request reuse

Status: accepted for an additive, unconnected increment, 2026-09-18.

## Decision

The Unix-only `nova_catalog_reasoner::protected` module stores local interpretations
using the existing v2 encrypted streaming snapshot store. It reuses profile-bound
authentication, private files, immutable commits and byte/file quotas. Its trusted
owner supplies a dedicated store namespace, key, source scope and opaque artifact IDs.
There are no new SQL schemas or cryptographic formats.

Each derivation retains the exact framed input (record/snapshot IDs, allowlisted
context, candidates and cannot-link constraints), frozen prompt/schema, their
fingerprints, declared model/runtime digests, validated model output and original
creation/expiry timestamps. These contents remain encrypted at rest. Digests still
declare artifacts; they do not attest the running inference process. Text vetting
is still the caller's responsibility, and complete in-memory zeroization is not claimed.

The format version covers derivation semantics and must change when incompatible
resolver policy changes require invalidation. Replay also checks the current full
context and protocol and the requested model/runtime declarations. It reuses the
same candidate/constraint validator as live inference. Encryption proves storage
integrity, not that a model interpretation is true.

Replay accumulates at most 256 KiB and returns no result before the final encrypted
receipt and archive validation succeed. Wrong keys, truncated/corrupt files, invalid
payloads and wrong source ownership are errors. Missing, expired, future-dated or
changed-input/protocol/model/runtime entries are misses. Abstention is cacheable.
The derived archive contains zero provider records; its payload represents one
derivation, rather than changing source-snapshot record-count semantics.

`ProtectedDerivations::resolve` checks a caller-supplied previous artifact first.
A valid hit needs no runtime connection. On a miss it runs local inference and
commits the derivation before returning. A storage failure therefore produces no
new reusable result. The caller indexes a new artifact only after success. This
increment does not implement that durable index, eviction or cross-request deduplication.
Expired artifacts are retained and count against quotas; replacement uses a fresh ID.

`CachedDerivation` exposes provisional assessment with the original timestamps,
so reuse cannot refresh the evidence's age. The request-start timestamp supplied
by the caller is used for a new result; inference latency consumes its TTL. The
caller must supply an accurate clock. Assessing at expiry returns unknown facts.
The opaque provenance reference names the successfully committed derivation in
the caller-owned profile/store namespace. No canonical publication is authorized.

## Validation and limits

Nine added tests cover encrypted reopening, original expiry, changed scope/context/
constraints/artifacts, negative reuse, wrong keys and truncated final receipts,
quota/overwrite/TTL errors, bounded replay, runtime-free hits, synthetic HTTP miss
and commit coordination, and authenticated but invalid archive payloads. Existing
reasoner boundary tests still pass after sharing live/replay validation.

This does not validate native Keychain entitlements, real-provider contexts, model
memory, cancellation of remote generation, runtime attestation, or app E2E. Existing
snapshot-store interruption tests remain the durability evidence for the reused
storage layer; no new crash test of a durable index is claimed because none exists.

## Next and rollback

Next: vetted context construction, durable artifact indexing and retention, runtime
ownership/global budgets, then compiler/source integration and shadow/app E2E gates.
The desktop importer remains unchanged. Rollback removes this unused module and its
crate-local dependencies; existing source archives and user databases are unaffected.
