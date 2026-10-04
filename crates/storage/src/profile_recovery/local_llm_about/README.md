# Recovered local-LLM migration lineage

These are verbatim historical SQL files used only to identify and reproduce the
old schema. They are outside the current migration directory and are not executed
by normal app startup. Keep their bytes unchanged.

Recovered from branch `feat/local-llm-about`:

- `0020_llm_about.sql`: introduced in `c4e392e`; Git blob
  `e7f32c5f74083023eeaad3202413c64dcf59ff93`.
- `0021_tmdb_credits.sql`: Git blob
  `b205af01611a88e74eaeb26c7bd5b09d20cae11b`.

The offline converter requires all 21 successful ledger rows to match the exact
historical checksums and compares the complete SQLite schema to a fresh fixture.
It archives the original ledger in `nova_legacy_migration_ledger_v1`, records the
conversion in `nova_profile_conversion_v1`, and removes the conflicting 20/21
entries only from the disposable copy's active ledger. Current SQL migrations
20 onward then genuinely execute and record their own checksums.
