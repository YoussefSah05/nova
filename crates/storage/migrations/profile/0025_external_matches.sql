-- P11: TMDB matches as external evidence on Nova entities, with provenance.
--
-- Nova ID != TMDB ID. A canonical entity (`canonical_entities.id`) is Nova's
-- identity. A TMDB ID is an external reference attached to it, together with
-- how it was chosen. Matches live on the entity, not on provider rows, so they
-- survive refreshes and are computed once per entity. Enrichment never merges or
-- renames entities: that remains entity resolution's job (P09).
CREATE TABLE IF NOT EXISTS entity_external_matches (
    entity_id       TEXT    NOT NULL REFERENCES canonical_entities(id),
    -- Authority + media kind, e.g. 'tmdb:movie'.
    namespace       TEXT    NOT NULL,
    status          TEXT    NOT NULL CHECK (status IN ('matched', 'unresolved')),
    -- The external ID when matched (TEXT: an opaque foreign key, never a Nova ID).
    external_id     TEXT,
    method          TEXT    NOT NULL CHECK (method IN ('trusted_id', 'search')),
    -- Decision confidence in basis points (search score; 10000 for a verified ID).
    confidence_bp   INTEGER CHECK (confidence_bp BETWEEN 0 AND 10000),
    matcher_version INTEGER NOT NULL,
    -- Secret-free provenance: queries, cache keys, candidates, thresholds, outcome.
    evidence_json   TEXT    NOT NULL,
    -- Display metadata, set only when matched.
    canonical_title TEXT,
    poster_url      TEXT,
    backdrop_url    TEXT,
    overview        TEXT,
    genres          TEXT,
    primary_genre   TEXT,
    checked_at      INTEGER NOT NULL,
    PRIMARY KEY (entity_id, namespace),
    CHECK ((status = 'matched') = (external_id IS NOT NULL)),
    CHECK (status = 'matched' OR canonical_title IS NULL)
);
CREATE INDEX IF NOT EXISTS idx_external_matches_external
    ON entity_external_matches(namespace, external_id);

-- Series have no canonical entity yet: record the same provenance next to the
-- existing per-source `series_meta` display row.
ALTER TABLE series_meta ADD COLUMN match_method TEXT;
ALTER TABLE series_meta ADD COLUMN confidence_bp INTEGER;
ALTER TABLE series_meta ADD COLUMN matcher_version INTEGER;
ALTER TABLE series_meta ADD COLUMN evidence_json TEXT;
ALTER TABLE series_meta ADD COLUMN checked_at INTEGER;
