-- Source-scoped, reversible projections of existing channel evidence.
ALTER TABLE channels ADD COLUMN live_channel_key TEXT;
ALTER TABLE channels ADD COLUMN live_variant_key TEXT;
ALTER TABLE channels ADD COLUMN live_metadata TEXT;
ALTER TABLE channels ADD COLUMN live_ruleset INTEGER;
CREATE INDEX idx_channels_live_identity ON channels(source_id, live_channel_key);
CREATE INDEX idx_channels_live_variant ON channels(source_id, live_variant_key);
CREATE INDEX idx_channels_live_ruleset ON channels(live_ruleset);
CREATE TABLE live_variant_health (
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    variant_key TEXT NOT NULL,
    last_success_at INTEGER,
    last_failure_at INTEGER,
    PRIMARY KEY(source_id, variant_key)
);
CREATE TABLE live_channel_preferences (
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    channel_key TEXT NOT NULL,
    variant_key TEXT NOT NULL,
    PRIMARY KEY(source_id, channel_key)
);
