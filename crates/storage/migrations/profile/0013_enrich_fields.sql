-- Catalog enrichment (Phase B): on-device embeddings tag each row with a
-- de-duplication group (variant channels/movies collapse to one) and mark its
-- enrichment state so a pass is idempotent and re-runnable. Genre fill writes
-- into the existing `category` column. Raw data is untouched.
ALTER TABLE channels ADD COLUMN dedup_group TEXT;
ALTER TABLE channels ADD COLUMN enrich_status TEXT;
ALTER TABLE vod_items ADD COLUMN dedup_group TEXT;
ALTER TABLE vod_items ADD COLUMN enrich_status TEXT;

CREATE INDEX IF NOT EXISTS idx_channels_dedup ON channels(source_id, dedup_group);
CREATE INDEX IF NOT EXISTS idx_vod_dedup ON vod_items(source_id, dedup_group);
