-- Catalog enrichment (Phase C): opt-in TMDB metadata. Fills a display layer of
-- canonical titles, posters, backdrops, overviews and real genres for movies and
-- series. Strictly additive — raw `name`/`logo_url` are untouched. The TMDB pass
-- is independent of Phase B's embedding pass (own `tmdb_status` so the two stages
-- gate separately and re-run idempotently on `tmdb_status IS NULL`).

ALTER TABLE vod_items ADD COLUMN canonical_title TEXT;
ALTER TABLE vod_items ADD COLUMN tmdb_id INTEGER;
ALTER TABLE vod_items ADD COLUMN poster_url TEXT;
ALTER TABLE vod_items ADD COLUMN backdrop_url TEXT;
ALTER TABLE vod_items ADD COLUMN overview TEXT;
ALTER TABLE vod_items ADD COLUMN tmdb_genres TEXT;
ALTER TABLE vod_items ADD COLUMN tmdb_status TEXT;

-- Series have no per-title row (only `series_episodes`); summaries are derived via
-- GROUP BY series_title. A sidecar table keyed by (source_id, series_title) holds
-- the enriched metadata, LEFT JOINed when building SeriesSummary.
CREATE TABLE IF NOT EXISTS series_meta (
    source_id       INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    series_title    TEXT    NOT NULL,
    canonical_title TEXT,
    tmdb_id         INTEGER,
    poster_url      TEXT,
    backdrop_url    TEXT,
    overview        TEXT,
    tmdb_genres     TEXT,
    tmdb_status     TEXT,
    PRIMARY KEY (source_id, series_title)
);

-- Response cache so repeat searches don't re-hit the API (TMDB ToS permits caching).
CREATE TABLE IF NOT EXISTS tmdb_cache (
    cache_key  TEXT    PRIMARY KEY NOT NULL,
    body       TEXT    NOT NULL,
    fetched_at INTEGER NOT NULL
);
