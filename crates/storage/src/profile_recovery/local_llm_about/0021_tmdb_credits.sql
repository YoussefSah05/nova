-- Netflix-style facts from TMDB's details/credits endpoint. `tmdb_id` already
-- exists (migration 0014) but was never populated; the enrichment pass now
-- stores it plus these. NULL when a title has no TMDB match.

ALTER TABLE vod_items ADD COLUMN director TEXT;
ALTER TABLE vod_items ADD COLUMN tmdb_cast TEXT;   -- comma-separated top billing
ALTER TABLE vod_items ADD COLUMN runtime INTEGER;  -- minutes
ALTER TABLE vod_items ADD COLUMN rating REAL;      -- TMDB vote_average (0-10)

ALTER TABLE series_meta ADD COLUMN director TEXT;
ALTER TABLE series_meta ADD COLUMN tmdb_cast TEXT;
ALTER TABLE series_meta ADD COLUMN runtime INTEGER;
ALTER TABLE series_meta ADD COLUMN rating REAL;
