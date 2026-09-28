-- Rich TMDB facts (master plan M4.6), parsed and stored per TMDB work like
-- `tmdb_title_art`: every entity, source and duplicate matched to one work
-- shares them. Fetched on demand when a title's page opens (never in bulk),
-- refreshed after a TTL (ratings and upcoming episodes change) or when the
-- metadata language or `facts_version` changes.
CREATE TABLE IF NOT EXISTS tmdb_title_facts (
    media_type    TEXT    NOT NULL CHECK (media_type IN ('movie', 'tv')),
    tmdb_id       INTEGER NOT NULL,
    language      TEXT    NOT NULL,
    facts         TEXT    NOT NULL,
    facts_version INTEGER NOT NULL,
    fetched_at    INTEGER NOT NULL,
    PRIMARY KEY (media_type, tmdb_id)
) WITHOUT ROWID;

-- TMDB episode lists per season of a TV work (names, overviews, stills,
-- air dates, runtimes), merged with provider episodes by episode number.
CREATE TABLE IF NOT EXISTS tmdb_season_episodes (
    tmdb_id       INTEGER NOT NULL,
    season        INTEGER NOT NULL,
    language      TEXT    NOT NULL,
    episodes      TEXT    NOT NULL,
    facts_version INTEGER NOT NULL,
    fetched_at    INTEGER NOT NULL,
    PRIMARY KEY (tmdb_id, season)
) WITHOUT ROWID;
