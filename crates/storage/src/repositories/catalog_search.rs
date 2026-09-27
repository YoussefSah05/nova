//! Canonical movie search: the `movie_search` FTS5 projection.
//!
//! One document per active canonical movie entity, built from canonical
//! tables only: the display title (confident TMDB title in the metadata
//! language) and original title, the stable entity title, provider and parsed
//! titles plus alias title keys, and genres/keywords/people. It is disposable:
//! [`CatalogSearchRepository::rebuild_movies_tx`] recreates it from scratch, and
//! a version bump rebuilds it on the next open.
//!
//! Retrieval here only finds candidates (every query word, as a prefix, in any
//! field, BM25-ordered). The final order, with exact titles first, is decided by
//! `nova_search::rank` in the caller.

use nova_core::now_ts;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;
use crate::repositories::canonical::parse_entity;
use nova_core::catalog::EntityId;

/// Version of the document builder below. Bump when the document content
/// changes; every profile rebuilds its index once on the next open.
pub const MOVIE_SEARCH_VERSION: i64 = 1;

/// A lexical candidate with its searchable text (newline-separated fields).
#[derive(Debug, Clone)]
pub struct MovieSearchCandidate {
    pub entity_id: EntityId,
    pub titles: String,
    pub aliases: String,
    pub metadata: String,
}

pub struct CatalogSearchRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> CatalogSearchRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// (Re)build the documents of one entity, or of every active movie entity
    /// when `entity` is `None`.
    async fn build_tx(
        tx: &mut Transaction<'_, Sqlite>,
        entity: Option<&str>,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "DELETE FROM movie_search WHERE ? IS NULL OR entity_id = ?",
            entity,
            entity,
        )
        .execute(&mut **tx)
        .await?;
        sqlx::query!(
            r#"INSERT INTO movie_search (entity_id, titles, aliases, metadata)
               SELECT e.id,
                      COALESCE(m.canonical_title, '') || char(10)
                          || COALESCE(m.original_title, '') || char(10) || e.title,
                      COALESCE((
                          SELECT group_concat(x, char(10)) FROM (
                              SELECT v.name AS x FROM stream_variants s
                              JOIN vod_items v ON v.id = s.vod_item_id
                              WHERE s.entity_id = e.id
                              UNION
                              SELECT v.clean_title FROM stream_variants s
                              JOIN vod_items v ON v.id = s.vod_item_id
                              WHERE s.entity_id = e.id AND v.clean_title IS NOT NULL
                              UNION
                              SELECT a.value FROM entity_aliases a
                              WHERE a.entity_id = e.id AND a.alias_kind = 'title')), ''),
                      COALESCE(m.genres, '') || char(10) || COALESCE(m.keywords, '')
                          || char(10) || COALESCE(m.people, '') || char(10)
                          || COALESCE((SELECT group_concat(DISTINCT v.genre)
                                       FROM stream_variants s
                                       JOIN vod_items v ON v.id = s.vod_item_id
                                       WHERE s.entity_id = e.id), '')
               FROM canonical_entities e
               LEFT JOIN entity_external_matches m
                 ON m.entity_id = e.id AND m.namespace = 'tmdb:movie' AND m.status = 'matched'
               WHERE e.kind = 'movie' AND e.state = 'active'
                 AND (? IS NULL OR e.id = ?)"#,
            entity,
            entity,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Rebuild the whole movie index (after an import resolved identities).
    pub async fn rebuild_movies_tx(tx: &mut Transaction<'_, Sqlite>) -> Result<(), StorageError> {
        Self::build_tx(tx, None).await?;
        let version = MOVIE_SEARCH_VERSION;
        let now = now_ts();
        sqlx::query!(
            r#"INSERT INTO search_index_state (id, movie_version, built_at) VALUES (1, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   movie_version = excluded.movie_version,
                   built_at = excluded.built_at"#,
            version,
            now,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Refresh the documents of specific entities (after TMDB matches changed).
    /// A merged or unknown entity simply ends up without a document.
    pub async fn refresh_movies_tx(
        tx: &mut Transaction<'_, Sqlite>,
        entities: &[EntityId],
    ) -> Result<(), StorageError> {
        for entity in entities {
            Self::build_tx(tx, Some(entity.as_str())).await?;
        }
        Ok(())
    }

    /// Open-time step: rebuild when the index predates this build's document
    /// builder (or was never built). A no-op otherwise.
    pub async fn ensure_movies(&self) -> Result<bool, StorageError> {
        let current = sqlx::query_scalar!(
            r#"SELECT movie_version as "v!: i64" FROM search_index_state WHERE id = 1"#
        )
        .fetch_optional(self.pool)
        .await?;
        if current.is_some_and(|v| v >= MOVIE_SEARCH_VERSION) {
            return Ok(false);
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        Self::rebuild_movies_tx(&mut tx).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Lexical candidates for `fts_expr` (see `nova_search::rank::fts_all_words`)
    /// among entities with an available variant in `source_id`, BM25-ordered
    /// with titles weighted over aliases over metadata.
    pub async fn movie_candidates(
        &self,
        source_id: i64,
        fts_expr: &str,
        limit: i64,
    ) -> Result<Vec<MovieSearchCandidate>, StorageError> {
        if fts_expr.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query!(
            r#"SELECT ms.entity_id as "entity_id!: String",
                      ms.titles as "titles!: String",
                      ms.aliases as "aliases!: String",
                      ms.metadata as "metadata!: String"
               FROM movie_search ms
               WHERE movie_search MATCH ?
                 AND EXISTS (SELECT 1 FROM stream_variants s
                             WHERE s.entity_id = ms.entity_id AND s.source_id = ?
                               AND s.vod_item_id IS NOT NULL)
               ORDER BY bm25(movie_search, 0.0, 10.0, 4.0, 1.0)
               LIMIT ?"#,
            fts_expr,
            source_id,
            limit,
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok(MovieSearchCandidate {
                    entity_id: parse_entity(r.entity_id)?,
                    titles: r.titles,
                    aliases: r.aliases,
                    metadata: r.metadata,
                })
            })
            .collect()
    }
}
