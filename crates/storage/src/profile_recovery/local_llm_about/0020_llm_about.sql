-- Cache of AI-generated "About this title" blurbs so each title generates once.
--   kind     = 'movie' | 'series'
--   item_key = vod_items.id (as text) for movies; series_title for series
--   model    = the LLM id (a model change re-generates, never serves stale text)
--   lang     = output language (BCP-47); v1 generates English.

CREATE TABLE IF NOT EXISTS llm_about (
    kind       TEXT    NOT NULL,
    item_key   TEXT    NOT NULL,
    model      TEXT    NOT NULL,
    lang       TEXT    NOT NULL,
    text       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (kind, item_key, model, lang)
);
