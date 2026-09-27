-- Episode resume across refresh.
--
-- A refresh replaces a source's rows, and `watch_history` addresses episodes by
-- row id. Movie and live history is restored by matching stream URLs, but
-- Xtream episodes don't come back with the refresh: the catalog only carries a
-- per-series placeholder, and episodes are re-fetched when the series is opened.
-- Episode history whose row isn't back yet is parked here under a durable,
-- secret-free key (`nova_core::catalog::media::episode_binding_key`: the
-- provider episode id for Xtream, a locator digest otherwise), then re-attached
-- when the episode rows are re-fetched.
CREATE TABLE IF NOT EXISTS pending_episode_history (
    source_id    INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    episode_key  TEXT    NOT NULL,
    -- Series the episode belonged to (parsed title), used to re-fetch it in the
    -- background so Continue Watching refills; matching never depends on it.
    series_title TEXT    NOT NULL,
    position_sec REAL    NOT NULL,
    duration_sec REAL,
    watched_at   INTEGER NOT NULL,
    completed    INTEGER NOT NULL DEFAULT 0,
    caption_mode TEXT,
    PRIMARY KEY (source_id, episode_key)
);
