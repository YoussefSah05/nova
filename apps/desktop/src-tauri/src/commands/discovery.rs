//! Discovery from the library (master plan M4.6 T2/T3): "More like this",
//! the rest of a collection, and "Trending in your library".
//!
//! TMDB suggests works; only titles present in the user's catalog are shown,
//! in TMDB's order. Unavailable suggestions never appear. Everything is
//! cached: related titles for a month, trending for a day (a few calls a day
//! at most), and nothing runs unless the user enabled TMDB.

use std::collections::HashMap;

use nova_core::catalog::media::CanonicalMediaSummary;
use nova_core::catalog::EntityId;
use nova_core::SeriesSummary;
use nova_enrich::tmdb_facts::{parse_collection, result_ids};
use nova_enrich::TmdbMediaKind;
use nova_storage::{
    CanonicalRepository, SeriesRepository, TitleArtMedia, TitleFactsRepository, TmdbCacheRepository,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use super::enrich::{CachedTmdb, TmdbFetch, TmdbRequest};
use super::require_profile_pool;
use super::title_art::{tmdb_access, tmdb_work, TitleArtSubject};
use super::title_facts::resolve_title_facts;
use crate::ipc::ApiError;
use crate::state::AppState;

const DAY: i64 = 24 * 3600;
const RELATED_TTL: i64 = 30 * DAY;
const TRENDING_TTL: i64 = DAY;
const RAIL_MAX: usize = 20;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedTitles {
    pub collection_name: Option<String>,
    /// Other films of the collection the library has, in release order.
    pub collection: Vec<CanonicalMediaSummary>,
    /// "More like this" (movie pages).
    pub movies: Vec<CanonicalMediaSummary>,
    /// "More like this" (series pages).
    pub series: Vec<SeriesSummary>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrendingSection {
    Movies,
    Series,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendingTitles {
    pub movies: Vec<CanonicalMediaSummary>,
    pub series: Vec<SeriesSummary>,
}

/// TMDB ids from a results page; nothing on errors (discovery is optional).
async fn ids_of(tmdb: &impl TmdbFetch, request: &TmdbRequest, ttl: i64) -> Vec<i64> {
    match tmdb.fetch_max_age(request, ttl).await {
        Ok(Some(body)) => result_ids(&body).unwrap_or_default(),
        Ok(None) => Vec::new(),
        Err(e) => {
            tracing::debug!(error = ?e, "discovery: TMDB unavailable");
            Vec::new()
        }
    }
}

/// Movies of the source matched to `tmdb_ids`, in that order, minus `skip`.
async fn library_movies(
    pool: &sqlx::SqlitePool,
    source_id: i64,
    tmdb_ids: &[i64],
    skip: Option<&str>,
) -> Result<Vec<CanonicalMediaSummary>, ApiError> {
    let matches = TitleFactsRepository::new(pool)
        .library_movies(tmdb_ids)
        .await?;
    let entities = in_tmdb_order(tmdb_ids, matches, |e: &EntityId| skip != Some(e.as_str()));
    let cards = CanonicalRepository::new(pool)
        .movies_by_ids(source_id, &entities)
        .await?;
    Ok(reorder(&entities, cards, |c| c.entity_id.clone()))
}

/// Series of the source matched to `tmdb_ids`, in that order, minus `skip`.
async fn library_series(
    pool: &sqlx::SqlitePool,
    source_id: i64,
    tmdb_ids: &[i64],
    skip: Option<&str>,
) -> Result<Vec<SeriesSummary>, ApiError> {
    let matches = TitleFactsRepository::new(pool)
        .library_series(source_id, tmdb_ids)
        .await?;
    let titles = in_tmdb_order(tmdb_ids, matches, |t: &String| skip != Some(t.as_str()));
    let cards = SeriesRepository::new(pool)
        .list_by_titles(source_id, &titles)
        .await?;
    Ok(reorder(&titles, cards, |c| c.series_title.clone()))
}

/// The library keys matched to `tmdb_ids`, following TMDB's order (several
/// library titles may share one work), capped to a rail.
pub(crate) fn in_tmdb_order<K: Clone + PartialEq>(
    tmdb_ids: &[i64],
    matches: Vec<(i64, K)>,
    keep: impl Fn(&K) -> bool,
) -> Vec<K> {
    let mut by_id: HashMap<i64, Vec<K>> = HashMap::new();
    for (id, key) in matches {
        by_id.entry(id).or_default().push(key);
    }
    let mut out: Vec<K> = Vec::new();
    for id in tmdb_ids {
        for key in by_id.remove(id).unwrap_or_default() {
            if keep(&key) && !out.contains(&key) && out.len() < RAIL_MAX {
                out.push(key);
            }
        }
    }
    out
}

fn reorder<K: PartialEq, T>(order: &[K], mut items: Vec<T>, key: impl Fn(&T) -> K) -> Vec<T> {
    items.sort_by_key(|item| {
        order
            .iter()
            .position(|k| *k == key(item))
            .unwrap_or(usize::MAX)
    });
    items
}

/// "More like this" and the collection of a movie or series page.
#[tauri::command]
pub async fn get_related_titles(
    profile_id: String,
    source_id: i64,
    subject: TitleArtSubject,
    state: State<'_, AppState>,
) -> Result<RelatedTitles, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let Some((media, tmdb_id)) = tmdb_work(&pool, &subject).await? else {
        return Ok(RelatedTitles::default());
    };
    let (client, language) = tmdb_access(&pool).await?;
    let Some(client) = client else {
        return Ok(RelatedTitles::default());
    };
    let cache = TmdbCacheRepository::new(&pool);
    let tmdb = CachedTmdb {
        client: &client,
        cache: &cache,
    };
    let kind = match media {
        TitleArtMedia::Movie => TmdbMediaKind::Movie,
        TitleArtMedia::Tv => TmdbMediaKind::Tv,
    };
    let recommended = ids_of(
        &tmdb,
        &TmdbRequest::Recommendations(kind, tmdb_id),
        RELATED_TTL,
    )
    .await;

    let mut related = RelatedTitles::default();
    match &subject {
        TitleArtSubject::Movie { entity_id } => {
            let skip = Some(entity_id.as_str());
            // The collection comes from the movie's facts (usually cached by
            // the page already).
            let facts = resolve_title_facts(
                &TitleFactsRepository::new(&pool),
                Some(&tmdb),
                media,
                tmdb_id,
                &language,
            )
            .await;
            if let Some(collection) = facts.and_then(|f| f.collection) {
                let request = TmdbRequest::Collection(collection.id);
                if let Ok(Some(body)) = tmdb.fetch_max_age(&request, RELATED_TTL).await {
                    if let Ok((name, parts)) = parse_collection(&body) {
                        related.collection = library_movies(&pool, source_id, &parts, skip).await?;
                        if !related.collection.is_empty() {
                            related.collection_name = Some(name).filter(|n| !n.is_empty());
                        }
                    }
                }
            }
            let mut movies = library_movies(&pool, source_id, &recommended, skip).await?;
            if movies.is_empty() {
                let similar =
                    ids_of(&tmdb, &TmdbRequest::Similar(kind, tmdb_id), RELATED_TTL).await;
                movies = library_movies(&pool, source_id, &similar, skip).await?;
            }
            // A film already shown in its collection is not suggested twice.
            movies.retain(|m| {
                !related
                    .collection
                    .iter()
                    .any(|c| c.entity_id == m.entity_id)
            });
            related.movies = movies;
        }
        TitleArtSubject::Series { series_title, .. } => {
            let skip = Some(series_title.as_str());
            let mut series = library_series(&pool, source_id, &recommended, skip).await?;
            if series.is_empty() {
                let similar =
                    ids_of(&tmdb, &TmdbRequest::Similar(kind, tmdb_id), RELATED_TTL).await;
                series = library_series(&pool, source_id, &similar, skip).await?;
            }
            related.series = series;
        }
    }
    Ok(related)
}

/// This week's TMDB trending titles that the library has.
#[tauri::command]
pub async fn get_trending_in_library(
    profile_id: String,
    source_id: i64,
    section: TrendingSection,
    state: State<'_, AppState>,
) -> Result<TrendingTitles, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let (client, _) = tmdb_access(&pool).await?;
    let Some(client) = client else {
        return Ok(TrendingTitles::default());
    };
    let cache = TmdbCacheRepository::new(&pool);
    let tmdb = CachedTmdb {
        client: &client,
        cache: &cache,
    };
    let kind = match section {
        TrendingSection::Movies => TmdbMediaKind::Movie,
        TrendingSection::Series => TmdbMediaKind::Tv,
    };
    // Two pages (40 works): enough for a library to intersect meaningfully.
    let mut ids = ids_of(&tmdb, &TmdbRequest::Trending(kind, 1), TRENDING_TTL).await;
    for id in ids_of(&tmdb, &TmdbRequest::Trending(kind, 2), TRENDING_TTL).await {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(match section {
        TrendingSection::Movies => TrendingTitles {
            movies: library_movies(&pool, source_id, &ids, None).await?,
            ..TrendingTitles::default()
        },
        TrendingSection::Series => TrendingTitles {
            series: library_series(&pool, source_id, &ids, None).await?,
            ..TrendingTitles::default()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_keys_follow_tmdb_order_without_duplicates() {
        let tmdb_ids = [30, 10, 20];
        let matches = vec![
            (10, "a".to_string()),
            (20, "b".to_string()),
            (30, "c".to_string()),
            (30, "c2".to_string()),
            (10, "a".to_string()),
            (99, "never".to_string()),
        ];
        let keys = in_tmdb_order(&tmdb_ids, matches, |k: &String| k != "b");
        assert_eq!(keys, vec!["c", "c2", "a"]);

        let many: Vec<(i64, i64)> = (1..=30).map(|i| (i, i)).collect();
        let ids: Vec<i64> = (1..=30).collect();
        assert_eq!(in_tmdb_order(&ids, many, |_| true).len(), RAIL_MAX);
    }

    #[test]
    fn cards_are_put_back_in_order() {
        let order = vec!["b", "a", "c"];
        let items = vec![("a", 1), ("c", 3), ("b", 2)];
        let sorted = reorder(&order, items, |(k, _)| *k);
        assert_eq!(sorted, vec![("b", 2), ("a", 1), ("c", 3)]);
    }
}
