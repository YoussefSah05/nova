-- Phase 5: caption-mode scaffolding for the player.
--
-- `watch_history.caption_mode` records what the user had selected when they
-- last played a media item (so the next session can restore it). Always
-- written by the player, but in Phase 5 it's always 'off' until Phase 6
-- delivers source captions and Phase 7 delivers translations.
ALTER TABLE watch_history ADD COLUMN caption_mode TEXT;

-- Sticky caption preference per content scope, per ARCHITECTURE.md section 6.
-- 'channel' / 'vod' keys map to channels.id / vod_items.id;
-- 'source_lang' keys are BCP-47 codes ('en', 'fr', 'ar', ...) for a "always
-- translate French content to English" preference. Populated by the caption
-- picker once Phase 6/7 land - empty in Phase 5.
CREATE TABLE IF NOT EXISTS caption_preferences (
    id           INTEGER PRIMARY KEY,
    scope        TEXT    NOT NULL,           -- 'channel' | 'vod' | 'source_lang'
    scope_key    TEXT    NOT NULL,
    caption_mode TEXT    NOT NULL,           -- 'off' | 'source' | 'translation:<lang>' | 'dual:<src>:<tgt>'
    updated_at   INTEGER NOT NULL,
    UNIQUE(scope, scope_key)
);
