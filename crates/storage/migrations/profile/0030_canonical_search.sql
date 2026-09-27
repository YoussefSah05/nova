-- P12: canonical search.
--
-- People and keywords of a confident TMDB match (details with credits and
-- keywords), stored with the match as searchable metadata.
ALTER TABLE entity_external_matches ADD COLUMN people TEXT;
ALTER TABLE entity_external_matches ADD COLUMN keywords TEXT;

-- One search document per active canonical movie entity. A disposable
-- projection: it is rebuilt from canonical tables (entities, bindings, TMDB
-- matches, aliases) and never an identity store.
--   titles   - display title (TMDB, metadata language) and original title;
--   aliases  - provider titles, parsed titles and alias title keys;
--   metadata - genres, keywords, people.
-- `unicode61 remove_diacritics 2` makes retrieval accent-insensitive; exact
-- ranking is decided in Rust (`nova_search::rank`).
CREATE VIRTUAL TABLE IF NOT EXISTS movie_search USING fts5(
    entity_id UNINDEXED,
    titles,
    aliases,
    metadata,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Version of the document builder; a newer build rebuilds the index once.
CREATE TABLE IF NOT EXISTS search_index_state (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    movie_version INTEGER NOT NULL,
    built_at     INTEGER NOT NULL
);
