# M26 real-profile qualification

Date: 2026-10-02 (interruption handling 2026-10-04). Status: **PARTIAL — known
schema-21 conversion qualified; copy recovery is available in the profile picker, with
reviewed interruption handling; unknown schema-6 lineage remains unsupported; native
recovery not yet exercised.**

## Initial qualification contract

Reuse `init_profile_db`, the SQLx migration ledger, existing stabilization fixtures,
and the opt-in `crates/storage/tests/real_profile_upgrade.rs` harness. Qualify only
isolated copies of real Nova profiles. Originals remain untouched; output contains
schema versions, counts, timings and failure categories, never profile content.

The missing evidence is an older real profile successfully traversing the current
migration chain while retaining source records, user flags, valid watch history,
captions and settings, followed by an idempotent reopen. A profile already at head
can provide reopen evidence only.

Changes are limited to the qualification harness and documentation. Before invoking
the migration runner, verify every applied migration checksum against this build.
Compare in-memory, byte-safe fingerprints of stable source/user columns before and
after initialization and reopen. Exclude migration 20's deliberate orphan-history
cleanup and transcript media-ID detachment, and mutable enrichment/canonical keys.

No production schema, IPC, UI, AI/resource scheduling or application lifecycle change.
The harness snapshots with SQLite `VACUUM INTO`; the source connection is read-only.
No concurrent application writes are performed by qualification. No checksum
overrides, applied SQL edits, speculative compatibility bridge or later milestone work.

Validation: focused regressions for lineage refusal, unchanged-count content changes
(including embedded NUL), and valid-history preservation versus orphan cleanup;
existing upgrade fixtures; storage suite; Clippy; actual isolated profile probes;
independent review and fixes. Reopen timings exclude snapshot/fingerprint work and
are not application startup or scale certification.

## Observed real profiles

| Starting schema | Result | Meaning |
| --- | --- | --- |
| 6 | Refused: migration 6 checksum differs | No matching migration SQL found in available Git history; provenance unresolved. |
| 21 | Normal startup refused; explicit offline conversion now passes on a real copy | Applied migrations 20/21 match the unmerged `feat/local-llm-about` branch (`llm_about` / `tmdb_credits`), whose numbering conflicts with main. |
| 42 | Initialization and reopen pass | Already at current schema: zero pending migrations, so no older-profile upgrade qualification. |

The schema-42 sample contains 28,597 movies, 5,525 episodes, 8,658 channels,
33,787 canonical entities and 16 history rows (four movie resume rows). Integrity is
`ok`, foreign-key violations zero, movie mapping losses zero. It has no favorites,
watchlist entries or transcript segments, so preservation of nonempty examples of
those states still depends on synthetic fixtures, not this real sample.

For the schema-42 source, read-only SQLite opening required unavailable WAL shared
memory files. A private temporary file copy was therefore made only after confirming
no WAL existed, with source size/mtime unchanged and source/copy SHA-256 equal. The
harness then made its own SQLite snapshot. These copies are disposable and are not
repository artifacts. Older-profile attempts also migrated only disposable copies.

## Known-lineage conversion

### Follow-up contract: offline known-lineage conversion

Add an explicit `nova-storage` example CLI that accepts a source and a new output
path. It operates inside a private temporary directory beside the output, refuses
existing output paths, and never installs the result into the running application.
Recognize only the complete, successful 1–21 ledger whose 1–19 checksums match main
and whose 20/21 checksums match the recovered local-LLM branch SQL. Unknown, partial,
dirty or newer histories remain refused.

On the isolated copy, archive the entire original ledger, remove only its conflicting
20/21 entries from the active ledger, and run the actual current 20–42 migrations
through `init_profile_db`. This is an explicit lineage conversion: it does not claim
the old 20/21 SQL was the current SQL and never changes an applied checksum. Preserve
the old branch's extra tables/columns and compare source/user-state fingerprints.
Check integrity, canonical backfills and reopen before publishing the completed
database without overwriting any destination. Failed conversions publish nothing.

Reuse the existing qualification checks in a shared offline helper. Include fixtures
with favorite/watchlist flags, progress, captions, transcripts and branch-only data;
test unknown checksums, dirty/incomplete histories, malformed schema, existing output,
source immutability and idempotent reopen. Then qualify the real known-branch copy.
Runtime SQL here is limited to the offline, multi-schema migration utility; normal
repositories and app startup are unchanged. No provider/model calls or frontend work.

### Implementation and results

The offline converter is implemented in
[`upgrade_legacy_profile.rs`](../../crates/storage/examples/upgrade_legacy_profile.rs),
with [usage and operational boundaries](../../crates/storage/examples/README.md).
Recovered SQL is byte-for-byte historical material outside the normal migration
directory. Only a complete matching ledger and exact recovered schema are accepted.
All destination SQLite sidecars, including dangling symlinks, are refused before
snapshotting and before publication. The caller must use an exclusive private output
directory; the tool does not coordinate with another process targeting that directory.

The real legacy copy traversed current migrations 20–42 and reopened successfully:
28,070 movies, 5,222 episodes, 9,104 channels, 47 history rows, 401 transcript segments,
eight legacy About rows and 32,571 canonical entities. The converter verifies stable
source/user data, original-ledger archival, retained branch-only data, integrity,
foreign keys, movie/series mappings and flags, latest movie/episode progress, and
canonical-state stability on reopen. The disposable output was removed by the test;
no converted database was installed and the original profile remains unchanged.

Fixtures include nonempty favorites, watchlists, progress, captions, translations and
branch-only data. Regressions cover invalid histories/schema, WAL snapshot contents,
existing outputs and sidecars, semantic state loss, duplicate bindings with newer
history, and exact fingerprint encoding of embedded NUL and adjacent REAL values.
Independent review found sidecar-publication and semantic-validation gaps; both were
fixed and re-reviewed. The original migration SQL and normal app startup are unchanged.

The baseline deliberately refuses any change to legacy flag values. Canonical merging
can legitimately promote duplicate-row flags, so some matching schema-21 datasets may
be conservatively refused. This tool does not promise to convert every such dataset.

## App recovery

### Follow-up contract: recover as a separate profile

Expose the qualified conversion through profile-picker recovery after an opening
failure. Inspect compatibility read-only; offer “Recover a copy” only for the exact
known lineage. Unknown histories receive a plain unsupported message. Reuse the
existing converter, preservation checks, profile-open mutex and profile repository.

Recovery creates a new UUID and private directory, validates/converts the database,
then registers the new profile in app.db. The existing profile remains the original
backup; no in-place replacement, source deletion or automatic rollback rewrite.
Recovering does not select the new profile or start its jobs. The user explicitly
opens the completed copy. Failed registration removes only the newly owned output;
an abrupt process exit can leave an unregistered private directory, never an active
half-converted profile. Existing profile IDs/paths must be validated before access.

Move the shared converter into the storage library without changing its checks.
Keep historical-schema introspection and migration SQL at this boundary; ordinary
profile registration uses compile-time checked queries. No migration file changes.
Add typed IPC, a focused accessible recovery dialog and pending/error/success states.
Test successful registration and reopen, failed conversion/registration, original
preservation, invalid IDs, unsupported recovery, double-submit guards and dialog focus.
Validate with isolated synthetic data; never install a real recovered profile during
development. No later architecture milestones are part of this slice.

### Implementation and validation

The converter and shared qualification checks now live in
`crates/storage/src/profile_recovery/`; the CLI reuses the same implementation.
The profile picker opens an accessible recovery dialog after an opening failure.
The typed `inspect_profile_recovery` command recognizes supported data read-only;
`recover_profile_copy` repeats validation on a snapshot, publishes into a new UUID
folder and registers a `(Recovered)` profile only after success. Normal profile
migrations still refuse divergent originals. Originals remain the backup and no
in-place replacement or automatic deletion is provided.

Recovery and profile opens/deletes share the open mutex. Snapshot/fingerprint work
runs on a blocking worker; it does not activate a profile or start provider/AI jobs.
Selection is a separate user action. Failed initialization explicitly closes its pool.
Only successfully opened profiles update their last-used timestamp. IDs must be
canonical UUIDs in registered profiles; recovery refuses symlinked source paths.

Validation: **293 storage/desktop Rust tests passed, six environment/opt-in tests
ignored; 150 frontend tests passed.** Typecheck, frontend lint, storage/desktop Clippy
with warnings denied and frontend production build pass. An EPG localhost test was
initially blocked by the sandbox; it passed in the permitted rerun. Synthetic tests
cover successful copy registration/reopen, failed registration cleanup, unsupported
and invalid IDs, original-file preservation and no automatic selection. Dialog tests
cover unsupported states, double submission, retry, focus trapping and return focus.
Independent review's focus-restoration finding was fixed and re-reviewed.

Browser visual inspection exercised the actual picker/dialog components with isolated
mock IPC, including recovery progress and success at 1280×720. The preview was removed.
No real profile was installed; native end-to-end IPC recovery and interruption during
process termination have not been exercised in a running Tauri app. Abrupt termination
before app registration may leave a private unregistered directory; it cannot appear
as an active profile. The pre-existing Vite chunk-size warning remains.

## Remaining work

### Follow-up contract: interruption and native qualification

Give app copy recovery a versioned staging receipt and a cross-process filesystem
lock. Before retrying, remove only unregistered outputs whose matching receipt proves
they belong to that interrupted attempt; preserve registered profiles, source paths,
unknown directories and symlinks. The completed original remains untouched. Keep the
profile-open lock owned by the background worker even when its IPC caller disappears.
Use existing app metadata (no schema changes) to distinguish committed registration.

Add process-kill tests around conversion/publication/registration, lock-contention and
receipt/path rejection tests. Provide a synthetic fixture for an isolated debug app;
exercise the actual recovery IPC and profile picker without accessing real profiles,
models or provider accounts. Record remaining unknown-lineage and duplicate-state
limitations honestly. No later milestone work.

### Interruption implementation and audit — 2026-10-04

Implemented in `crates/storage/src/profile_recovery/interruption.rs` and
`recover_as_profile`. A nonblocking, non-symlink, owner-only `flock` on
`profiles/.nova-profile-recovery.lock` serializes recovery and cleanup across
processes; only contention reports "busy". Each attempt writes a versioned receipt
(source and target UUIDs) into `.nova-recovery-v1-<target>-*/`, converts there,
assembles the complete target directory (receipt plus hard-linked database) inside
staging, and publishes it with a single rename after a vacancy check. App
registration is the commit point. The receipt is then released (removed and the
directory synced). Opening any profile releases it again, so an opened profile
never carries one.

Cleanup runs under the lock before each new recovery, or through
`cleanup_interrupted_profiles`. For a staging directory with a valid receipt whose
name matches its target, cleanup:

- releases the target's receipt if the target is registered;
- moves an unregistered target with the identical receipt into staging, so a crash
  mid-removal leaves no partial directory under a UUID;
- leaves any other entry at the target name untouched.

It then removes the staging directory. A staging directory that is empty, or holds
only an unparseable marker (a crash before the receipt was complete), is removed.
Everything else is retained: unknown content, complete but invalid receipts, and
symlinks. Each entry is reconciled independently, so one undeletable leftover is
retained with a warning and does not block later recoveries.
Staging directories are removed with their receipt last, so an interrupted or
failed removal can still be finished by a later cleanup. The staging directory is
not removed on drop: a cancelled future leaves a receipt for the next cleanup.
A failed registration moves the published target back into staging. If that move
fails, staging is kept as the target's proof. The desktop command moves the owned
profile-open guard into its blocking worker. Opening a profile warns, rather than
failing, if its receipt cannot be released.

**Crash windows (verified by killing a child process):**

| Kill point | What stays on disk after the kill | What the next cleanup does |
| --- | --- | --- |
| Staging directory created, receipt incomplete | Staging directory that is empty or holds a partial marker | Removes it |
| Receipt written, conversion running, or target being assembled | Staging directory with a receipt | Removes it |
| Target published, not yet registered | Staging directory, plus a target carrying the same receipt | Removes both |
| Registered, receipt not yet released | Staging directory, plus a registered target that still has its receipt | Removes the target's receipt and the staging directory; keeps the profile |

Cancelling the future (dropping it) at the converted, published and registered points
has the same results.

**Bounded limits:**

- Cleanup runs only when recovery is retried (or through the helper), not at app
  startup. Leftover staging directories hold a full database copy until then.
- The app.db uses `synchronous=NORMAL`, so a power loss can roll back a committed
  registration while the published directory survives. Two cases follow:
  - If its receipt release reached disk, the copy remains as an unreferenced
    directory.
  - Otherwise cleanup may remove that copy. That copy was never opened, so nothing
    the user did in it is lost, and the original is untouched.
- A registered copy that was deleted without ever being opened can also be removed
  by cleanup. This is the same never-opened, converted copy; the original remains.
- If a library caller cancels during `register`, SQLite may still commit after the
  lock is released. A concurrent cleanup could then withdraw the directory, leaving
  a profile row that opens as empty. The desktop command never cancels the future.
- Publication checks vacancy and then renames. That rename could replace an empty
  directory created in between under the same fresh, unregistered UUID. There is no
  exclusive-rename primitive in use.
- A failure to read the profiles directory or app.db still aborts that recovery.
  Such failures are transient; no partial state is created.
- A profile that was opened and then deleted can be removed only if three releases
  of its receipt all fail: the one after registration, the staging removal that
  keeps the marker, and the one at open. Reviewed and accepted as documented.
- `delete_profile` keeps profile directories on disk by design. Once a recovered
  profile has been opened, cleanup never touches it, whether it is registered or
  deleted.

**Validation:**

- **Tests:** 298 storage/desktop Rust tests pass, with seven opt-in, environment or
  child-helper tests ignored. Clippy with `-D warnings`, `cargo fmt --check` and
  `git diff --check` pass. Frontend typecheck, lint, 150 tests and the production
  build pass.
- **Independent review:** found three issues; a re-review then found three more:
  - a used-then-deleted recovered copy could be removed;
  - one stuck leftover blocked all later recoveries;
  - there were leak windows during creation, removal and cancellation;
  - lock errors were misreported as "busy";
  - removal could leave a staging directory without its receipt;
  - a failed move-back after a registration failure orphaned the target;
  - a receipt-release failure blocked profile opens.

  All were fixed and covered by regressions. The remaining plausible races are
  bounded above.
- **Not yet done:** native app validation, with a separate debug bundle ID and a
  synthetic `NOVA_VALIDATION_DIR`.

Recover the schema-6 migration provenance; it remains refused. Address intentional
flag promotion for duplicate rows before broadening accepted datasets. Qualify native
end-to-end recovery and interrupted-process cleanup. M26 remains partial; no later
milestone started.
