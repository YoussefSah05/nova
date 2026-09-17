# ADR 0006: streaming evidence and conservative storage quotas

Status: local prototype; not wired to production imports.
Date: 2026-09-17.

## Decision

Keep the v1 reader/format intact and add `snapshots::stream` with a distinct v2
namespace at `<parent>/<profile UUID>/streams-v2/<snapshot ID>.nvs2`. No implicit
migration or reinterpretation of existing encrypted files. Retain the same supplied
ProfileKey boundary; native Keychain provisioning still needs signed-app validation.

`begin(descriptor)` creates an unpublished writer. `append` feeds bounded chunks,
and `seal(receipt)` verifies the source/snapshot/scope, byte count and digest before
committing. The caller can feed its SnapshotCapture and the writer during the same
fetch; a complete raw payload is never required in memory. Use fixed 64 KiB data
frames, a one-slot channel and per-append acknowledgement. Filesystem/crypto work
runs on blocking workers. Only the final data frame may be short. Empty payloads
have no data frames and still require a final receipt.

`replay_into` streams authenticated chunks to a caller-owned sink with backpressure.
Output is **provisional until the method returns Ok with a receipt**. A missing or
invalid final frame, digest/count mismatch, sink failure or trailing data fails the
whole replay. The consumer must stage and discard output on failure; it must not
publish incremental parser output as complete evidence. Do not use an ordinary
plaintext temporary file as the sink for secret-bearing snapshots. This API does
not itself implement adapter staging or atomic catalog publication.

## V2 authenticated format

Header: `NOVASNP2` (8 bytes), then a fresh 32-byte random salt. Derive an object key
with existing HKDF-SHA256/AES-256-GCM helpers, using a distinct v2 domain plus profile
UUID and snapshot ID. Each frame is a one-byte kind (0=data, 1=receipt), four-byte
big-endian ciphertext length, ciphertext and a 16-byte authentication tag. Data
plaintext is at most 64 KiB; final receipt JSON is at most 4 KiB. Receipt fields and
raw SHA-256 semantics match v1; the ciphertext format and namespace are separate.

The frame nonce is four zero bytes followed by the sequential u64 frame index.
Associated data binds domain/profile/snapshot, index and kind. A fresh random
object salt prevents counter reuse across writes/retries; counters never restart
inside an object. The [ring contract](https://docs.rs/ring/0.17.14/ring/aead/struct.LessSafeKey.html)
requires nonce uniqueness when sealing with a key. Data order and the final marker
are authenticated; length bounds are checked before allocation. A 4 GiB hard
payload ceiling bounds the number of frames far below counter exhaustion.

Final receipt JSON and payload are encrypted; lengths, profile/snapshot filenames,
frame count and format are not concealed. No new dependency or custom cipher was
introduced. This framing protocol still needs security review before activation.

## Quotas, coordination and cancellation

StreamLimits sets payload bytes (1..4 GiB), total logical stored bytes and file slots
(1..10,000). The file-slot cap also bounds inventory scan work. Every regular file
inside streams-v2 except `.store.lock` counts, including encrypted staging orphans
and unrecognized files. Directories and symlinks fail admission instead of being
traversed or deleted. V1 files outside this namespace are not covered by v2 quotas.
Logical lengths are not filesystem allocation/reserved space; physical ENOSPC is
still an I/O failure. Filesystem metadata and the lock file are not charged.

Use the standard [File try-lock API](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock),
compiled with the pinned Rust 1.95 toolchain. An exclusive OS lock covers inventory,
staging and commit; replay holds a shared lock. Independent handles/processes using
this protocol cannot race quota admission. Lock contention returns Busy immediately;
callers choose bounded retry/backoff. All writers must use the same profile policy.
Advisory locks cannot constrain unrelated software, root or a compromised process.
The trusted-parent/private-permissions threat boundary from ADR 0004 still applies.

Check total logical bytes before each write, including headers, tags and receipt.
A short chunk can remain buffered until seal, so a successful append does not promise
that the final receipt fits. Failed quota/limit/seal checks remove only the writer's
own temporary file and preserve previous snapshots. Successful commits sync the
file, persist without clobbering, then sync the directory. A final sync error can
leave uncertain publication; replay the ID instead of overwriting it.

Dropping a writer closes its channel; the worker then removes its own staging file
before releasing the lock. This cleanup is asynchronous. A process exit leaves
encrypted staging charged on reopen; no recovery scan deletes it automatically.
A cancelled seal may finish committing. A dropped replay consumer closes its channel
so the worker releases its read lock. Callers must bound upstream/sink stalls and
hold global admission limits across profiles; no worker timeout or app-wide memory
budget is claimed here. Buffers are bounded independently of payload size, but
allocator/RSS/latency benchmarks are not yet measured.

Quota exhaustion rejects new work. **Automatic eviction is not implemented.** Safe
cleanup needs durable catalog references plus reader/playback leases and an explicit
retention policy. Refusing admission preserves previous complete/partial evidence
until those ownership rules exist. There is no destructive maintenance API in this
slice.

## Validation and integration gates

Tests were written first and failed against the missing API. Nine integration tests
cover synthetic fixture replay/reopen, a 273×64 KiB (17.0625 MiB) stream into a
hashing sink, handle contention/cancellation, payload/file/byte quotas, orphan charges,
receipt mismatch, frame corruption/reordering/truncation/trailing data, cross-key/
profile/ID copying, symlink rejection and immutable IDs. Two additional subprocess
cases exit without destructors before/after v2 commit, then verify replay visibility,
orphan accounting and OS-lock release. These are process-exit tests, not physical
power-loss, actual SIGKILL timing, throughput or RSS certification.

Next: protected normalized-record staging and lossless adapters; reference-aware
retention/deletion; signed-app Keychain lifecycle; isolated-profile app E2E. Importers
remain unchanged. Rollback removes the unconsumed v2 module/API additions; no existing
v1 file, profile database or user credential is migrated or deleted.
