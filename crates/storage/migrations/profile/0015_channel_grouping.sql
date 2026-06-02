-- Live channel grouping: a brand-family key (collapses `beIN Sports 1/2/HD` into
-- one navigable bundle) and a coarse region bucket for two-axis (type + region)
-- browsing. Both are computed from the channel name/country at import; existing
-- catalogs pick them up on a Refresh (re-import). Raw `name`/`country` untouched.
ALTER TABLE channels ADD COLUMN channel_group TEXT;
ALTER TABLE channels ADD COLUMN region TEXT;

CREATE INDEX IF NOT EXISTS idx_channels_group ON channels(source_id, channel_group);
CREATE INDEX IF NOT EXISTS idx_channels_region ON channels(source_id, region);
