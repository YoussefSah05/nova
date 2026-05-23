CREATE TABLE IF NOT EXISTS sources (
    id                  INTEGER PRIMARY KEY,
    kind                TEXT    NOT NULL DEFAULT 'm3u',
    name                TEXT    NOT NULL,
    url                 TEXT    NOT NULL,
    last_refreshed_at   INTEGER,
    auto_refresh_hours  INTEGER NOT NULL DEFAULT 24
);

CREATE TABLE IF NOT EXISTS channels (
    id              INTEGER PRIMARY KEY,
    source_id       INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    tvg_id          TEXT,
    name            TEXT    NOT NULL,
    country         TEXT,
    category        TEXT,
    raw_group       TEXT,
    logo_url        TEXT,
    logo_cached_path TEXT,
    stream_url      TEXT    NOT NULL,
    is_favorite     INTEGER NOT NULL DEFAULT 0,
    last_played_at  INTEGER
);
CREATE INDEX IF NOT EXISTS idx_channels_source   ON channels(source_id);
CREATE INDEX IF NOT EXISTS idx_channels_category ON channels(source_id, category);

CREATE TABLE IF NOT EXISTS vod_items (
    id                INTEGER PRIMARY KEY,
    source_id         INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    name              TEXT    NOT NULL,
    clean_title       TEXT,
    year              INTEGER,
    logo_url          TEXT,
    stream_url        TEXT    NOT NULL,
    category          TEXT,
    detected_language TEXT
);
CREATE INDEX IF NOT EXISTS idx_vod_source   ON vod_items(source_id);
CREATE INDEX IF NOT EXISTS idx_vod_category ON vod_items(source_id, category);

CREATE TABLE IF NOT EXISTS series_episodes (
    id            INTEGER PRIMARY KEY,
    source_id     INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    series_title  TEXT    NOT NULL,
    season        INTEGER NOT NULL,
    episode       INTEGER NOT NULL,
    episode_title TEXT,
    logo_url      TEXT,
    stream_url    TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_series_source ON series_episodes(source_id, series_title);

CREATE TABLE IF NOT EXISTS jobs (
    id          INTEGER PRIMARY KEY,
    kind        TEXT    NOT NULL,
    payload     TEXT    NOT NULL,
    state       TEXT    NOT NULL DEFAULT 'queued',
    priority    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_jobs_state ON jobs(state, priority DESC, created_at);
