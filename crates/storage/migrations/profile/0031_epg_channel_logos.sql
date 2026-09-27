-- Channel logos declared by XMLTV feeds (`<channel id><icon src>`), matched to
-- the source's channels by tvg-id at EPG refresh. A fallback only: a provider
-- logo on the channel row wins. Keyed by the channel's lowercased tvg-id, so a
-- catalog refresh that rebuilds channel rows keeps its logos.
CREATE TABLE IF NOT EXISTS epg_channel_logos (
    epg_source_id INTEGER NOT NULL REFERENCES epg_sources(id) ON DELETE CASCADE,
    source_id     INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    channel_key   TEXT    NOT NULL,
    url           TEXT    NOT NULL,
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY (epg_source_id, channel_key)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_epg_channel_logos_lookup
    ON epg_channel_logos (source_id, channel_key);
