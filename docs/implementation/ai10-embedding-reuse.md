# AI10 embedding reuse across playlist refreshes

## Implementation contract

Reuse the enrichment job (`run_enrichment`), `embed_checkpointed` (runtime leases, governor
yields, cancellation), `EmbeddingRepository`'s f32 BLOB encoding and the `MODEL_ID` key.

Missing behavior: a refresh deletes and reinserts provider rows, and triggers delete their
`catalog_embeddings`, so every refresh re-embedded the whole catalog and semantic search ran on
partial vectors until it finished. Profile migration 0045 adds `embedding_text_cache` (model,
exact embedded text, document vector, last use). `embed_cached` reuses hits, embeds only misses,
stores new vectors, bumps only the timestamp of reused ones (2,000-row transactions), and prunes
entries unused for 90 days. Duplicate missed titles are embedded once; a failed cache write is
logged and never discards the enrichment run. Genre
label vectors use it too, so an unchanged catalog never loads the model.

Data: additive table, no backfill (stored rows do not keep the exact embedded text); the first
enrichment after upgrading fills it. Migration 0044 is reserved for the uncommitted
live-channel identity work; sqlx applies a later 0044 after 0045 without error
(`ignore_missing`, versions applied by absence, not order).

Non-goals: preserving provider rows across refresh (M08 fingerprints), entity-owned vectors,
series embeddings (already title-keyed and incremental), query-vector caching.

## Delivered and qualification

- Measured on this Mac with the installed multilingual-e5-small (release build, 2,000 real
  titles sampled read-only from the active profile): 202 titles/s, so the ~37k-row catalog
  (8.7k channels, 28.6k VOD) costs about 183 s of inference per refresh.
- Cache at the same scale (37k texts, 384-dim, release build): first fill 1.0 s, lookup 0.29 s,
  reuse timestamp update 0.57 s, so an unchanged refresh drops from ~3 minutes of model CPU to
  under 1 s of SQLite work, with no model load.
- Tests: round trip per model and exact text, use refreshes entries, stale entries pruned per
  model, oversize texts and empty vectors not cached; real-profile upgrade tests still pass.
- Independent review: no correctness bugs; its robustness/performance points (cache failure
  aborting the run, rewriting reused vectors, WITHOUT ROWID with wide rows, empty-vector caching,
  duplicate misses) were all applied.

Not qualified: an in-app refresh of the real playlist after the upgrade (the first run fills the
cache; the second should log `reused` ≈ total).
