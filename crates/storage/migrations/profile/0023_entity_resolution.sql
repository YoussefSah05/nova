-- P09: deterministic canonical entity resolution with durable identity history.
--
-- Merged entities are tombstoned, never deleted: their row stays so that
-- historical IDs (a stale UI cache, a bookmark, a future sync peer) resolve
-- through `entity_redirects` to the surviving entity. User state moves to the
-- survivor at merge time; `entity_lineage` records what each side held before.
-- Additive only.

-- 'active' entities are served; 'merged' ones only redirect.
ALTER TABLE canonical_entities
    ADD COLUMN state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'merged'));

-- Provider-claimed TMDB movie ID (e.g. Xtream `get_vod_streams.tmdb`), used as
-- trusted-ID evidence by the resolver. Kept separate from the display-only
-- TMDB enrichment columns (0014), which are lexical matches, not identity.
ALTER TABLE vod_items ADD COLUMN provider_tmdb_id INTEGER;

-- Append-only operation log. Every identity change is one row. A future
-- correction (unmerge) adds a compensating row and never rewrites history.
CREATE TABLE IF NOT EXISTS entity_lineage (
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
CREATE INDEX IF NOT EXISTS idx_entity_lineage_a ON entity_lineage(entity_a);
CREATE INDEX IF NOT EXISTS idx_entity_lineage_b ON entity_lineage(entity_b);

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

-- Old ID -> surviving ID. Chains are compressed on every merge, so a redirect
-- normally points directly at an active entity. Readers still resolve with a
-- bounded, cycle-checked walk.
CREATE TABLE IF NOT EXISTS entity_redirects (
    old_id       TEXT    PRIMARY KEY REFERENCES canonical_entities(id),
    new_id       TEXT    NOT NULL REFERENCES canonical_entities(id),
    operation_id INTEGER NOT NULL REFERENCES entity_lineage(id),
    created_at   INTEGER NOT NULL,
    CHECK (old_id <> new_id)
);
CREATE INDEX IF NOT EXISTS idx_entity_redirects_new ON entity_redirects(new_id);

-- Durable identity evidence that outlives the provider rows it came from:
-- normalized titles, years and trusted external IDs (`tmdb:movie:603`). An
-- alias is not globally unique, and several entities may share one.
CREATE TABLE IF NOT EXISTS entity_aliases (
    entity_id  TEXT    NOT NULL REFERENCES canonical_entities(id),
    alias_kind TEXT    NOT NULL CHECK (alias_kind IN ('title', 'year', 'external')),
    value      TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (entity_id, alias_kind, value)
);
CREATE INDEX IF NOT EXISTS idx_entity_aliases_value ON entity_aliases(alias_kind, value);

-- Explicit "these are different" decisions (user corrections). They always
-- override matching evidence and follow merges to the survivor.
CREATE TABLE IF NOT EXISTS entity_cannot_link (
    entity_a     TEXT    NOT NULL REFERENCES canonical_entities(id),
    entity_b     TEXT    NOT NULL REFERENCES canonical_entities(id),
    operation_id INTEGER NOT NULL REFERENCES entity_lineage(id),
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (entity_a, entity_b),
    CHECK (entity_a < entity_b)
);

-- Which deterministic rule set last resolved this profile. A newer build
-- re-resolves once on open when the version moves.
CREATE TABLE IF NOT EXISTS entity_resolution_state (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    ruleset_version INTEGER NOT NULL,
    resolved_at     INTEGER NOT NULL
);
