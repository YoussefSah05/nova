ALTER TABLE series_episodes ADD COLUMN category TEXT;
CREATE INDEX IF NOT EXISTS idx_series_category ON series_episodes(source_id, category);
