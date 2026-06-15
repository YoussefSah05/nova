-- Series semantic search. Series are title-keyed (no numeric id), so their
-- embeddings live in a dedicated table rather than catalog_embeddings (which is
-- keyed by integer content_id). One vector per (source, title, model); the FK
-- cascade drops them when a source is removed. Enrichment fills these in a
-- series stage; brute-force cosine in nova_search ranks them like the catalog.

CREATE TABLE IF NOT EXISTS series_embeddings (
    source_id    INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    series_title TEXT    NOT NULL,
    model        TEXT    NOT NULL,
    vector       BLOB    NOT NULL,
    PRIMARY KEY (source_id, series_title, model)
);
