-- Vectors by the exact text that was embedded, per model. A playlist refresh
-- replaces provider rows (and their catalog_embeddings via triggers), but the
-- titles are mostly unchanged: enrichment reuses these instead of re-running
-- the model. Document (passage) vectors only, never query vectors.
-- Version 0044 is reserved for the live-channel identity work.
-- A rowid table: ~1.5 KB vectors are too wide for WITHOUT ROWID fanout.
CREATE TABLE embedding_text_cache (
    model   TEXT    NOT NULL,
    text    TEXT    NOT NULL,
    vector  BLOB    NOT NULL,
    used_at INTEGER NOT NULL,
    UNIQUE (model, text)
);

CREATE INDEX idx_embedding_text_cache_used ON embedding_text_cache(model, used_at);
