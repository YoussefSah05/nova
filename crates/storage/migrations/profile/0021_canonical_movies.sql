-- P07: first canonical-catalog vertical slice (movies).
--
--   SourceRecord  = a provider row (`vod_items`), replaced on every refresh.
--   CanonicalEntity = a Nova-owned movie identity that owns durable user state.
--   StreamVariant = a playable provider source of an entity, bound to the
--                   current source record by a secret-free binding key.
--
-- Additive only: legacy columns (`vod_items.is_favorite` / `is_in_watchlist`,
-- `watch_history` rows for 'vod') are kept as a compatibility mirror. Rows are
-- compiled/backfilled by `CanonicalRepository` in Rust (binding keys are
-- SHA-256 digests, which SQLite cannot compute).

-- Secret-free keys computed at the catalog compiler boundary. `binding_key`
-- prefers a provider-scoped stable ID (e.g. an Xtream stream ID) so identity
-- survives credential rotation; `locator_key` is the locator digest, used to
-- re-bind variants created before a stable ID was known.
ALTER TABLE vod_items ADD COLUMN binding_key TEXT;
ALTER TABLE vod_items ADD COLUMN locator_key TEXT;
CREATE INDEX IF NOT EXISTS idx_vod_binding ON vod_items(source_id, binding_key);
CREATE INDEX IF NOT EXISTS idx_vod_locator ON vod_items(source_id, locator_key);

CREATE TABLE IF NOT EXISTS canonical_entities (
    id              TEXT    PRIMARY KEY,
    kind            TEXT    NOT NULL CHECK (kind IN ('movie')),
    -- Last compiled display title/year; retained when no variant is available.
    title           TEXT    NOT NULL,
    year            INTEGER,
    is_favorite     INTEGER NOT NULL DEFAULT 0,
    is_in_watchlist INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);

-- No cascade from entities: an entity (and its user state) outlives every
-- variant. Source deletion removes that source's variants only.
CREATE TABLE IF NOT EXISTS stream_variants (
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
CREATE INDEX IF NOT EXISTS idx_stream_variants_entity ON stream_variants(entity_id);

-- Resume/completion owned by the entity, not a volatile provider row ID.
CREATE TABLE IF NOT EXISTS entity_watch_state (
    entity_id    TEXT    PRIMARY KEY REFERENCES canonical_entities(id) ON DELETE CASCADE,
    position_sec REAL    NOT NULL,
    duration_sec REAL,
    watched_at   INTEGER NOT NULL,
    completed    INTEGER NOT NULL DEFAULT 0,
    caption_mode TEXT
);
CREATE INDEX IF NOT EXISTS idx_entity_watch_recent ON entity_watch_state(watched_at DESC);
