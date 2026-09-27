-- Title artwork chosen from TMDB `/images` (artwork v1): the title logo in
-- the metadata language and a text-free backdrop. Keyed by the TMDB work, not
-- by a Nova entity, so every entity, source and duplicate matched to one work
-- shares it. Fetched on demand when a title is shown (never in bulk); a row
-- with no logo records that TMDB had none, so it is not asked again until the
-- language or `artwork_version` changes.
CREATE TABLE IF NOT EXISTS tmdb_title_art (
    media_type      TEXT    NOT NULL CHECK (media_type IN ('movie', 'tv')),
    tmdb_id         INTEGER NOT NULL,
    -- Metadata language (BCP-47) the art was selected for.
    language        TEXT    NOT NULL,
    logo_url        TEXT,
    -- ISO 639-1 of the logo's text; NULL for a language-neutral logo.
    logo_language   TEXT,
    logo_width      INTEGER,
    logo_height     INTEGER,
    backdrop_url    TEXT,
    artwork_version INTEGER NOT NULL,
    fetched_at      INTEGER NOT NULL,
    PRIMARY KEY (media_type, tmdb_id)
) WITHOUT ROWID;
