-- Naming v1, step 2: keep the provider's original evidence next to each row.
--
-- The display columns (`channels.name`, `vod_items.clean_title`,
-- `series_episodes.series_title`) hold parsed titles. Until now the raw
-- provider title was discarded for channels and series, and the raw group label
-- for movies and series. These columns keep that evidence recoverable, and
-- record which parser produced the display values and which quality/language
-- tags it removed. Additive only; values fill in on the next import/refresh.

-- Raw provider title (`vod_items.name` already holds it for movies).
ALTER TABLE channels ADD COLUMN raw_name TEXT;
ALTER TABLE series_episodes ADD COLUMN raw_name TEXT;

-- Raw provider group label (`channels.raw_group` already exists).
ALTER TABLE vod_items ADD COLUMN raw_group TEXT;
ALTER TABLE series_episodes ADD COLUMN raw_group TEXT;

-- `nova_core::clean::PARSER_VERSION` that produced the parsed columns; NULL
-- for rows written before this migration or not parsed from a provider title.
ALTER TABLE channels ADD COLUMN parser_version INTEGER;
ALTER TABLE vod_items ADD COLUMN parser_version INTEGER;
ALTER TABLE series_episodes ADD COLUMN parser_version INTEGER;

-- Tags removed from the title, comma-separated normalized uppercase
-- (`4K,HEVC` / `MULTI,VOSTFR`). Variant hints, never identity.
ALTER TABLE channels ADD COLUMN quality_hints TEXT;
ALTER TABLE channels ADD COLUMN language_hints TEXT;
ALTER TABLE vod_items ADD COLUMN quality_hints TEXT;
ALTER TABLE vod_items ADD COLUMN language_hints TEXT;
ALTER TABLE series_episodes ADD COLUMN quality_hints TEXT;
ALTER TABLE series_episodes ADD COLUMN language_hints TEXT;

-- How trustworthy `season`/`episode` are. Both columns stay NOT NULL (the
-- Xtream catalog sentinel uses 0/0), so unknown numbering is stated explicitly
-- instead of being mistaken for a real S01E01:
--   season_episode - both numbers came from the provider or the title;
--   absolute       - only an episode number (e.g. "Folge 1234"); season is a placeholder;
--   unknown        - neither is known; both are placeholders.
ALTER TABLE series_episodes ADD COLUMN numbering TEXT NOT NULL DEFAULT 'season_episode'
    CHECK (numbering IN ('season_episode', 'absolute', 'unknown'));
