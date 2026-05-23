-- Phase 4: EPG programs + per-source EPG URLs tracker.
-- Programs are indexed by `channel_tvg_id`, which matches `channels.tvg_id`.
-- Times are UTC epoch seconds (never strings), per skills/xmltv-epg/SKILL.md.

CREATE TABLE IF NOT EXISTS programs (
    id              INTEGER PRIMARY KEY,
    channel_tvg_id  TEXT    NOT NULL,
    title           TEXT    NOT NULL,
    sub_title       TEXT,
    description     TEXT,
    start_utc       INTEGER NOT NULL,
    stop_utc        INTEGER NOT NULL,
    category        TEXT,
    icon_url        TEXT
);
CREATE INDEX IF NOT EXISTS idx_programs_channel_time ON programs(channel_tvg_id, start_utc);
CREATE INDEX IF NOT EXISTS idx_programs_stop         ON programs(stop_utc);

-- One row per resolved EPG feed URL for a source. A single source may produce
-- multiple rows: the provider-supplied feed (Xtream xmltv.php or M3U url-tvg)
-- and any community fallback feeds resolved later.
CREATE TABLE IF NOT EXISTS epg_sources (
    id                INTEGER PRIMARY KEY,
    source_id         INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    url               TEXT    NOT NULL,
    kind              TEXT    NOT NULL,  -- 'xtream' | 'm3u_url_tvg' | 'community'
    last_refresh_utc  INTEGER,
    last_error        TEXT,
    UNIQUE(source_id, url)
);
CREATE INDEX IF NOT EXISTS idx_epg_sources_source ON epg_sources(source_id);
