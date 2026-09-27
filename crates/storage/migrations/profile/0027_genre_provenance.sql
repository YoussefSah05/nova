-- Naming v1, step 3: separate the provider's collection from Nova's genre.
--
-- `category` used to hold three different things in one column: the provider
-- group label (normalized), an embedding-inferred genre, and a TMDB genre, with
-- no way to tell which. It keeps its role as the browse facet (chips, rails and
-- filters read it), but it is now *derived*:
--
--     category = COALESCE(genre, collection)
--
-- and maintained by the writers (import, enrichment, TMDB) from:
--   collection   - the provider's group label, normalized for browsing
--                  (provenance; `raw_group` keeps the untouched text);
--   genre        - Nova's genre for the title, and
--   genre_source - where it came from, strongest first:
--                  'tmdb' (confident match) > 'provider' (a specific genre in
--                  the provider label) > 'inferred' (on-device embedding).
-- Additive only.

ALTER TABLE channels ADD COLUMN collection TEXT;
ALTER TABLE channels ADD COLUMN genre TEXT;
ALTER TABLE channels ADD COLUMN genre_source TEXT
    CHECK (genre_source IN ('tmdb', 'provider', 'inferred'));

ALTER TABLE vod_items ADD COLUMN collection TEXT;
ALTER TABLE vod_items ADD COLUMN genre TEXT;
ALTER TABLE vod_items ADD COLUMN genre_source TEXT
    CHECK (genre_source IN ('tmdb', 'provider', 'inferred'));

ALTER TABLE series_episodes ADD COLUMN collection TEXT;
ALTER TABLE series_episodes ADD COLUMN genre TEXT;
ALTER TABLE series_episodes ADD COLUMN genre_source TEXT
    CHECK (genre_source IN ('tmdb', 'provider', 'inferred'));

-- Existing rows: the old `category` value's origin is unknowable, so it is
-- kept as the collection and no genre is claimed. The next import/refresh
-- recomputes all three columns.
UPDATE channels SET collection = category WHERE collection IS NULL;
UPDATE vod_items SET collection = category WHERE collection IS NULL;
UPDATE series_episodes SET collection = category WHERE collection IS NULL;
