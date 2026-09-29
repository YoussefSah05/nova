//! Canonical movie API: the first `SourceRecord != CanonicalEntity !=
//! StreamVariant` vertical slice. Responses are secret-free canonical DTOs;
//! durable user state is addressed by entity, and Rust selects the variant to
//! play. Legacy `*_vod_*` commands remain for unmigrated surfaces and write
//! through to the same entity.

use nova_core::catalog::media::{CanonicalMediaDetail, CanonicalMediaSummary, MoviePlaybackTarget};
use nova_core::catalog::{EntityId, VariantId};
use nova_storage::{
    CanonicalRepository, DecadeCount, FacetCount, LibraryCollection, MovieFilters,
    ProfileSettingsRepository,
};
use tauri::State;

use crate::ipc::ApiError;
use crate::state::AppState;

use super::{normalize_filter, require_profile_pool, ContentFilters};

fn entity_id(value: String) -> Result<EntityId, ApiError> {
    EntityId::new(value).map_err(|_| ApiError::NotFound("movie not found".into()))
}

fn variant_id(value: Option<String>) -> Result<Option<VariantId>, ApiError> {
    value
        .map(|v| VariantId::new(v).map_err(|_| ApiError::NotFound("variant not found".into())))
        .transpose()
}

#[tauri::command]
pub async fn list_movies(
    profile_id: String,
    source_id: i64,
    limit: i64,
    offset: i64,
    filters: Option<ContentFilters>,
    state: State<'_, AppState>,
) -> Result<Vec<CanonicalMediaSummary>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let filters = filters.unwrap_or_default();
    let owned = OwnedMovieFilters::from(filters);
    CanonicalRepository::new(&pool)
        .list_movies(source_id, limit, offset, &owned.borrow(), owned.sort)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn count_movies(
    profile_id: String,
    source_id: i64,
    filters: Option<ContentFilters>,
    state: State<'_, AppState>,
) -> Result<i64, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let owned = OwnedMovieFilters::from(filters.unwrap_or_default());
    CanonicalRepository::new(&pool)
        .count_movies(source_id, &owned.borrow())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_movie(
    profile_id: String,
    entity_id: String,
    state: State<'_, AppState>,
) -> Result<CanonicalMediaDetail, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    // "Picked" follows the same ranking playback uses for this profile.
    let preferred = ProfileSettingsRepository::new(&pool)
        .get()
        .await
        .ok()
        .and_then(|s| s.preferred_audio_lang);
    CanonicalRepository::new(&pool)
        .movie_detail_with(&self::entity_id(entity_id)?, preferred.as_deref())
        .await?
        .ok_or_else(|| ApiError::NotFound("movie not found".into()))
}

/// Nova's genres among the playlist's films, most common first.
#[tauri::command]
pub async fn list_movie_genres(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<FacetCount>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    CanonicalRepository::new(&pool)
        .movie_genres(source_id)
        .await
        .map_err(Into::into)
}

/// Films per release decade, newest first, with a few posters each.
#[tauri::command]
pub async fn list_movie_decades(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<DecadeCount>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    CanonicalRepository::new(&pool)
        .movie_decades(source_id)
        .await
        .map_err(Into::into)
}

/// TMDB collections with at least two films in the playlist.
#[tauri::command]
pub async fn list_library_collections(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<LibraryCollection>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    CanonicalRepository::new(&pool)
        .library_collections(source_id)
        .await
        .map_err(Into::into)
}

/// Normalized owned copies of the browse filters (the repository borrows).
struct OwnedMovieFilters {
    category: Option<String>,
    query: Option<String>,
    genre: Option<String>,
    favorites_only: bool,
    watchlist_only: bool,
    year_from: Option<i64>,
    year_to: Option<i64>,
    collection_id: Option<i64>,
    sort: nova_core::ContentSort,
}

impl From<ContentFilters> for OwnedMovieFilters {
    fn from(f: ContentFilters) -> Self {
        Self {
            category: normalize_filter(f.category),
            query: normalize_filter(f.query),
            genre: normalize_filter(f.genre),
            favorites_only: f.favorites_only,
            watchlist_only: f.watchlist_only,
            year_from: f.year_from,
            year_to: f.year_to,
            collection_id: f.collection_id,
            sort: f.sort,
        }
    }
}

impl OwnedMovieFilters {
    fn borrow(&self) -> MovieFilters<'_> {
        MovieFilters {
            category: self.category.as_deref(),
            query: self.query.as_deref(),
            favorites_only: self.favorites_only,
            watchlist_only: self.watchlist_only,
            genre: self.genre.as_deref(),
            year_from: self.year_from,
            year_to: self.year_to,
            collection_id: self.collection_id,
        }
    }
}

#[tauri::command]
pub async fn set_movie_favorite(
    profile_id: String,
    entity_id: String,
    is_favorite: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let found = CanonicalRepository::new(&pool)
        .set_favorite(&self::entity_id(entity_id)?, is_favorite)
        .await?;
    found
        .then_some(())
        .ok_or_else(|| ApiError::NotFound("movie not found".into()))
}

#[tauri::command]
pub async fn set_movie_in_watchlist(
    profile_id: String,
    entity_id: String,
    in_watchlist: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let found = CanonicalRepository::new(&pool)
        .set_in_watchlist(&self::entity_id(entity_id)?, in_watchlist)
        .await?;
    found
        .then_some(())
        .ok_or_else(|| ApiError::NotFound("movie not found".into()))
}

/// Rust selects the variant (the requested one when valid for this entity,
/// otherwise the best by the variant selector: no recent failure, the
/// profile's preferred audio language, picture quality) and returns a
/// transient player locator.
#[tauri::command]
pub async fn resolve_movie_playback(
    profile_id: String,
    entity_id: String,
    variant_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<MoviePlaybackTarget, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let variant = self::variant_id(variant_id)?;
    let preferred = ProfileSettingsRepository::new(&pool)
        .get()
        .await
        .ok()
        .and_then(|s| s.preferred_audio_lang);
    CanonicalRepository::new(&pool)
        .resolve_playback_with(
            &self::entity_id(entity_id)?,
            variant.as_ref(),
            preferred.as_deref(),
            &[],
        )
        .await?
        .ok_or_else(|| ApiError::NotFound("no playable version is available".into()))
}
