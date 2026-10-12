-- Live pages sort by edition then natural title. Kept as columns (filled by
-- the live projection, ruleset 2) so a page walks an index instead of
-- decoding every channel's JSON metadata.
ALTER TABLE channels ADD COLUMN live_edition TEXT;
ALTER TABLE channels ADD COLUMN live_sort_key TEXT;
CREATE INDEX idx_channels_live_order
    ON channels(source_id, live_edition, live_sort_key, live_channel_key);
