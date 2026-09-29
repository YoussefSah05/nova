-- Movies browse (UI-2): genre and decade facets, and their filters, count and
-- page one playlist's films by Nova's genre and release year.
CREATE INDEX IF NOT EXISTS idx_vod_items_source_genre ON vod_items(source_id, genre);
CREATE INDEX IF NOT EXISTS idx_vod_items_source_year ON vod_items(source_id, year);
