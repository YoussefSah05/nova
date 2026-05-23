-- Phase 3: favourites, recently-added timestamps, watch history.

ALTER TABLE channels         ADD COLUMN added_at    INTEGER;
ALTER TABLE vod_items        ADD COLUMN added_at    INTEGER;
ALTER TABLE vod_items        ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE series_episodes  ADD COLUMN added_at    INTEGER;
ALTER TABLE series_episodes  ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_channels_added        ON channels(source_id, added_at DESC);
CREATE INDEX IF NOT EXISTS idx_channels_favorite     ON channels(source_id, is_favorite);
CREATE INDEX IF NOT EXISTS idx_vod_added             ON vod_items(source_id, added_at DESC);
CREATE INDEX IF NOT EXISTS idx_vod_favorite          ON vod_items(source_id, is_favorite);
CREATE INDEX IF NOT EXISTS idx_series_added          ON series_episodes(source_id, added_at DESC);
CREATE INDEX IF NOT EXISTS idx_series_favorite_title ON series_episodes(source_id, series_title, is_favorite);

-- Continue Watching. Populated by Phase 5 playback; remains empty until then.
-- One row per (media_kind, media_id) — INSERT OR REPLACE updates the same row.
CREATE TABLE IF NOT EXISTS watch_history (
    id            INTEGER PRIMARY KEY,
    media_kind    TEXT    NOT NULL,           -- 'vod' | 'episode' | 'live'
    media_id      INTEGER NOT NULL,
    position_sec  REAL    NOT NULL,
    duration_sec  REAL,
    watched_at    INTEGER NOT NULL,
    completed     INTEGER NOT NULL DEFAULT 0,
    UNIQUE(media_kind, media_id)
);
CREATE INDEX IF NOT EXISTS idx_watch_history_recent ON watch_history(watched_at DESC);
