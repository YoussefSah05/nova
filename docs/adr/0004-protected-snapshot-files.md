# ADR 0004: bounded encrypted snapshot files

Status: implemented prototype; not connected to production imports.
Date: 2026-09-17.

## Decision and research

Add `nova_storage::snapshots` on Unix. Persist immutable capture receipts and their
byte-exact raw payload together, encrypted under a caller-supplied profile key.
Do not put raw credentials, unencrypted metadata, or keys in SQLite or temporary
files. No existing migration, profile database or importer changes in this slice.

Use existing locked ring 0.17.14 for HKDF-SHA256 and AES-256-GCM, zeroize 1.8.2 for
owned key/plaintext buffers, and tempfile 3.27.0 for private staging. These are
already-resolved dependencies; new direct edges do not upgrade external packages.
The [ring AEAD contract](https://docs.rs/ring/0.17.14/ring/aead/struct.LessSafeKey.html)
requires unique sealing nonces per key. Each write generates a fresh 256-bit random
salt and derives a separate object key with HKDF; nonce zero is used once under
that derived key. Profile UUID, snapshot ID and a format domain are included in
both key derivation and authenticated associated data. Fresh random salt is also
required on retries. There is no persisted counter to roll back after a crash.

The [tempfile persistence contract](https://docs.rs/tempfile/3.27.0/tempfile/struct.NamedTempFile.html#method.persist_noclobber)
refuses replacement but does not itself guarantee synchronization. Write and sync
the encrypted staging file, persist without clobbering, then sync the directory.
Sync the parent after creating/opening the private profile directory. Staging is
on the same filesystem and never discoverable as a committed snapshot by replay.
A leftover staging link is possible after interruption and remains ignored.

## Format and replay

Format v1: eight-byte `NOVASNP1` magic, 32-byte salt, AES-GCM ciphertext and tag.
The authenticated plaintext is a four-byte big-endian metadata length, bounded
JSON metadata and the original payload bytes. Metadata contains typed source,
snapshot and endpoint IDs, nonzero parser revision, completeness, byte/record
counts and SHA-256 digest. JSON is a storage encoding, not a canonical identity hash.
No plaintext source IDs, receipt digests or provider fields appear in the header.
Profile/snapshot IDs remain visible in directory/file names and must be opaque.

Persist checks receipt length and digest before touching a file. Replay authenticates
before parsing, bounds the metadata and payload, checks typed IDs and parser revision,
and verifies length/digest again. Unknown format, wrong key, cross-profile/ID copying,
truncation, substitution and invalid metadata fail closed. Partial evidence stays
partial; valid empty evidence stays distinct from a missing file. Returned evidence
is not a reconstructed published generation or proof of correct record counts.

Paths derive only from a UUID and validated opaque SnapshotId. The caller owns a
trusted private parent whose ancestors cannot be replaced by an attacker. Profile
directories require private Unix permissions; files are created 0600. Replay checks
regular files and private mode, uses O_NOFOLLOW/O_NONBLOCK to reject symlinks and
avoid FIFO waits, and reads at most its bound even if a file grows concurrently.
Root/same-user compromise, hostile parent replacement and protection from a process
already holding the key are outside this prototype's threat model.

## Resource, cancellation and recovery limits

A configured limit must be 1..16 MiB per payload. Metadata is capped at 4 KiB.
Buffers are bounded but this prototype holds a whole payload and ciphertext during
persistence/replay. It is unsuitable for 200k-record catalogs until a separately
versioned chunked format and streaming API exist. One worker per shared handle is
admitted at a time; separate handles can race safely at commit. Caller-owned queued
payloads and separately opened stores still need an application-wide memory budget.
Filesystem and cryptographic work execute on blocking workers, never the async loop.

Dropping a persist future does not cancel an already-running blocking worker. An
I/O failure after publication can leave a committed file. The caller must replay
that ID to resolve uncertainty; retries never overwrite evidence. Process-restart
atomicity is tested, not physical power-loss durability or all filesystems.

No automatic listing, retention, garbage collection or deletion exists. Abandoned
staging remains encrypted and is not removed indiscriminately. Disk usage can grow
across snapshots; aggregate quotas and lease-aware cleanup are integration gates.
Zeroizing owned buffers does not guarantee erasure of caller copies, allocator,
compiler, cryptographic key schedules or OS memory. No diagnostic exporter exists.

## Validation and next boundary

Tests were written first and failed to compile against the absent module. Eight
integration tests cover fixture roundtrip after reopen, partial/empty receipts,
receipt mismatch, size bounds, concurrent and duplicate commits, authentication,
corruption/truncation, orphans, private permissions and symlink rejection. Two
subprocess tests abruptly exit without destructors after staging sync and after
commit sync; reopen observes no published snapshot or complete evidence respectively.
The ignored subprocess entry is invoked by both tests, not skipped validation.

Next: Keychain provider and key lifecycle tests, aggregate retention/streaming, then
normalized SourceRecord persistence and lossless replay adapters. Only then connect
to isolated-profile app flows. No live credentials, real provider or existing user
data were used. Rollback removes unconsumed module/dependency edges; no schema rollback
or destructive cleanup is needed. Format v1 must not be silently reinterpreted.
