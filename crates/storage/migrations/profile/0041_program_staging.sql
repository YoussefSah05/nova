-- EPG boundary (M24, review): a feed refresh parses into this staging table
-- in short per-batch writes, then publishes it to `programs` in one short
-- transaction, so the database write lock is never held while a large feed
-- is still being parsed. Rows left by an interrupted refresh are cleared by
-- the feed's next refresh.
CREATE TABLE IF NOT EXISTS program_staging (
    id              INTEGER PRIMARY KEY,
    epg_source_id   INTEGER NOT NULL REFERENCES epg_sources(id) ON DELETE CASCADE,
    channel_tvg_id  TEXT    NOT NULL,
    title           TEXT    NOT NULL,
    sub_title       TEXT,
    description     TEXT,
    start_utc       INTEGER NOT NULL,
    stop_utc        INTEGER NOT NULL,
    category        TEXT,
    icon_url        TEXT
);
CREATE INDEX IF NOT EXISTS idx_program_staging_feed ON program_staging(epg_source_id, channel_tvg_id);

-- Which feeds cover a channel, for choosing the one guide a channel shows.
CREATE INDEX IF NOT EXISTS idx_programs_channel_feed ON programs(channel_tvg_id, epg_source_id);
