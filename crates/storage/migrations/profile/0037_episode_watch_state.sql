-- Canonical series, slice 3: episode progress owned by the series entity.
--
-- Episode resume used to be keyed to `series_episodes` row IDs, which a
-- refresh replaces (and an Xtream refresh drops until the series is opened
-- again), so it was parked in `pending_episode_history` and re-attached. Now
-- each episode's progress belongs to its canonical series entity under the
-- episode's durable, secret-free key (`nova_core::catalog::media::
-- episode_binding_key`: the provider episode ID for Xtream, else a locator
-- digest), independent of rows, refreshes and provider renames.
--
-- `watch_history` rows for episodes and the parking table stay as a legacy
-- mirror (ADD → MIGRATE → VERIFY → CUT OVER; removal is later). Rows are keyed
-- and existing progress is seeded by `WatchHistoryRepository` in Rust on
-- profile open (the key is a SHA-256 digest, which SQLite cannot compute).

ALTER TABLE series_episodes ADD COLUMN episode_key TEXT;
CREATE INDEX IF NOT EXISTS idx_series_episode_key ON series_episodes(source_id, episode_key);
-- Steady-state probe for rows written before this migration (empty after
-- the first open).
CREATE INDEX IF NOT EXISTS idx_series_episode_unkeyed
    ON series_episodes(source_id) WHERE episode_key IS NULL;

CREATE TABLE IF NOT EXISTS episode_watch_state (
    series_entity_id TEXT    NOT NULL REFERENCES canonical_entities(id) ON DELETE CASCADE,
    episode_key      TEXT    NOT NULL,
    -- Last known coordinates, so "Up next" can be found by position.
    season           INTEGER,
    episode          INTEGER,
    position_sec     REAL    NOT NULL,
    duration_sec     REAL,
    watched_at       INTEGER NOT NULL,
    completed        INTEGER NOT NULL DEFAULT 0,
    caption_mode     TEXT,
    PRIMARY KEY (series_entity_id, episode_key)
);
CREATE INDEX IF NOT EXISTS idx_episode_watch_recent ON episode_watch_state(watched_at DESC);
