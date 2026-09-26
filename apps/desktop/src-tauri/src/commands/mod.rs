use nova_core::{
    Channel, ContentSort, ContinueWatchingItem, ImportProgress, ImportStage, MediaKind,
    ParsedEntry, Profile, SeriesEpisode, SeriesSummary, Source, VodItem,
};
use nova_storage::{
    init_profile_db, CanonicalRepository, ChannelRepository, EpgSourcesRepository,
    ProfileRepository, ProfileSettingsRepository, SeriesRepository, SourceRepository,
    VodRepository, WatchHistoryRepository, WatchSnapshot,
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::ipc::ApiError;
use crate::state::AppState;

use std::time::{Duration, Instant};

pub mod caption_style;
pub mod enrich;
pub mod epg;
pub mod models;
pub mod movies;
pub mod playback;
pub mod profile_settings;
pub mod search;
pub mod settings;
pub mod transcription;

// ── Profile commands (Phase 1) ────────────────────────────────────────────────

#[tauri::command]
pub async fn list_profiles(state: State<'_, AppState>) -> Result<Vec<Profile>, ApiError> {
    ProfileRepository::new(&state.pool)
        .list()
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_profile(
    name: String,
    avatar_color: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Profile, ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::Conflict("name cannot be empty".into()));
    }
    let profile = ProfileRepository::new(&state.pool)
        .create(&name, &avatar_color)
        .await
        .map_err(ApiError::from)?;

    // Open (or create) the per-profile DB immediately
    ensure_profile_pool(&profile.id, &state, &app).await?;

    Ok(profile)
}

#[tauri::command]
pub async fn delete_profile(id: String, state: State<'_, AppState>) -> Result<(), ApiError> {
    ProfileRepository::new(&state.pool)
        .delete(&id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn select_profile(
    id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Profile, ApiError> {
    let repo = ProfileRepository::new(&state.pool);
    repo.update_last_used(&id).await.map_err(ApiError::from)?;
    let profile = repo.get(&id).await.map_err(ApiError::from)?;

    ensure_profile_pool(&id, &state, &app).await?;
    let pool = require_profile_pool(&id, &state).await?;
    let jobs = state.jobs_for_profile(&id, pool, app.clone()).await;
    *state.active_profile_id.write().await = Some(id);
    jobs.kick_all();
    Ok(profile)
}

/// Open the per-profile DB pool if it isn't already (no side effects — does *not*
/// mark the profile active or touch `last_used_at`). Used by `select_profile` and
/// by read-only aggregate commands that need a profile's own DB.
pub(crate) async fn ensure_profile_pool(
    profile_id: &str,
    state: &AppState,
    app: &AppHandle,
) -> Result<(), ApiError> {
    if state.profile_pool(profile_id).await.is_some() {
        return Ok(());
    }
    // Re-check under the open lock: a concurrent caller may have opened it.
    let _open = state.profile_open.lock().await;
    if state.profile_pool(profile_id).await.is_none() {
        open_profile_db(profile_id, state, app).await?;
    }
    Ok(())
}

/// At-a-glance summary for the profile picker: accent theme, source/item counts
/// and continue-watching state, aggregated from the profile's own DB. Opens that
/// DB if needed (warming the pool, so the subsequent `select_profile` is instant).
#[tauri::command]
pub async fn get_profile_overview(
    profile_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<nova_core::ProfileOverview, ApiError> {
    ensure_profile_pool(&profile_id, &state, &app).await?;
    let pool = require_profile_pool(&profile_id, &state).await?;

    let accent_color = ProfileSettingsRepository::new(&pool)
        .get()
        .await
        .map(|s| s.accent_color)
        .unwrap_or_else(|_| "amber".to_string());

    let sources = SourceRepository::new(&pool)
        .list()
        .await
        .unwrap_or_default();
    let source_count = sources.len() as i64;

    let channels = ChannelRepository::new(&pool);
    let vod = VodRepository::new(&pool);
    let series = SeriesRepository::new(&pool);
    let mut item_count = 0i64;
    for source in &sources {
        item_count += channels
            .count_filtered(source.id, None, None, None, None, false, false)
            .await
            .unwrap_or(0);
        item_count += vod
            .count_filtered(source.id, None, None, false, false)
            .await
            .unwrap_or(0);
        item_count += series
            .count_series(source.id, None, None, false, false)
            .await
            .unwrap_or(0);
    }

    let continuing = WatchHistoryRepository::new(&pool)
        .list_continue_watching(20)
        .await
        .unwrap_or_default();
    let continue_count = continuing.len() as i64;
    let last_title = continuing.first().map(|i| i.title.clone());

    Ok(nova_core::ProfileOverview {
        accent_color,
        source_count,
        item_count,
        continue_count,
        last_title,
    })
}

// ── Source commands (Phase 2) ─────────────────────────────────────────────────

#[tauri::command]
pub async fn list_sources(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Source>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    SourceRepository::new(&pool)
        .list()
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn add_source(
    profile_id: String,
    name: String,
    url: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<i64, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;

    // Detect Xtream vs plain M3U once, so the stored `kind` always matches the
    // parser actually used (credential parsing can still fail on a malformed
    // Xtream URL, in which case we fall back to the plain M3U parser).
    let xtream_creds = nova_xtream::detect(&url)
        .then(|| nova_xtream::XtreamCredentials::from_url(&url))
        .flatten();
    let kind = if xtream_creds.is_some() {
        "xtream"
    } else {
        "m3u"
    };

    let source = SourceRepository::new(&pool)
        .create(kind, &name, &url)
        .await
        .map_err(ApiError::from)?;

    let source_id = source.id;
    state
        .jobs_for_profile(&profile_id, pool, app)
        .await
        .import(source_id, ImportMode::Append)
        .await
        .map_err(ApiError::Internal)?;
    Ok(source_id)
}

/// How `run_import` treats existing rows for the source.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportMode {
    /// Fresh add: only insert (the source has no rows yet).
    Append,
    /// Refresh: snapshot user state, clear the source's rows, re-import, then
    /// restore favorites/watchlist + movie/live resume — all in one transaction.
    Replace,
}

/// Re-import an existing source from its provider, replacing its catalog while
/// preserving Favorites/Watchlist and movie/live Continue-Watching. Reuses the
/// same `playlist:import:progress` events as the initial import.
#[tauri::command]
pub async fn refresh_source(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    SourceRepository::new(&pool)
        .get(source_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("source {source_id} not found")))?;
    state
        .jobs_for_profile(&profile_id, pool, app)
        .await
        .import(source_id, ImportMode::Replace)
        .await
        .map_err(ApiError::Internal)?;
    Ok(())
}

/// Worker for add + refresh: detect Xtream vs M3U, register the EPG feed,
/// stream the provider data, and commit or roll back atomically.
pub(crate) async fn run_source_import(
    pool: std::sync::Arc<sqlx::SqlitePool>,
    app: AppHandle,
    source: Source,
    mode: ImportMode,
) -> bool {
    let source_id = source.id;
    let url = source.url;
    let xtream_creds = nova_xtream::detect(&url)
        .then(|| nova_xtream::XtreamCredentials::from_url(&url))
        .flatten();

    // EPG seeding (Phase 4): record any provider-supplied XMLTV URL alongside
    // the source so a later `epg_refresh` can pull programmes for it. Xtream's
    // is known immediately; M3U's `url-tvg` is only revealed once the parser
    // has seen the `#EXTM3U` header line, so we wait for the metadata oneshot
    // in a separate task.
    let (rx, m3u_metadata_rx) = match &xtream_creds {
        Some(creds) => {
            let epg_url = creds.xmltv_url();
            if let Err(e) = EpgSourcesRepository::new(&pool)
                .upsert(source_id, &epg_url, "xtream")
                .await
            {
                tracing::warn!(source_id, error = %e, "failed to register Xtream xmltv.php URL");
            }
            (nova_xtream::fetch(creds).await, None)
        }
        None => {
            let pl = nova_m3u::parse(&url).await;
            (pl.entries, Some(pl.metadata))
        }
    };

    if let Some(meta_rx) = m3u_metadata_rx {
        let pool_meta = std::sync::Arc::clone(&pool);
        tokio::spawn(async move {
            if let Ok(meta) = meta_rx.await {
                if let Some(url_tvg) = meta.url_tvg {
                    if let Err(e) = EpgSourcesRepository::new(&pool_meta)
                        .upsert(source_id, url_tvg.trim(), "m3u_url_tvg")
                        .await
                    {
                        tracing::warn!(source_id, error = %e, "failed to register M3U url-tvg");
                    } else {
                        tracing::info!(source_id, "M3U url-tvg registered");
                    }
                }
            }
        });
    }

    run_import(
        source_id,
        rx,
        pool,
        |progress| {
            let _ = app.emit("playlist:import:progress", progress);
        },
        mode,
    )
    .await
}

// ── Browse commands (Phase 2) ─────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelFilters {
    country: Option<String>,
    category: Option<String>,
    region: Option<String>,
    query: Option<String>,
    #[serde(default)]
    favorites_only: bool,
    #[serde(default)]
    watchlist_only: bool,
    #[serde(default)]
    sort: ContentSort,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentFilters {
    category: Option<String>,
    query: Option<String>,
    #[serde(default)]
    favorites_only: bool,
    #[serde(default)]
    watchlist_only: bool,
    #[serde(default)]
    sort: ContentSort,
}

#[tauri::command]
pub async fn list_channels(
    profile_id: String,
    source_id: i64,
    limit: i64,
    offset: i64,
    filters: Option<ChannelFilters>,
    state: State<'_, AppState>,
) -> Result<Vec<Channel>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let filters = filters.unwrap_or_default();
    let country = normalize_filter(filters.country);
    let category = normalize_filter(filters.category);
    let region = normalize_filter(filters.region);
    let query = normalize_filter(filters.query);
    ChannelRepository::new(&pool)
        .list(
            source_id,
            limit,
            offset,
            country.as_deref(),
            category.as_deref(),
            region.as_deref(),
            query.as_deref(),
            filters.favorites_only,
            filters.watchlist_only,
            filters.sort,
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn count_channels(
    profile_id: String,
    source_id: i64,
    filters: Option<ChannelFilters>,
    state: State<'_, AppState>,
) -> Result<i64, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let filters = filters.unwrap_or_default();
    let country = normalize_filter(filters.country);
    let category = normalize_filter(filters.category);
    let region = normalize_filter(filters.region);
    let query = normalize_filter(filters.query);
    ChannelRepository::new(&pool)
        .count_filtered(
            source_id,
            country.as_deref(),
            category.as_deref(),
            region.as_deref(),
            query.as_deref(),
            filters.favorites_only,
            filters.watchlist_only,
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_channel_countries(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<String>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ChannelRepository::new(&pool)
        .countries(source_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_channel_categories(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<String>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ChannelRepository::new(&pool)
        .categories(source_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_channel_regions(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<String>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ChannelRepository::new(&pool)
        .regions(source_id)
        .await
        .map_err(Into::into)
}

/// Live brand-family bundles (one card per brand) for the live wall, filtered by
/// type (`category`) + `region` + free text.
#[tauri::command]
pub async fn list_channel_groups(
    profile_id: String,
    source_id: i64,
    limit: i64,
    offset: i64,
    filters: Option<ChannelFilters>,
    state: State<'_, AppState>,
) -> Result<Vec<nova_core::ChannelGroup>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let filters = filters.unwrap_or_default();
    let category = normalize_filter(filters.category);
    let region = normalize_filter(filters.region);
    let query = normalize_filter(filters.query);
    ChannelRepository::new(&pool)
        .list_groups(
            source_id,
            category.as_deref(),
            region.as_deref(),
            query.as_deref(),
            filters.favorites_only,
            filters.watchlist_only,
            limit,
            offset,
        )
        .await
        .map_err(Into::into)
}

/// All channels of one brand-family (for the bundle start screen / cinema layer 1).
#[tauri::command]
pub async fn list_channels_in_group(
    profile_id: String,
    source_id: i64,
    group: String,
    state: State<'_, AppState>,
) -> Result<Vec<Channel>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ChannelRepository::new(&pool)
        .list_in_group(source_id, &group)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_vod(
    profile_id: String,
    source_id: i64,
    limit: i64,
    offset: i64,
    filters: Option<ContentFilters>,
    state: State<'_, AppState>,
) -> Result<Vec<VodItem>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let filters = filters.unwrap_or_default();
    let category = normalize_filter(filters.category);
    let query = normalize_filter(filters.query);
    VodRepository::new(&pool)
        .list(
            source_id,
            limit,
            offset,
            category.as_deref(),
            query.as_deref(),
            filters.favorites_only,
            filters.watchlist_only,
            filters.sort,
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn count_vod(
    profile_id: String,
    source_id: i64,
    filters: Option<ContentFilters>,
    state: State<'_, AppState>,
) -> Result<i64, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let filters = filters.unwrap_or_default();
    let category = normalize_filter(filters.category);
    let query = normalize_filter(filters.query);
    VodRepository::new(&pool)
        .count_filtered(
            source_id,
            category.as_deref(),
            query.as_deref(),
            filters.favorites_only,
            filters.watchlist_only,
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_vod_categories(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<String>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    VodRepository::new(&pool)
        .categories(source_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_series(
    profile_id: String,
    source_id: i64,
    limit: Option<i64>,
    offset: Option<i64>,
    filters: Option<ContentFilters>,
    state: State<'_, AppState>,
) -> Result<Vec<SeriesSummary>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let filters = filters.unwrap_or_default();
    let category = normalize_filter(filters.category);
    let query = normalize_filter(filters.query);
    let limit = limit.unwrap_or(i64::MAX);
    let offset = offset.unwrap_or(0);
    SeriesRepository::new(&pool)
        .list_series(
            source_id,
            category.as_deref(),
            query.as_deref(),
            filters.favorites_only,
            filters.watchlist_only,
            filters.sort,
            limit,
            offset,
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn count_series(
    profile_id: String,
    source_id: i64,
    filters: Option<ContentFilters>,
    state: State<'_, AppState>,
) -> Result<i64, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let filters = filters.unwrap_or_default();
    let category = normalize_filter(filters.category);
    let query = normalize_filter(filters.query);
    SeriesRepository::new(&pool)
        .count_series(
            source_id,
            category.as_deref(),
            query.as_deref(),
            filters.favorites_only,
            filters.watchlist_only,
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_series_categories(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<String>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    SeriesRepository::new(&pool)
        .categories(source_id)
        .await
        .map_err(Into::into)
}

/// List a series' episodes, fetching them lazily for Xtream providers.
///
/// Xtream import stores one catalog *sentinel* per series (`stream_url` of
/// `xtream://series/{id}`) instead of episodes, to keep import fast. The first
/// time a series is opened we call `get_series_info`, persist the real episodes
/// (replacing the sentinel), and return them; subsequent opens read the cache.
/// M3U sources already carry real episodes inline, so this is a passthrough.
#[tauri::command]
pub async fn list_series_episodes(
    profile_id: String,
    source_id: i64,
    series_title: String,
    state: State<'_, AppState>,
) -> Result<Vec<SeriesEpisode>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    fetch_series_episodes(&pool, source_id, &series_title).await
}

/// Episodes of the series the given episode belongs to — for the player's
/// cinema-mode episode browser and autoplay-next. Resolves the episode's
/// `source_id` + `series_title`, then reuses the same lazy Xtream fetch as
/// [`list_series_episodes`]. Returns `[]` if `episode_id` isn't an episode.
#[tauri::command]
pub async fn list_player_episodes(
    profile_id: String,
    episode_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<SeriesEpisode>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let Some(ep) = SeriesRepository::new(&pool).get_episode(episode_id).await? else {
        return Ok(Vec::new());
    };
    fetch_series_episodes(&pool, ep.source_id, &ep.series_title).await
}

/// List a series' episodes, fetching them lazily for Xtream providers.
///
/// Xtream import stores one catalog *sentinel* per series (`stream_url` of
/// `xtream://series/{id}`) instead of episodes, to keep import fast. The first
/// time a series is opened we call `get_series_info`, persist the real episodes
/// (replacing the sentinel), and return them; subsequent opens read the cache.
/// M3U sources already carry real episodes inline, so this is a passthrough.
async fn fetch_series_episodes(
    pool: &sqlx::SqlitePool,
    source_id: i64,
    series_title: &str,
) -> Result<Vec<SeriesEpisode>, ApiError> {
    let repo = SeriesRepository::new(pool);
    let existing = repo.list_episodes(source_id, series_title).await?;

    // Find a not-yet-fetched Xtream sentinel.
    let sentinel = existing
        .iter()
        .find_map(|e| e.stream_url.strip_prefix("xtream://series/"));
    let Some(series_id_str) = sentinel else {
        // M3U, or Xtream already fetched — return as-is.
        return Ok(existing);
    };
    let series_id: i64 = series_id_str
        .parse()
        .map_err(|_| ApiError::Internal(format!("bad series sentinel: {series_id_str}")))?;

    let source = SourceRepository::new(pool)
        .get(source_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("source {source_id} not found")))?;
    let creds = nova_xtream::XtreamCredentials::from_url(&source.url).ok_or_else(|| {
        ApiError::Internal("series episodes require an Xtream source".to_string())
    })?;

    let fetched = nova_xtream::fetch_series_info(&creds, series_id)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to load episodes: {e}")))?;

    let rows: Vec<SeriesEpisode> = fetched
        .into_iter()
        .map(|ep| SeriesEpisode {
            id: 0,
            source_id,
            series_title: series_title.to_string(),
            category: existing.first().and_then(|e| e.category.clone()),
            season: ep.season,
            episode: ep.episode,
            episode_title: ep.title,
            // Prefer the per-episode still; fall back to the series cover.
            logo_url: ep
                .still_url
                .or_else(|| existing.first().and_then(|e| e.logo_url.clone())),
            stream_url: ep.stream_url,
            added_at: None,
        })
        .collect();

    // Nothing came back (e.g. provider returned an empty info blob) — keep the
    // sentinel so a later open can retry, and report empty.
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    repo.replace_series_episodes(source_id, series_title, &rows)
        .await?;
    repo.list_episodes(source_id, series_title)
        .await
        .map_err(Into::into)
}

// ── Favourites + Continue Watching (Phase 3) ──────────────────────────────────

#[tauri::command]
pub async fn set_channel_favorite(
    profile_id: String,
    channel_id: i64,
    is_favorite: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ChannelRepository::new(&pool)
        .set_favorite(channel_id, is_favorite)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_vod_favorite(
    profile_id: String,
    vod_id: i64,
    is_favorite: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    // Written through to the owning canonical entity by the repository.
    VodRepository::new(&pool)
        .set_favorite(vod_id, is_favorite)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_series_favorite(
    profile_id: String,
    source_id: i64,
    series_title: String,
    is_favorite: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    SeriesRepository::new(&pool)
        .set_favorite(source_id, &series_title, is_favorite)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_channel_in_watchlist(
    profile_id: String,
    channel_id: i64,
    in_watchlist: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ChannelRepository::new(&pool)
        .set_in_watchlist(channel_id, in_watchlist)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_vod_in_watchlist(
    profile_id: String,
    vod_id: i64,
    in_watchlist: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    // Written through to the owning canonical entity by the repository.
    VodRepository::new(&pool)
        .set_in_watchlist(vod_id, in_watchlist)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_series_in_watchlist(
    profile_id: String,
    source_id: i64,
    series_title: String,
    in_watchlist: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    SeriesRepository::new(&pool)
        .set_in_watchlist(source_id, &series_title, in_watchlist)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_continue_watching(
    profile_id: String,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<ContinueWatchingItem>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    WatchHistoryRepository::new(&pool)
        .list_continue_watching(limit.unwrap_or(20))
        .await
        .map_err(Into::into)
}

// ── Import worker ─────────────────────────────────────────────────────────────

const BATCH: usize = 1_000;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

async fn run_import(
    source_id: i64,
    mut rx: tokio::sync::mpsc::Receiver<Result<ParsedEntry, String>>,
    pool: std::sync::Arc<sqlx::SqlitePool>,
    emit: impl Fn(ImportProgress),
    mode: ImportMode,
) -> bool {
    let started_at = Instant::now();
    let mut stats = ImportStats::default();
    let mut errors: Vec<String> = Vec::new();

    // Emit immediately so the UI transitions to the progress screen before
    // the first chunk even arrives.
    emit_import_progress(
        &emit,
        source_id,
        ImportStage::Connecting,
        &stats,
        None,
        None,
        false,
    );

    let mut live_buf: Vec<Channel> = Vec::with_capacity(BATCH);
    let mut vod_buf: Vec<VodItem> = Vec::with_capacity(BATCH);
    let mut series_buf: Vec<SeriesEpisode> = Vec::with_capacity(BATCH);
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
            emit_import_progress(
                &emit,
                source_id,
                ImportStage::Failed,
                &stats,
                Some(e.to_string()),
                None,
                true,
            );
            return false;
        }
    };

    // Refresh: snapshot user state and clear the source's rows up front, inside
    // this transaction, so the old catalog stays visible to the UI until commit.
    let replace_snapshot = if mode == ImportMode::Replace {
        match snapshot_and_clear(&mut tx, source_id).await {
            Ok(snap) => Some(snap),
            Err(e) => {
                stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
                emit_import_progress(
                    &emit,
                    source_id,
                    ImportStage::Failed,
                    &stats,
                    Some(e.to_string()),
                    None,
                    true,
                );
                return false;
            }
        }
    } else {
        None
    };

    loop {
        let wait_started = Instant::now();
        let result = match tokio::time::timeout(IDLE_TIMEOUT, rx.recv()).await {
            Ok(Some(result)) => result,
            Ok(None) => break, // all producers dropped — import finished
            Err(_) => {
                stats.receive_wait_ms += wait_started.elapsed().as_millis() as u64;
                stats.failed_count += 1;
                stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
                let msg = format!(
                    "import stalled after {}s with no new data",
                    IDLE_TIMEOUT.as_secs()
                );
                tracing::warn!(
                    source_id,
                    parsed = stats.parsed,
                    inserted = stats.inserted,
                    failed = stats.failed_count,
                    "{msg}"
                );
                errors.push(msg);
                break;
            }
        };
        stats.receive_wait_ms += wait_started.elapsed().as_millis() as u64;

        let entry = match result {
            Ok(e) => e,
            Err(e) => {
                stats.failed_count += 1;
                errors.push(e.clone());
                stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
                emit_import_progress(
                    &emit,
                    source_id,
                    ImportStage::Receiving,
                    &stats,
                    None,
                    Some(e),
                    false,
                );
                continue;
            }
        };
        stats.parsed += 1;
        match entry.kind {
            MediaKind::Live => stats.live_count += 1,
            MediaKind::Movie => stats.movie_count += 1,
            MediaKind::Series => stats.series_count += 1,
        }
        push_entry(
            &entry,
            source_id,
            &mut live_buf,
            &mut vod_buf,
            &mut series_buf,
        );

        if let Err(e) = flush_ready_buffers(
            &mut tx,
            &mut live_buf,
            &mut vod_buf,
            &mut series_buf,
            &mut stats,
        )
        .await
        {
            stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
            emit_import_progress(
                &emit,
                source_id,
                ImportStage::Failed,
                &stats,
                Some(e.to_string()),
                None,
                true,
            );
            return false;
        }

        // Emit progress every 500 entries for responsive feedback without
        // saturating the frontend event bridge on very large playlists.
        if stats.parsed.is_multiple_of(500) {
            stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
            emit_import_progress(
                &emit,
                source_id,
                ImportStage::Receiving,
                &stats,
                None,
                warning_from_errors(&errors),
                false,
            );
        }
    }

    // Any endpoint failure or receive timeout makes a replacement incomplete.
    // Dropping this transaction keeps the previous catalog and user state intact.
    if mode == ImportMode::Replace && !errors.is_empty() {
        stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_import_progress(
            &emit,
            source_id,
            ImportStage::Failed,
            &stats,
            Some("Refresh incomplete; previous catalog retained".to_string()),
            warning_from_errors(&errors),
            true,
        );
        return false;
    }

    if stats.parsed == 0 {
        let error = if errors.is_empty() {
            "import completed without any playlist entries".to_string()
        } else {
            errors.join("; ")
        };
        stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_import_progress(
            &emit,
            source_id,
            ImportStage::Failed,
            &stats,
            Some(error),
            None,
            true,
        );
        return false;
    }

    stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
    emit_import_progress(
        &emit,
        source_id,
        ImportStage::Persisting,
        &stats,
        None,
        warning_from_errors(&errors),
        false,
    );

    if let Err(e) = flush_all_buffers(
        &mut tx,
        &mut live_buf,
        &mut vod_buf,
        &mut series_buf,
        &mut stats,
    )
    .await
    {
        stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_import_progress(
            &emit,
            source_id,
            ImportStage::Failed,
            &stats,
            Some(e.to_string()),
            None,
            true,
        );
        return false;
    }

    // Refresh: re-apply the snapshotted user state onto the new rows.
    if let Some(snap) = replace_snapshot {
        if let Err(e) = restore_user_state(&mut tx, source_id, &snap).await {
            stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
            emit_import_progress(
                &emit,
                source_id,
                ImportStage::Failed,
                &stats,
                Some(e.to_string()),
                None,
                true,
            );
            return false;
        }
    }

    // Catalog compiler: bind this generation's movie records to canonical
    // entities in the same transaction, so a failed import leaves the previous
    // bindings (and all entity-owned user state) intact.
    if let Err(e) = CanonicalRepository::compile_movies_tx(&mut tx, source_id).await {
        stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_import_progress(
            &emit,
            source_id,
            ImportStage::Failed,
            &stats,
            Some(e.to_string()),
            None,
            true,
        );
        return false;
    }

    // Deterministic entity resolution across every source of the profile,
    // still in the import transaction: new or re-listed records merge into
    // the entity (and user state) they provably belong to, or stay separate.
    if let Err(e) = CanonicalRepository::resolve_movies_tx(&mut tx).await {
        stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_import_progress(
            &emit,
            source_id,
            ImportStage::Failed,
            &stats,
            Some(e.to_string()),
            None,
            true,
        );
        return false;
    }

    if let Err(e) = SourceRepository::touch_refreshed_tx(&mut tx, source_id).await {
        stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_import_progress(
            &emit,
            source_id,
            ImportStage::Failed,
            &stats,
            Some(e.to_string()),
            None,
            true,
        );
        return false;
    }

    let commit_started = Instant::now();
    if let Err(e) = tx.commit().await {
        stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_import_progress(
            &emit,
            source_id,
            ImportStage::Failed,
            &stats,
            Some(e.to_string()),
            None,
            true,
        );
        return false;
    }
    stats.persist_ms += commit_started.elapsed().as_millis() as u64;
    stats.elapsed_ms = started_at.elapsed().as_millis() as u64;

    tracing::info!(
        source_id,
        total_ms = stats.elapsed_ms,
        fetch_parse_wait_ms = stats.receive_wait_ms,
        db_persist_ms = stats.persist_ms,
        post_processing_ms = 0_u64,
        parsed = stats.parsed,
        inserted = stats.inserted,
        live = stats.live_count,
        movies = stats.movie_count,
        series = stats.series_count,
        failed = stats.failed_count,
        skipped = stats.skipped_count,
        "playlist import complete"
    );

    emit_import_progress(
        &emit,
        source_id,
        ImportStage::Complete,
        &stats,
        None,
        warning_from_errors(&errors),
        true,
    );
    true
}

#[derive(Default)]
struct ImportStats {
    parsed: u64,
    inserted: u64,
    live_count: u64,
    movie_count: u64,
    series_count: u64,
    failed_count: u64,
    skipped_count: u64,
    elapsed_ms: u64,
    receive_wait_ms: u64,
    persist_ms: u64,
}

fn emit_import_progress(
    emit: &impl Fn(ImportProgress),
    source_id: i64,
    stage: ImportStage,
    stats: &ImportStats,
    error: Option<String>,
    warning: Option<String>,
    done: bool,
) {
    emit(ImportProgress {
        source_id,
        stage,
        parsed: stats.parsed,
        inserted: stats.inserted,
        live_count: stats.live_count,
        movie_count: stats.movie_count,
        series_count: stats.series_count,
        failed_count: stats.failed_count,
        skipped_count: stats.skipped_count,
        elapsed_ms: stats.elapsed_ms,
        done,
        error,
        warning,
    });
}

fn warning_from_errors(errors: &[String]) -> Option<String> {
    match errors.len() {
        0 => None,
        1 => Some(format!("Imported with one nonfatal issue: {}", errors[0])),
        n => Some(format!(
            "Imported with {n} nonfatal issues; see logs for details."
        )),
    }
}

async fn flush_ready_buffers(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    live_buf: &mut Vec<Channel>,
    vod_buf: &mut Vec<VodItem>,
    series_buf: &mut Vec<SeriesEpisode>,
    stats: &mut ImportStats,
) -> Result<(), nova_storage::StorageError> {
    if live_buf.len() >= BATCH {
        flush_live(tx, live_buf, stats).await?;
    }
    if vod_buf.len() >= BATCH {
        flush_vod(tx, vod_buf, stats).await?;
    }
    if series_buf.len() >= BATCH {
        flush_series(tx, series_buf, stats).await?;
    }
    Ok(())
}

async fn flush_all_buffers(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    live_buf: &mut Vec<Channel>,
    vod_buf: &mut Vec<VodItem>,
    series_buf: &mut Vec<SeriesEpisode>,
    stats: &mut ImportStats,
) -> Result<(), nova_storage::StorageError> {
    flush_live(tx, live_buf, stats).await?;
    flush_vod(tx, vod_buf, stats).await?;
    flush_series(tx, series_buf, stats).await?;
    Ok(())
}

async fn flush_live(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    live_buf: &mut Vec<Channel>,
    stats: &mut ImportStats,
) -> Result<(), nova_storage::StorageError> {
    if live_buf.is_empty() {
        return Ok(());
    }
    let started = Instant::now();
    ChannelRepository::insert_batch_tx(tx, live_buf).await?;
    stats.persist_ms += started.elapsed().as_millis() as u64;
    stats.inserted += live_buf.len() as u64;
    live_buf.clear();
    Ok(())
}

async fn flush_vod(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    vod_buf: &mut Vec<VodItem>,
    stats: &mut ImportStats,
) -> Result<(), nova_storage::StorageError> {
    if vod_buf.is_empty() {
        return Ok(());
    }
    let started = Instant::now();
    VodRepository::insert_batch_tx(tx, vod_buf).await?;
    stats.persist_ms += started.elapsed().as_millis() as u64;
    stats.inserted += vod_buf.len() as u64;
    vod_buf.clear();
    Ok(())
}

async fn flush_series(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    series_buf: &mut Vec<SeriesEpisode>,
    stats: &mut ImportStats,
) -> Result<(), nova_storage::StorageError> {
    if series_buf.is_empty() {
        return Ok(());
    }
    let started = Instant::now();
    SeriesRepository::insert_batch_tx(tx, series_buf).await?;
    stats.persist_ms += started.elapsed().as_millis() as u64;
    stats.inserted += series_buf.len() as u64;
    series_buf.clear();
    Ok(())
}

/// User state captured before a refresh clears a source's catalog, so it can be
/// restored onto the re-imported rows. Flags are `(key, is_favorite,
/// is_in_watchlist)` — key is `stream_url` for channels/vod and `series_title`
/// for series.
struct ReplaceSnapshot {
    channel_flags: Vec<(String, bool, bool)>,
    vod_flags: Vec<(String, bool, bool)>,
    series_flags: Vec<(String, bool, bool)>,
    watch: Vec<WatchSnapshot>,
}

/// Snapshot favorites/watchlist + movie/live resume, then delete the source's
/// rows. Runs inside the import transaction (order matters: snapshot watch
/// history and delete its rows while the media rows still exist).
async fn snapshot_and_clear(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    source_id: i64,
) -> Result<ReplaceSnapshot, nova_storage::StorageError> {
    let channel_flags = ChannelRepository::snapshot_flags_tx(tx, source_id).await?;
    let vod_flags = VodRepository::snapshot_flags_tx(tx, source_id).await?;
    let series_flags = SeriesRepository::snapshot_series_flags_tx(tx, source_id).await?;
    let watch = WatchHistoryRepository::snapshot_for_source_tx(tx, source_id).await?;

    WatchHistoryRepository::delete_for_source_media_tx(tx, source_id).await?;
    ChannelRepository::delete_by_source_tx(tx, source_id).await?;
    VodRepository::delete_by_source_tx(tx, source_id).await?;
    SeriesRepository::delete_by_source_tx(tx, source_id).await?;

    Ok(ReplaceSnapshot {
        channel_flags,
        vod_flags,
        series_flags,
        watch,
    })
}

/// Re-apply a [`ReplaceSnapshot`] onto the freshly re-imported rows.
async fn restore_user_state(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    source_id: i64,
    snap: &ReplaceSnapshot,
) -> Result<(), nova_storage::StorageError> {
    ChannelRepository::restore_flags_tx(tx, source_id, &snap.channel_flags).await?;
    VodRepository::restore_flags_tx(tx, source_id, &snap.vod_flags).await?;
    SeriesRepository::apply_series_flags_tx(tx, source_id, &snap.series_flags).await?;
    WatchHistoryRepository::restore_tx(tx, source_id, &snap.watch).await?;
    Ok(())
}

fn push_entry(
    e: &ParsedEntry,
    source_id: i64,
    live: &mut Vec<Channel>,
    vod: &mut Vec<VodItem>,
    series: &mut Vec<SeriesEpisode>,
) {
    match e.kind {
        MediaKind::Live => {
            let name = nova_core::polish_title(&e.clean_title);
            let region = e
                .country_prefix
                .as_deref()
                .and_then(nova_core::region_of)
                .map(str::to_string);
            live.push(Channel {
                id: 0,
                source_id,
                tvg_id: e.tvg_id.clone(),
                channel_group: Some(nova_core::clean::channel_brand(&name)),
                name,
                country: e.country_prefix.clone(),
                category: e.category.clone(),
                raw_group: Some(e.group_title.clone()),
                logo_url: e.logo_url.clone(),
                stream_url: e.stream_url.clone(),
                is_favorite: false,
                is_in_watchlist: false,
                last_played_at: None,
                added_at: None,
                dedup_group: None,
                region,
            })
        }
        MediaKind::Movie => vod.push(VodItem {
            id: 0,
            source_id,
            name: e.name.clone(),
            clean_title: Some(nova_core::polish_title(&e.clean_title)),
            year: e.year.map(|y| y as i64),
            logo_url: e.logo_url.clone(),
            stream_url: e.stream_url.clone(),
            category: e.category.clone(),
            is_favorite: false,
            is_in_watchlist: false,
            added_at: None,
            dedup_group: None,
            canonical_title: None,
            poster_url: None,
            backdrop_url: None,
            overview: None,
            upstream_id: e.upstream_id.clone(),
            provider_tmdb_id: e.provider_tmdb_id,
        }),
        MediaKind::Series => series.push(SeriesEpisode {
            id: 0,
            source_id,
            series_title: nova_core::polish_title(
                e.series_title.as_deref().unwrap_or(&e.clean_title),
            ),
            category: e.category.clone(),
            season: e.season.unwrap_or(1) as i64,
            episode: e.episode.unwrap_or(1) as i64,
            episode_title: None,
            logo_url: e.logo_url.clone(),
            stream_url: e.stream_url.clone(),
            added_at: None,
        }),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn normalize_filter(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub(crate) async fn require_profile_pool(
    profile_id: &str,
    state: &AppState,
) -> Result<std::sync::Arc<sqlx::SqlitePool>, ApiError> {
    state
        .profile_pool(profile_id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("profile pool not open: {profile_id}")))
}

async fn open_profile_db(
    profile_id: &str,
    state: &AppState,
    app: &AppHandle,
) -> Result<(), ApiError> {
    use tauri::Manager;
    let root = match crate::validation_data_dir() {
        Some(root) => root,
        None => app
            .path()
            .app_data_dir()
            .map_err(|e| ApiError::Internal(e.to_string()))?,
    };
    let data_dir = root.join("profiles").join(profile_id);
    std::fs::create_dir_all(&data_dir).map_err(|e| ApiError::Internal(e.to_string()))?;
    let db_path = data_dir.join("profile.db");
    let pool = init_profile_db(
        db_path
            .to_str()
            .ok_or_else(|| ApiError::Internal("non-utf8 path".into()))?,
    )
    .await
    .map_err(ApiError::from)?;
    state
        .insert_profile_pool(profile_id.to_string(), pool)
        .await;
    Ok(())
}

#[cfg(test)]
mod stabilization_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn entry(name: &str) -> ParsedEntry {
        ParsedEntry {
            kind: MediaKind::Movie,
            name: name.into(),
            clean_title: name.into(),
            group_title: String::new(),
            stream_url: format!("https://example.invalid/{name}.mp4"),
            tvg_id: None,
            logo_url: None,
            country_prefix: None,
            year: None,
            series_title: None,
            season: None,
            episode: None,
            category: None,
            upstream_id: None,
            provider_tmdb_id: None,
        }
    }

    async fn import(
        pool: Arc<sqlx::SqlitePool>,
        id: i64,
        rows: Vec<Result<ParsedEntry, String>>,
        mode: ImportMode,
    ) -> ImportProgress {
        let (tx, rx) = tokio::sync::mpsc::channel(rows.len().max(1));
        for row in rows {
            tx.send(row).await.unwrap();
        }
        drop(tx);
        let events = Mutex::new(Vec::new());
        run_import(id, rx, pool, |p| events.lock().unwrap().push(p), mode).await;
        let events = events.into_inner().unwrap();
        assert_eq!(events.iter().filter(|p| p.done).count(), 1);
        events.last().unwrap().clone()
    }

    #[tokio::test]
    async fn failed_refresh_preserves_catalog_flags_and_resume() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(
            init_profile_db(dir.path().join("profile.db").to_str().unwrap())
                .await
                .unwrap(),
        );
        let id = SourceRepository::new(&pool)
            .create("m3u", "test", "https://example.invalid/list.m3u")
            .await
            .unwrap()
            .id;
        import(
            pool.clone(),
            id,
            vec![Ok(entry("original"))],
            ImportMode::Append,
        )
        .await;
        let vod = VodRepository::new(&pool);
        let original = vod
            .list(id, 10, 0, None, None, false, false, ContentSort::Default)
            .await
            .unwrap()[0]
            .clone();
        vod.set_favorite(original.id, true).await.unwrap();
        WatchHistoryRepository::new(&pool)
            .record_progress("vod", original.id, 42.0, Some(100.0), false, None)
            .await
            .unwrap();
        let failed = import(
            pool.clone(),
            id,
            vec![
                Ok(entry("partial")),
                Err("provider endpoint unavailable".into()),
            ],
            ImportMode::Replace,
        )
        .await;
        assert!(matches!(failed.stage, ImportStage::Failed));
        let rows = vod
            .list(id, 10, 0, None, None, false, false, ContentSort::Default)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "original");
        assert!(rows[0].is_favorite);
        assert_eq!(
            WatchHistoryRepository::new(&pool)
                .resume_position("vod", original.id)
                .await
                .unwrap()
                .unwrap()
                .position_sec,
            42.0
        );
        // A complete refresh still commits, preserves user state and admits new titles.
        let ok = import(
            pool.clone(),
            id,
            vec![Ok(entry("new")), Ok(entry("original"))],
            ImportMode::Replace,
        )
        .await;
        assert!(matches!(ok.stage, ImportStage::Complete));
        let rows = vod
            .list(id, 10, 0, None, None, false, false, ContentSort::Default)
            .await
            .unwrap();
        let original = rows.iter().find(|v| v.name == "original").unwrap();
        assert_eq!(rows.len(), 2);
        assert!(original.is_favorite);
        assert_eq!(
            WatchHistoryRepository::new(&pool)
                .resume_position("vod", original.id)
                .await
                .unwrap()
                .unwrap()
                .position_sec,
            42.0
        );
    }

    fn xtream_movie(name: &str, stream_id: u32, password: &str) -> ParsedEntry {
        ParsedEntry {
            stream_url: format!("http://host/movie/user/{password}/{stream_id}.mkv"),
            upstream_id: Some(format!("xtream:vod:{stream_id}")),
            ..entry(name)
        }
    }

    #[tokio::test]
    async fn canonical_movie_state_survives_failed_and_rotated_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(
            init_profile_db(dir.path().join("profile.db").to_str().unwrap())
                .await
                .unwrap(),
        );
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        import(
            pool.clone(),
            id,
            vec![Ok(xtream_movie("Heat", 7, "old"))],
            ImportMode::Append,
        )
        .await;
        let canonical = CanonicalRepository::new(&pool);
        let filters = nova_storage::MovieFilters::default();
        let entity = canonical
            .list_movies(id, 10, 0, &filters, ContentSort::Default)
            .await
            .unwrap()[0]
            .entity_id
            .clone();
        canonical.set_favorite(&entity, true).await.unwrap();
        // The player reports progress against the transient row (as
        // `record_watch_progress` does): it lands on the entity.
        let row = canonical
            .resolve_playback(&entity, None)
            .await
            .unwrap()
            .unwrap()
            .media_id;
        let owner = canonical.entity_for_vod(row).await.unwrap().unwrap();
        canonical
            .record_progress(&owner, 42.0, Some(100.0), false, None)
            .await
            .unwrap();

        let failed = import(
            pool.clone(),
            id,
            vec![
                Ok(xtream_movie("Other", 8, "new")),
                Err("provider down".into()),
            ],
            ImportMode::Replace,
        )
        .await;
        assert!(matches!(failed.stage, ImportStage::Failed));
        assert_eq!(
            canonical
                .resolve_playback(&entity, None)
                .await
                .unwrap()
                .unwrap()
                .media_id,
            row,
            "rolled-back refresh keeps the previous binding"
        );

        // Credentials rotate: every URL changes, stable IDs do not.
        let ok = import(
            pool.clone(),
            id,
            vec![
                Ok(xtream_movie("Other", 8, "new")),
                Ok(xtream_movie("Heat", 7, "new")),
            ],
            ImportMode::Replace,
        )
        .await;
        assert!(matches!(ok.stage, ImportStage::Complete));
        let movies = canonical
            .list_movies(id, 10, 0, &filters, ContentSort::Default)
            .await
            .unwrap();
        assert_eq!(movies.len(), 2);
        let heat = movies.iter().find(|m| m.title == "Heat").unwrap();
        assert_eq!(heat.entity_id, entity);
        assert!(heat.is_favorite);
        let target = canonical
            .resolve_playback(&entity, None)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(target.media_id, row, "new generation row");
        let owner = canonical
            .entity_for_vod(target.media_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            canonical
                .resume(&owner)
                .await
                .unwrap()
                .unwrap()
                .position_sec,
            42.0
        );
        // The pre-P07 URL-keyed restore could not follow a rotated URL; the
        // legacy mirror is re-derived from the entity instead.
        let legacy = VodRepository::new(&pool)
            .list(id, 10, 0, None, None, true, false, ContentSort::Default)
            .await
            .unwrap();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].name, "Heat");
    }

    #[tokio::test]
    async fn import_resolves_renumbered_and_cross_source_copies_into_one_identity() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(
            init_profile_db(dir.path().join("profile.db").to_str().unwrap())
                .await
                .unwrap(),
        );
        let sources = SourceRepository::new(&pool);
        let main = sources
            .create("xtream", "main", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let backup = sources
            .create("xtream", "backup", "http://other/get.php")
            .await
            .unwrap()
            .id;
        let dated = |name: &str, stream_id: u32| ParsedEntry {
            year: Some(1995),
            ..xtream_movie(name, stream_id, "p")
        };
        import(
            pool.clone(),
            main,
            vec![Ok(dated("Heat", 7))],
            ImportMode::Append,
        )
        .await;
        let canonical = CanonicalRepository::new(&pool);
        let filters = nova_storage::MovieFilters::default();
        let list = |source| {
            let canonical = CanonicalRepository::new(&pool);
            let filters = filters.clone();
            async move {
                canonical
                    .list_movies(source, 10, 0, &filters, ContentSort::Default)
                    .await
                    .unwrap()
            }
        };
        let entity = list(main).await[0].entity_id.clone();
        // Pin the original as oldest (entities created in one test second tie).
        sqlx::query("UPDATE canonical_entities SET created_at = 0 WHERE id = ?")
            .bind(entity.as_str())
            .execute(&*pool)
            .await
            .unwrap();
        canonical.set_favorite(&entity, true).await.unwrap();
        canonical
            .record_progress(&entity, 42.0, Some(100.0), false, None)
            .await
            .unwrap();

        // The provider renumbers the film, and a second source lists it too,
        // with a provider-claimed TMDB ID under a localized title.
        let ok = import(
            pool.clone(),
            main,
            vec![Ok(dated("Heat", 70))],
            ImportMode::Replace,
        )
        .await;
        assert!(matches!(ok.stage, ImportStage::Complete));
        let localized = ParsedEntry {
            provider_tmdb_id: Some(949),
            ..dated("Heat FHD", 3)
        };
        import(
            pool.clone(),
            backup,
            vec![Ok(localized)],
            ImportMode::Append,
        )
        .await;

        for source in [main, backup] {
            let movies = list(source).await;
            assert_eq!(movies.len(), 1);
            assert_eq!(
                movies[0].entity_id, entity,
                "one identity across refresh and sources"
            );
            assert!(movies[0].is_favorite);
        }
        assert_eq!(
            canonical
                .resume(&entity)
                .await
                .unwrap()
                .unwrap()
                .position_sec,
            42.0
        );
        let detail = canonical.movie_detail(&entity).await.unwrap().unwrap();
        assert_eq!(detail.variants.iter().filter(|v| v.available).count(), 2);
        let aliases: Vec<String> = sqlx::query_scalar(
            "SELECT value FROM entity_aliases WHERE entity_id = ? AND alias_kind = 'external'",
        )
        .bind(entity.as_str())
        .fetch_all(&*pool)
        .await
        .unwrap();
        assert_eq!(aliases, vec!["tmdb:movie:949"]);
    }

    /// Opt-in live check of the same fetch, import, and SQLite commit path used
    /// by add_source. The credential-bearing URL stays in the process environment
    /// and the isolated profile database is removed when the test exits.
    #[tokio::test]
    #[ignore = "requires NOVA_PROBE_URL and network access"]
    async fn real_provider_import_persists_catalog() {
        let url = std::env::var("NOVA_PROBE_URL").expect("set NOVA_PROBE_URL");
        let creds = nova_xtream::detect(&url)
            .then(|| nova_xtream::XtreamCredentials::from_url(&url))
            .flatten();
        let kind = if creds.is_some() { "xtream" } else { "m3u" };
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(
            init_profile_db(dir.path().join("profile.db").to_str().unwrap())
                .await
                .unwrap(),
        );
        let source_id = SourceRepository::new(&pool)
            .create(kind, "live validation", &url)
            .await
            .unwrap()
            .id;
        let rx = match creds {
            Some(creds) => nova_xtream::fetch(&creds).await,
            None => nova_m3u::parse(&url).await.entries,
        };
        let events = Mutex::new(Vec::new());
        tokio::time::timeout(
            Duration::from_secs(360),
            run_import(
                source_id,
                rx,
                pool.clone(),
                |event| events.lock().unwrap().push(event),
                ImportMode::Append,
            ),
        )
        .await
        .expect("live import exceeded deadline");
        let events = events.into_inner().unwrap();
        assert_eq!(events.iter().filter(|event| event.done).count(), 1);
        let result = events.last().unwrap();
        assert!(
            matches!(result.stage, ImportStage::Complete),
            "live import failed: {:?}",
            result.error
        );
        assert!(result.parsed > 0);
        assert_eq!(result.failed_count, 0);

        let persisted: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM channels WHERE source_id = ?) + \
                    (SELECT COUNT(*) FROM vod_items WHERE source_id = ?) + \
                    (SELECT COUNT(*) FROM series_episodes WHERE source_id = ?)",
        )
        .bind(source_id)
        .bind(source_id)
        .bind(source_id)
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert!(persisted > 0);
        assert_eq!(persisted as u64, result.inserted);
        println!(
            "live import completed: parsed={}, persisted={}, errors={}",
            result.parsed, persisted, result.failed_count
        );
    }
}
