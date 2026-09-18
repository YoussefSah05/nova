# ADR 0009: Bounded local catalog reasoner

Status: accepted for an additive, unconnected increment, 2026-09-18.

## Decision

`nova-catalog-reasoner` implements a replaceable `CatalogReasoner` interface and
a loopback HTTP adapter using the evaluated v2 prompt/schema. Qwen3.5-4B Q4_K_M
remains the development reference. The desktop does not depend on this crate.

Callers supply an allowlisted draft and up to 16 candidates. The immutable request
binds source-record/snapshot IDs and caller-owned cannot-link constraints into an
input fingerprint. Scope IDs and cannot-link constraints stay outside model messages;
candidate IDs are necessarily included. No locator or raw-attribute field is accepted.
This is not a credential scrubber: text can still contain secrets, so a vetted context
builder remains required before provider data is used. Country checks here validate
two-letter uppercase syntax only, not membership in an authoritative registry.

Transport is fixed to `127.0.0.1`, with proxies and redirects disabled. Limits are
16 KiB serialized input, 32 KiB request, 64 KiB response, 512 output tokens and a
configurable deadline of at most 60 seconds. Both declared-length and chunked
responses are bounded. Each adapter instance permits one active client request;
additional requests return Busy instead of accumulating a queue. Dropping the future
releases its permit, but does not prove GPU generation stopped in the external runtime.
A runtime owner must enforce process-wide budgets and cancellation.

Typed parsing rejects incomplete output, duplicate/extra model fields, fabricated
candidate IDs and name/type mismatches. Cannot-link constraints and explicit EPG,
country, year, season, episode or content-kind contradictions reject a selection.
These checks are conservative constraints, not a complete identity resolver. Missing
discriminators still permit ambiguity; no selection authorizes an identity merge.

Model name/type/quality suggestions become only `Authority::Inference` assertions.
Quality hints never become observed playback quality. The bridge scopes each proposal
to its input and supplies an expiry of 1–86400 seconds, with overflow rejection.
Core assessment therefore remains provisional or unknown after expiry. The bridge
assesses only its own inference bundle; combining it with other authorities belongs
to the future orchestrator. It cannot write a database, merge entities or publish.

Results retain input/protocol fingerprints and caller-declared model/runtime artifact
digests. Those digests are not process attestation; the development runtime digest is
the distribution archive digest. The caller must persist a protected derivation and
its provenance reference before publication. This increment does not persist it.

## Validation

Nine Rust boundary tests cover actual loopback HTTP, output validation, conflicts,
limits (including chunked responses), expiry, fingerprints, busy/deadline handling
and caller cancellation. Ten Node harness tests include prompt/schema parity between
the evaluation harness and Rust's frozen protocol. Workspace tests and Clippy pass.

The `local_probe` example ran against the pinned llama.cpp b10964 server and Qwen
4B model with the first synthetic development case. It selected the expected candidate
and produced three provisional fields. Model and runtime-archive SHA-256 values were
rechecked against the manifest. The owned runtime was stopped afterward. This is
transport/bridge evidence, not another model-quality evaluation or app E2E result.

## Remaining work and rollback

Next: protected context/provenance persistence and cache coordination; process/model
ownership and attestation; global workload budgets; reviewed corpus and realistic
catalog throughput; playback/caption coexistence; then source/compiler integration
and shadow import/app E2E gates. Existing importer behavior remains unchanged.

Rollback removes this unused crate and its workspace entry. No data migration or
model packaging change is introduced, and no external dependency version was upgraded.
