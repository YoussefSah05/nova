-- Channel logos from the opt-in iptv-org channel directory (artwork v1),
-- matched on device by tvg-id or by a unique name (+ country). The last
-- fallback: a provider logo, then an EPG logo, win. Keyed like the channel
-- read path computes it: lower(tvg_id), else 'name:' || lower(name), so a
-- catalog refresh that rebuilds channel rows keeps the logos.
CREATE TABLE IF NOT EXISTS directory_channel_logos (
    source_id     INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    channel_key   TEXT    NOT NULL,
    -- The directory's channel id (e.g. 'CNN.us'): provenance.
    directory_id  TEXT    NOT NULL,
    url           TEXT    NOT NULL,
    method        TEXT    NOT NULL CHECK (method IN ('tvg_id', 'name')),
    confidence_bp INTEGER NOT NULL CHECK (confidence_bp BETWEEN 0 AND 10000),
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY (source_id, channel_key)
) WITHOUT ROWID;
