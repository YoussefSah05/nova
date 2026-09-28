-- Provider facts (master plan M4.6): what the provider ships beyond titles and
-- art (plot, cast, director, genre, release date, rating, runtime, trailer,
-- backdrop), normalized by `nova_core::provider_facts` and stored as JSON so
-- the field set can grow without schema changes. Display precedence: user →
-- confident TMDB → provider → parsed.
ALTER TABLE vod_items ADD COLUMN provider_facts TEXT;
ALTER TABLE series_episodes ADD COLUMN provider_facts TEXT;

-- Series-level facts. Kept apart from `series_episodes` because an Xtream
-- series' catalog row is replaced by its real episodes on first open, and
-- apart from `series_meta` (TMDB-owned). Refreshed from the catalog on every
-- import and from `get_series_info` when the series is opened.
CREATE TABLE IF NOT EXISTS series_provider_facts (
    source_id        INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    series_title     TEXT    NOT NULL,
    facts            TEXT,
    -- Provider-claimed TMDB TV id: trusted-id evidence for series matching.
    provider_tmdb_id INTEGER,
    updated_at       INTEGER NOT NULL,
    PRIMARY KEY (source_id, series_title)
) WITHOUT ROWID;
