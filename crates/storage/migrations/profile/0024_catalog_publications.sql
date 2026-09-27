-- P10: compile -> validate -> atomic publish.
--
-- One row per import attempt for a source. A 'published' row is written inside
-- the import transaction, so it commits atomically with the catalog it
-- describes: the source's active generation is its latest published row. A
-- failed or rejected candidate is rolled back and then recorded here, outside
-- that transaction, as history only. It never changes the active generation.
CREATE TABLE IF NOT EXISTS catalog_publications (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id    INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    mode         TEXT    NOT NULL CHECK (mode IN ('append', 'replace')),
    status       TEXT    NOT NULL CHECK (status IN ('published', 'rejected', 'failed')),
    -- Secret-free machine code (e.g. 'collapse', 'endpoint_error'); NULL when published.
    reason_code  TEXT,
    -- Secret-free user-facing summary; NULL when published.
    reason       TEXT,
    live_count   INTEGER NOT NULL DEFAULT 0,
    movie_count  INTEGER NOT NULL DEFAULT 0,
    series_count INTEGER NOT NULL DEFAULT 0,
    started_at   INTEGER NOT NULL,
    finished_at  INTEGER NOT NULL,
    CHECK ((status = 'published') = (reason_code IS NULL))
);
CREATE INDEX IF NOT EXISTS idx_catalog_publications_source
    ON catalog_publications(source_id, id DESC);
