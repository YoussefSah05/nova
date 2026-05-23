-- Phase 3 polish: watchlist (save-for-later) + backfill added_at for rows
-- imported before migration 0003 added the column.

ALTER TABLE channels        ADD COLUMN is_in_watchlist INTEGER NOT NULL DEFAULT 0;
ALTER TABLE vod_items       ADD COLUMN is_in_watchlist INTEGER NOT NULL DEFAULT 0;
ALTER TABLE series_episodes ADD COLUMN is_in_watchlist INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_channels_watchlist     ON channels(source_id, is_in_watchlist);
CREATE INDEX IF NOT EXISTS idx_vod_watchlist          ON vod_items(source_id, is_in_watchlist);
CREATE INDEX IF NOT EXISTS idx_series_watchlist_title ON series_episodes(source_id, series_title, is_in_watchlist);

-- All pre-migration-0003 rows have added_at = NULL, which excludes them from the
-- Recently Added shelf. Stamp them with the migration time so they appear in
-- ascending-id order (the secondary sort key) within the shelf.
UPDATE channels        SET added_at = CAST(strftime('%s', 'now') AS INTEGER) WHERE added_at IS NULL;
UPDATE vod_items       SET added_at = CAST(strftime('%s', 'now') AS INTEGER) WHERE added_at IS NULL;
UPDATE series_episodes SET added_at = CAST(strftime('%s', 'now') AS INTEGER) WHERE added_at IS NULL;
