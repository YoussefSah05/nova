-- Refresh restore and rename detection look episodes up by
-- (source_id, stream_url) once per watched episode; index it.
CREATE INDEX IF NOT EXISTS idx_series_episode_url ON series_episodes(source_id, stream_url);
