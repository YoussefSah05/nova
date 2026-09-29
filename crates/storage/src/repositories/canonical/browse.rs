//! Canonical movie browse reads: one card per entity with an available
//! variant in the playlist, shown through its lowest bound row, with the
//! card facts (genres, runtime, rating, badges) derived from stored evidence
//! — the provider's facts, the TMDB match and TMDB facts fetched earlier —
//! and never from the network. Plus the facet lists the Movies section
//! browses by: genres, decades and TMDB collections present in the library.

use nova_core::catalog::media::{CanonicalMediaSummary, CardFacts};
use nova_core::catalog::variants::display_badges;
use nova_core::catalog::{ContentKind, EntityId};
use nova_core::ContentSort;
use serde::Serialize;

use super::{parse_entity, CanonicalRepository, MovieFilters};
use crate::error::StorageError;

/// A facet value with how many films carry it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetCount {
    pub name: String,
    pub count: i64,
}

/// Films of one decade (`1990` for the 1990s), with a few posters.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecadeCount {
    pub decade: i64,
    pub count: i64,
    pub poster_urls: Vec<String>,
}

/// A TMDB collection with at least two of its films in the library.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryCollection {
    pub id: i64,
    pub name: String,
    pub count: i64,
    pub poster_urls: Vec<String>,
}

/// Separator for poster lists built with `group_concat` (URLs never hold it).
const SEP: char = '\u{1f}';

fn posters(joined: Option<String>, max: usize) -> Vec<String> {
    joined
        .map(|j| {
            j.split(SEP)
                .filter(|p| !p.is_empty())
                .take(max)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn sort_code(sort: ContentSort) -> i64 {
    match sort {
        ContentSort::Default => 0,
        ContentSort::RecentlyAdded => 1,
        ContentSort::Title => 2,
        ContentSort::Year => 3,
        ContentSort::Rating => 4,
    }
}

/// Card facts from a row's stored evidence.
#[allow(clippy::too_many_arguments)]
pub(super) fn card_facts(
    tmdb_genres: Option<String>,
    genre: Option<String>,
    runtime_min: Option<i64>,
    rating: Option<f64>,
    rating_source: Option<String>,
    quality: Option<String>,
    languages: Option<String>,
    version_count: i64,
) -> CardFacts {
    let genres = tmdb_genres
        .filter(|g| !g.trim().is_empty())
        .map(|g| {
            g.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .or_else(|| genre.map(|g| vec![g]))
        .unwrap_or_default();
    let quality = quality.unwrap_or_default();
    let languages = languages.unwrap_or_default();
    let rating = rating.filter(|r| *r > 0.0).map(|r| r as f32);
    CardFacts {
        genres,
        runtime_min: runtime_min.filter(|m| *m > 0),
        rating,
        rating_source: rating.and(rating_source),
        badges: display_badges(quality.split(','), languages.split(',')),
        version_count,
    }
}

/// Build a card from a query row: the browse queries all select the same
/// columns under the same names.
macro_rules! summary_from_row {
    ($r:expr) => {{
        let r = $r;
        Ok::<_, StorageError>(CanonicalMediaSummary {
            entity_id: parse_entity(r.entity_id)?,
            kind: ContentKind::Movie,
            title: r.title,
            source_title: r.source_title,
            year: r.year,
            category: r.category,
            poster_url: r.poster_url,
            backdrop_url: r.backdrop_url,
            overview: r.overview,
            is_favorite: r.is_favorite != 0,
            is_in_watchlist: r.is_in_watchlist != 0,
            added_at: r.added_at,
            dedup_group: r.dedup_group,
            facts: card_facts(
                r.genres,
                r.genre,
                r.runtime_min,
                r.rating,
                r.rating_source,
                r.quality,
                r.languages,
                r.versions,
            ),
        })
    }};
}

impl CanonicalRepository<'_> {
    /// One card per canonical movie with an available variant in `source_id`.
    /// Filters apply to the representative row; `genre` is Nova's primary
    /// genre, the year range is inclusive, `collection_id` a TMDB collection.
    pub async fn list_movies(
        &self,
        source_id: i64,
        limit: i64,
        offset: i64,
        filters: &MovieFilters<'_>,
        sort: ContentSort,
    ) -> Result<Vec<CanonicalMediaSummary>, StorageError> {
        let search = super::like_query(filters.query);
        let search = search.as_deref();
        let category = filters.category;
        let genre = filters.genre;
        let fav: i64 = filters.favorites_only.into();
        let wl: i64 = filters.watchlist_only.into();
        let sort = sort_code(sort);
        let rows = sqlx::query!(
            r#"SELECT entity_id as "entity_id!: String", title as "title!: String",
                      source_title as "source_title!: String", year as "year?: i64",
                      category as "category?: String", poster_url as "poster_url?: String",
                      backdrop_url as "backdrop_url?: String", overview as "overview?: String",
                      is_favorite as "is_favorite!: i64", is_in_watchlist as "is_in_watchlist!: i64",
                      added_at as "added_at?: i64", dedup_group as "dedup_group?: String",
                      genres as "genres?: String", genre as "genre?: String",
                      runtime_min as "runtime_min?: i64", rating as "rating?: f64",
                      rating_source as "rating_source?: String", quality as "quality?: String",
                      languages as "languages?: String", versions as "versions!: i64"
               FROM (
                 SELECT e.id AS entity_id,
                        COALESCE(v.canonical_title, e.title, v.name) AS title,
                        v.name AS source_title, v.year AS year, v.category AS category,
                        COALESCE(v.poster_url, v.logo_url) AS poster_url,
                        v.backdrop_url AS backdrop_url, v.overview AS overview,
                        e.is_favorite AS is_favorite, e.is_in_watchlist AS is_in_watchlist,
                        v.added_at AS added_at, v.dedup_group AS dedup_group,
                        COALESCE(m.genres, v.tmdb_genres) AS genres, v.genre AS genre,
                        CAST(COALESCE(json_extract(f.facts, '$.runtimeMin'),
                                      json_extract(v.provider_facts, '$.duration_sec') / 60)
                             AS INTEGER) AS runtime_min,
                        COALESCE(json_extract(f.facts, '$.rating'),
                                 json_extract(v.provider_facts, '$.rating')) AS rating,
                        CASE WHEN json_extract(f.facts, '$.rating') IS NOT NULL THEN 'tmdb'
                             WHEN json_extract(v.provider_facts, '$.rating') IS NOT NULL THEN 'provider'
                        END AS rating_source,
                        (SELECT group_concat(q.quality_hints) FROM stream_variants r
                         JOIN vod_items q ON q.id = r.vod_item_id
                         WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id) AS quality,
                        (SELECT group_concat(q.language_hints) FROM stream_variants r
                         JOIN vod_items q ON q.id = r.vod_item_id
                         WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id) AS languages,
                        (SELECT COUNT(*) FROM stream_variants r
                         WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id
                           AND r.vod_item_id IS NOT NULL) AS versions,
                        v.id AS row_id
                 FROM stream_variants s
                 JOIN canonical_entities e ON e.id = s.entity_id
                 JOIN vod_items v ON v.id = s.vod_item_id
                 LEFT JOIN entity_external_matches m
                        ON m.entity_id = e.id AND m.namespace = 'tmdb:movie' AND m.status = 'matched'
                 LEFT JOIN tmdb_title_facts f
                        ON f.media_type = 'movie' AND f.tmdb_id = CAST(m.external_id AS INTEGER)
                 WHERE s.source_id = ?2
                   AND s.vod_item_id = (
                     SELECT MIN(r.vod_item_id) FROM stream_variants r
                     WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id)
                   AND (?3 IS NULL OR v.category = ?3)
                   AND (?4 IS NULL OR v.name LIKE ?4 OR v.clean_title LIKE ?4
                        OR v.canonical_title LIKE ?4 OR v.category LIKE ?4)
                   AND (?5 = 0 OR e.is_favorite = 1)
                   AND (?6 = 0 OR e.is_in_watchlist = 1)
                   AND (?7 IS NULL OR v.genre = ?7)
                   AND (?8 IS NULL OR v.year >= ?8)
                   AND (?9 IS NULL OR v.year <= ?9)
                   AND (?12 IS NULL OR json_extract(f.facts, '$.collection.id') = ?12)
               )
               ORDER BY CASE WHEN ?1 = 1 THEN added_at END DESC,
                        CASE WHEN ?1 = 1 THEN row_id END DESC,
                        CASE WHEN ?1 = 2 THEN title END COLLATE NOCASE ASC,
                        CASE WHEN ?1 = 3 THEN year END DESC,
                        CASE WHEN ?1 = 4 THEN rating END DESC,
                        row_id ASC
               LIMIT ?10 OFFSET ?11"#,
            sort,
            source_id,
            category,
            search,
            fav,
            wl,
            genre,
            filters.year_from,
            filters.year_to,
            limit,
            offset,
            filters.collection_id,
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter().map(|r| summary_from_row!(r)).collect()
    }

    /// Cards for specific entities in `source_id`, in the order given
    /// (search results). Entities without an available variant there are
    /// skipped.
    pub async fn movies_by_ids(
        &self,
        source_id: i64,
        ids: &[EntityId],
    ) -> Result<Vec<CanonicalMediaSummary>, StorageError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let json = serde_json::to_string(&ids.iter().map(EntityId::as_str).collect::<Vec<_>>())?;
        let rows = sqlx::query!(
            r#"SELECT e.id as "entity_id!: String",
                      COALESCE(v.canonical_title, e.title, v.name) as "title!: String",
                      v.name as "source_title!: String", v.year as "year?: i64",
                      v.category as "category?: String",
                      COALESCE(v.poster_url, v.logo_url) as "poster_url?: String",
                      v.backdrop_url as "backdrop_url?: String", v.overview as "overview?: String",
                      e.is_favorite as "is_favorite!: i64", e.is_in_watchlist as "is_in_watchlist!: i64",
                      v.added_at as "added_at?: i64", v.dedup_group as "dedup_group?: String",
                      COALESCE(m.genres, v.tmdb_genres) as "genres?: String",
                      v.genre as "genre?: String",
                      CAST(COALESCE(json_extract(f.facts, '$.runtimeMin'),
                                    json_extract(v.provider_facts, '$.duration_sec') / 60)
                           AS INTEGER) as "runtime_min?: i64",
                      COALESCE(json_extract(f.facts, '$.rating'),
                               json_extract(v.provider_facts, '$.rating')) as "rating?: f64",
                      CASE WHEN json_extract(f.facts, '$.rating') IS NOT NULL THEN 'tmdb'
                           WHEN json_extract(v.provider_facts, '$.rating') IS NOT NULL THEN 'provider'
                      END as "rating_source?: String",
                      (SELECT group_concat(q.quality_hints) FROM stream_variants r
                       JOIN vod_items q ON q.id = r.vod_item_id
                       WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id) as "quality?: String",
                      (SELECT group_concat(q.language_hints) FROM stream_variants r
                       JOIN vod_items q ON q.id = r.vod_item_id
                       WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id) as "languages?: String",
                      (SELECT COUNT(*) FROM stream_variants r
                       WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id
                         AND r.vod_item_id IS NOT NULL) as "versions!: i64"
               FROM stream_variants s
               JOIN canonical_entities e ON e.id = s.entity_id
               JOIN vod_items v ON v.id = s.vod_item_id
               LEFT JOIN entity_external_matches m
                      ON m.entity_id = e.id AND m.namespace = 'tmdb:movie' AND m.status = 'matched'
               LEFT JOIN tmdb_title_facts f
                      ON f.media_type = 'movie' AND f.tmdb_id = CAST(m.external_id AS INTEGER)
               WHERE s.source_id = ?
                 AND s.entity_id IN (SELECT value FROM json_each(?))
                 AND s.vod_item_id = (
                   SELECT MIN(r.vod_item_id) FROM stream_variants r
                   WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id)"#,
            source_id,
            json,
        )
        .fetch_all(self.pool)
        .await?;
        let mut by_id = std::collections::HashMap::new();
        for r in rows {
            let key = r.entity_id.clone();
            by_id.insert(key, summary_from_row!(r)?);
        }
        Ok(ids
            .iter()
            .filter_map(|id| by_id.remove(id.as_str()))
            .collect())
    }

    pub async fn count_movies(
        &self,
        source_id: i64,
        filters: &MovieFilters<'_>,
    ) -> Result<i64, StorageError> {
        let search = super::like_query(filters.query);
        let search = search.as_deref();
        let category = filters.category;
        let genre = filters.genre;
        let fav: i64 = filters.favorites_only.into();
        let wl: i64 = filters.watchlist_only.into();
        let n = sqlx::query_scalar!(
            r#"SELECT COUNT(*) as "n!: i64"
               FROM stream_variants s
               JOIN canonical_entities e ON e.id = s.entity_id
               JOIN vod_items v ON v.id = s.vod_item_id
               LEFT JOIN entity_external_matches m
                      ON m.entity_id = e.id AND m.namespace = 'tmdb:movie' AND m.status = 'matched'
               LEFT JOIN tmdb_title_facts f
                      ON f.media_type = 'movie' AND f.tmdb_id = CAST(m.external_id AS INTEGER)
               WHERE s.source_id = ?1
                 AND s.vod_item_id = (
                   SELECT MIN(r.vod_item_id) FROM stream_variants r
                   WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id)
                 AND (?2 IS NULL OR v.category = ?2)
                 AND (?3 IS NULL OR v.name LIKE ?3 OR v.clean_title LIKE ?3
                      OR v.canonical_title LIKE ?3 OR v.category LIKE ?3)
                 AND (?4 = 0 OR e.is_favorite = 1)
                 AND (?5 = 0 OR e.is_in_watchlist = 1)
                 AND (?6 IS NULL OR v.genre = ?6)
                 AND (?7 IS NULL OR v.year >= ?7)
                 AND (?8 IS NULL OR v.year <= ?8)
                 AND (?9 IS NULL OR json_extract(f.facts, '$.collection.id') = ?9)"#,
            source_id,
            category,
            search,
            fav,
            wl,
            genre,
            filters.year_from,
            filters.year_to,
            filters.collection_id,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(n)
    }

    /// Nova's genres among the playlist's films, most common first.
    pub async fn movie_genres(&self, source_id: i64) -> Result<Vec<FacetCount>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT v.genre as "name!: String", COUNT(*) as "count!: i64"
               FROM stream_variants s
               JOIN vod_items v ON v.id = s.vod_item_id
               WHERE s.source_id = ?
                 AND s.vod_item_id = (
                   SELECT MIN(r.vod_item_id) FROM stream_variants r
                   WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id)
                 AND v.genre IS NOT NULL AND v.genre <> ''
               GROUP BY v.genre
               ORDER BY COUNT(*) DESC, v.genre"#,
            source_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| FacetCount {
                name: r.name,
                count: r.count,
            })
            .collect())
    }

    /// Films per release decade, newest first, with up to four posters each.
    pub async fn movie_decades(&self, source_id: i64) -> Result<Vec<DecadeCount>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT d.decade as "decade!: i64", d.n as "count!: i64",
                      (SELECT group_concat(p, char(31)) FROM (
                         SELECT COALESCE(v2.poster_url, v2.logo_url) AS p
                         FROM stream_variants s2
                         JOIN vod_items v2 ON v2.id = s2.vod_item_id
                         WHERE s2.source_id = ?1
                           AND s2.vod_item_id = (
                             SELECT MIN(r.vod_item_id) FROM stream_variants r
                             WHERE r.entity_id = s2.entity_id AND r.source_id = s2.source_id)
                           AND v2.year / 10 * 10 = d.decade
                           AND COALESCE(v2.poster_url, v2.logo_url) IS NOT NULL
                         ORDER BY v2.poster_url IS NULL, v2.id
                         LIMIT 4)) as "posters?: String"
               FROM (
                 SELECT v.year / 10 * 10 AS decade, COUNT(*) AS n
                 FROM stream_variants s
                 JOIN vod_items v ON v.id = s.vod_item_id
                 WHERE s.source_id = ?1
                   AND s.vod_item_id = (
                     SELECT MIN(r.vod_item_id) FROM stream_variants r
                     WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id)
                   AND v.year BETWEEN 1888 AND 2100
                 GROUP BY v.year / 10
               ) d
               ORDER BY d.decade DESC"#,
            source_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| DecadeCount {
                decade: r.decade,
                count: r.count,
                poster_urls: posters(r.posters, 4),
            })
            .collect())
    }

    /// TMDB collections (sagas, trilogies) with at least two films in the
    /// playlist, known from the facts fetched for its films; largest first.
    pub async fn library_collections(
        &self,
        source_id: i64,
    ) -> Result<Vec<LibraryCollection>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT json_extract(f.facts, '$.collection.id') as "id!: i64",
                      json_extract(f.facts, '$.collection.name') as "name!: String",
                      COUNT(*) as "count!: i64",
                      group_concat(COALESCE(v.poster_url, v.logo_url), char(31)) as "posters?: String"
               FROM stream_variants s
               JOIN vod_items v ON v.id = s.vod_item_id
               JOIN entity_external_matches m
                      ON m.entity_id = s.entity_id AND m.namespace = 'tmdb:movie' AND m.status = 'matched'
               JOIN tmdb_title_facts f
                      ON f.media_type = 'movie' AND f.tmdb_id = CAST(m.external_id AS INTEGER)
               WHERE s.source_id = ?
                 AND s.vod_item_id = (
                   SELECT MIN(r.vod_item_id) FROM stream_variants r
                   WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id)
                 AND json_extract(f.facts, '$.collection.id') IS NOT NULL
               GROUP BY json_extract(f.facts, '$.collection.id')
               HAVING COUNT(*) >= 2
               ORDER BY COUNT(*) DESC, 2
               LIMIT 24"#,
            source_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| LibraryCollection {
                id: r.id,
                name: r.name,
                count: r.count,
                poster_urls: posters(r.posters, 3),
            })
            .collect())
    }
}
