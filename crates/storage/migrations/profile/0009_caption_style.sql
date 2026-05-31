-- Phase 6.2: personalizable caption appearance, per profile.
--
-- A singleton row (id = 1) holding the user's caption-style choices. Pure
-- presentation, applied by the React CaptionOverlay; defaults reproduce the
-- pre-6.2 look (medium sans, white text, drop-shadow). String columns mirror
-- nova_core::CaptionStyle so new presets don't need a schema change.
CREATE TABLE IF NOT EXISTS caption_style (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    font_scale   REAL    NOT NULL DEFAULT 1.0,
    font_family  TEXT    NOT NULL DEFAULT 'sans',
    text_color   TEXT    NOT NULL DEFAULT 'white',
    background    TEXT   NOT NULL DEFAULT 'shadow',
    bg_opacity   REAL    NOT NULL DEFAULT 0.6,
    updated_at   INTEGER NOT NULL DEFAULT 0
);
