-- Naming v1, step 4: TMDB naming data.
--
-- `canonical_title` is TMDB's title in the requested metadata language. Keep
-- which language that was, and the original-language title, so a localized
-- display title never replaces the original and a language change can be
-- detected (and the title re-fetched). Additive only.
ALTER TABLE entity_external_matches ADD COLUMN original_title TEXT;
-- BCP-47 tag of `canonical_title`, e.g. 'en-US', 'fr-FR'.
ALTER TABLE entity_external_matches ADD COLUMN title_language TEXT;

ALTER TABLE series_meta ADD COLUMN original_title TEXT;
ALTER TABLE series_meta ADD COLUMN title_language TEXT;
