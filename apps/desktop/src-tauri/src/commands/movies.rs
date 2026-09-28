//! Canonical movie API: the first `SourceRecord != CanonicalEntity !=
//! StreamVariant` vertical slice. Responses are secret-free canonical DTOs;
//! durable user state is addressed by entity, and Rust selects the variant to
//! play. Legacy `*_vod_*` commands remain for unmigrated surfaces and write
//! through to the same entity.

use nova_core::catalog::media::{CanonicalMediaDetail, CanonicalMediaSummary, MoviePlaybackTarget};
use nova_core::catalog::{EntityId, VariantId};
use nova_storage::{CanonicalRepository, MovieFilters, ProfileSettingsRepository};
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
    let category = normalize_filter(filters.category);
    let query = normalize_filter(filters.query);
    CanonicalRepository::new(&pool)
        .list_movies(
            source_id,
            limit,
            offset,
            &MovieFilters {
                category: category.as_deref(),
                query: query.as_deref(),
                favorites_only: filters.favorites_only,
                watchlist_only: filters.watchlist_only,
            },
            filters.sort,
        )
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
    let filters = filters.unwrap_or_default();
    let category = normalize_filter(filters.category);
    let query = normalize_filter(filters.query);
    CanonicalRepository::new(&pool)
        .count_movies(
            source_id,
            &MovieFilters {
                category: category.as_deref(),
                query: query.as_deref(),
                favorites_only: filters.favorites_only,
                watchlist_only: filters.watchlist_only,
            },
        )
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
    CanonicalRepository::new(&pool)
        .movie_detail(&self::entity_id(entity_id)?)
        .await?
        .ok_or_else(|| ApiError::NotFound("movie not found".into()))
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
