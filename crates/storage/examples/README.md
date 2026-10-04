# Offline legacy profile conversion

This utility supports only the recovered `local-llm-about` schema-21 lineage.
Unknown checksums and any changed schema are refused. The CLI does not install a converted profile, contact providers or run models.
The profile picker now uses the same converter for explicit recovery into a separate
profile; normal startup still refuses divergent originals. Shared implementation:
`crates/storage/src/profile_recovery/`.

From the repository root:

```sh
SQLX_OFFLINE=true cargo run -p nova-storage --example upgrade_legacy_profile -- \
  /absolute/path/to/source/profile.db \
  /absolute/path/to/private-unused-output/converted.db
```

Create the private output directory first, with access limited to your account.
Keep that directory exclusive to this conversion; do not open the output with another
process while conversion is running. The output main path and its SQLite `-wal`,
`-shm`, and `-journal` paths must all be absent. The tool checks this both before
snapshotting and immediately before publication, and never removes existing files.
The source opens read-only and `VACUUM INTO` includes committed WAL data. If SQLite
cannot open the original read-only, stop and obtain a consistent backup; do not
force immutable mode on a database with a live WAL.

Work happens inside a private temporary directory next to the output. The converter
checks the historical ledger and schema, archives the ledger, runs current migrations
and backfills, checks preserved data and canonical user state, and reopens the result.
Only a completed database is published, atomically without replacing an existing main
file, with permissions 0600 on Unix. Errors print categories, not profile content.

Qualification:

```sh
SQLX_OFFLINE=true cargo test -p nova-storage --test legacy_profile_conversion
NOVA_LEGACY_UPGRADE_DB=/absolute/path/to/source/profile.db SQLX_OFFLINE=true \
  cargo test -p nova-storage --test legacy_profile_conversion \
    real_known_branch_converts_on_a_temporary_copy -- --ignored --nocapture
```

The opt-in test converts only a temporary copy and removes it afterward. It prints
counts and elapsed time only. A successful conversion does not authorize replacing
an installed profile. Normal startup still refuses the divergent original. The old
schema-6 checksum with unknown provenance is intentionally unsupported.
