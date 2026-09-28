-- Canonical series, slice 1: series identity + series-level user state.
--
-- A series becomes a canonical entity (kind 'series') that owns favorite and
-- watchlist, bound per source by a secret-free key: the provider's series ID
-- (the Xtream catalog sentinel `xtream://series/<id>`), else the exact parsed
-- title. `series_episodes` rows stay the source records; their per-row flags
-- become a compatibility mirror, as `vod_items` flags did in 0021.
--
-- `canonical_entities.kind` has CHECK (kind IN ('movie')), which SQLite can't
-- alter in place, so the table is rebuilt. Foreign keys are enforced and
-- migrations run in a transaction (`PRAGMA foreign_keys` is a no-op here), and
-- dropping a referenced parent would cascade into `entity_watch_state` and
-- fail on the other references. As in 0010, every child is parked in an
-- FK-less copy and dropped (children of `entity_lineage` first), the parent is
-- rebuilt, and each child is recreated with its exact previous definition and
-- reloaded. Nothing is reinterpreted; ids, rows and AUTOINCREMENT order stay.

-- 1. Park the children.
CREATE TABLE tmp_entity_redirects        AS SELECT * FROM entity_redirects;
CREATE TABLE tmp_entity_cannot_link      AS SELECT * FROM entity_cannot_link;
CREATE TABLE tmp_entity_lineage          AS SELECT * FROM entity_lineage;
CREATE TABLE tmp_entity_aliases          AS SELECT * FROM entity_aliases;
CREATE TABLE tmp_entity_external_matches AS SELECT * FROM entity_external_matches;
CREATE TABLE tmp_entity_watch_state      AS SELECT * FROM entity_watch_state;
CREATE TABLE tmp_stream_variants         AS SELECT * FROM stream_variants;

DROP TABLE entity_redirects;
DROP TABLE entity_cannot_link;
DROP TABLE entity_lineage;
DROP TABLE entity_aliases;
DROP TABLE entity_external_matches;
DROP TABLE entity_watch_state;
DROP TABLE stream_variants;

-- 2. Rebuild the parent with the widened kind.
CREATE TABLE canonical_entities_new (
    id              TEXT    PRIMARY KEY,
    kind            TEXT    NOT NULL CHECK (kind IN ('movie', 'series')),
    -- Last compiled display title/year; retained when no variant is available.
    title           TEXT    NOT NULL,
    year            INTEGER,
    is_favorite     INTEGER NOT NULL DEFAULT 0,
    is_in_watchlist INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    state           TEXT    NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'merged'))
);
INSERT INTO canonical_entities_new
    (id, kind, title, year, is_favorite, is_in_watchlist, created_at, state)
  SELECT id, kind, title, year, is_favorite, is_in_watchlist, created_at, state
  FROM canonical_entities;
DROP TABLE canonical_entities;
ALTER TABLE canonical_entities_new RENAME TO canonical_entities;

-- 3. Recreate the children exactly as they were, then reload them.
CREATE TABLE stream_variants (
    id          TEXT    PRIMARY KEY,
    entity_id   TEXT    NOT NULL REFERENCES canonical_entities(id),
    source_id   INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    binding_key TEXT    NOT NULL,
    -- Current source record; NULL while the provider no longer lists it
    -- (including mid-refresh, before the compiler re-binds it).
    vod_item_id INTEGER UNIQUE REFERENCES vod_items(id) ON DELETE SET NULL,
    created_at  INTEGER NOT NULL,
    UNIQUE (source_id, binding_key)
);
CREATE TABLE entity_watch_state (
    entity_id    TEXT    PRIMARY KEY REFERENCES canonical_entities(id) ON DELETE CASCADE,
    position_sec REAL    NOT NULL,
    duration_sec REAL,
    watched_at   INTEGER NOT NULL,
    completed    INTEGER NOT NULL DEFAULT 0,
    caption_mode TEXT
);
CREATE TABLE entity_lineage (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    operation       TEXT    NOT NULL CHECK (operation IN ('merge', 'cannot_link')),
    -- merge: survivor / absorbed. cannot_link: the ordered pair.
    entity_a        TEXT    NOT NULL REFERENCES canonical_entities(id),
    entity_b        TEXT    NOT NULL REFERENCES canonical_entities(id),
    actor           TEXT    NOT NULL CHECK (actor IN ('resolver', 'user')),
    rule            TEXT,
    ruleset_version INTEGER,
    -- Secret-free JSON: rule, partner, ruleset. No titles, URLs or credentials.
    evidence_json   TEXT    NOT NULL,
    -- User state of both sides before the operation, so it can be corrected.
    prior_state_json TEXT   NOT NULL,
    created_at      INTEGER NOT NULL,
    CHECK (entity_a <> entity_b)
);
CREATE TABLE entity_redirects (
    old_id       TEXT    PRIMARY KEY REFERENCES canonical_entities(id),
    new_id       TEXT    NOT NULL REFERENCES canonical_entities(id),
    operation_id INTEGER NOT NULL REFERENCES entity_lineage(id),
    created_at   INTEGER NOT NULL,
    CHECK (old_id <> new_id)
);
CREATE TABLE entity_aliases (
    entity_id  TEXT    NOT NULL REFERENCES canonical_entities(id),
    alias_kind TEXT    NOT NULL CHECK (alias_kind IN ('title', 'year', 'external')),
    value      TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (entity_id, alias_kind, value)
);
CREATE TABLE entity_cannot_link (
    entity_a     TEXT    NOT NULL REFERENCES canonical_entities(id),
    entity_b     TEXT    NOT NULL REFERENCES canonical_entities(id),
    operation_id INTEGER NOT NULL REFERENCES entity_lineage(id),
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (entity_a, entity_b),
    CHECK (entity_a < entity_b)
);
CREATE TABLE entity_external_matches (
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
    checked_at      INTEGER NOT NULL, original_title TEXT, title_language TEXT, people TEXT, keywords TEXT,
    PRIMARY KEY (entity_id, namespace),
    CHECK ((status = 'matched') = (external_id IS NOT NULL)),
    CHECK (status = 'matched' OR canonical_title IS NULL)
);

-- Parents before children; column order matches the parked copies.
INSERT INTO stream_variants         SELECT * FROM tmp_stream_variants;
INSERT INTO entity_watch_state      SELECT * FROM tmp_entity_watch_state;
INSERT INTO entity_lineage          SELECT * FROM tmp_entity_lineage;
INSERT INTO entity_redirects        SELECT * FROM tmp_entity_redirects;
INSERT INTO entity_aliases          SELECT * FROM tmp_entity_aliases;
INSERT INTO entity_cannot_link      SELECT * FROM tmp_entity_cannot_link;
INSERT INTO entity_external_matches SELECT * FROM tmp_entity_external_matches;

DROP TABLE tmp_stream_variants;
DROP TABLE tmp_entity_watch_state;
DROP TABLE tmp_entity_lineage;
DROP TABLE tmp_entity_redirects;
DROP TABLE tmp_entity_aliases;
DROP TABLE tmp_entity_cannot_link;
DROP TABLE tmp_entity_external_matches;

CREATE INDEX IF NOT EXISTS idx_stream_variants_entity ON stream_variants(entity_id);
CREATE INDEX IF NOT EXISTS idx_entity_watch_recent ON entity_watch_state(watched_at DESC);
CREATE INDEX IF NOT EXISTS idx_entity_lineage_a ON entity_lineage(entity_a);
CREATE INDEX IF NOT EXISTS idx_entity_lineage_b ON entity_lineage(entity_b);
CREATE INDEX IF NOT EXISTS idx_entity_redirects_new ON entity_redirects(new_id);
CREATE INDEX IF NOT EXISTS idx_entity_aliases_value ON entity_aliases(alias_kind, value);
CREATE INDEX IF NOT EXISTS idx_external_matches_external
    ON entity_external_matches(namespace, external_id);

CREATE TRIGGER IF NOT EXISTS entity_lineage_no_update
BEFORE UPDATE ON entity_lineage
BEGIN
    SELECT RAISE(ABORT, 'entity_lineage is append-only');
END;
CREATE TRIGGER IF NOT EXISTS entity_lineage_no_delete
BEFORE DELETE ON entity_lineage
BEGIN
    SELECT RAISE(ABORT, 'entity_lineage is append-only');
END;

-- 4. Series bindings: one per (source, key). `series_title` is the series'
--    current parsed title in `series_episodes`; NULL while the provider no
--    longer lists it (including mid-refresh, before the compiler re-binds).
--    No cascade from entities: a series entity and its state outlive every
--    binding, and source deletion removes that source's bindings only.
CREATE TABLE IF NOT EXISTS series_bindings (
    source_id    INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    binding_key  TEXT    NOT NULL,
    entity_id    TEXT    NOT NULL REFERENCES canonical_entities(id),
    series_title TEXT,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (source_id, binding_key)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_series_bindings_title
    ON series_bindings(source_id, series_title) WHERE series_title IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_series_bindings_entity ON series_bindings(entity_id);
