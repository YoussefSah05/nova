use nova_core::catalog::publish::{check_collapse, check_integrity, PublishRejection};
use nova_core::{
    Channel, ContentSort, ContinueWatchingItem, EpisodeNumberingKind, ImportProgress, ImportStage,
    MediaKind, ParsedEntry, Profile, SeriesEpisode, SeriesSummary, Source, SourceEvidence, VodItem,
};
use nova_storage::{
    init_profile_db, CanonicalRepository, CatalogSearchRepository, ChannelRepository,
    EpgSourcesRepository, ExternalMatchRepository, FailedAttempt, FailureStatus, ProfileRepository,
    ProfileSettingsRepository, PublicationMode, PublicationRepository, SeriesRepository,
    SourceRepository, VodRepository, WatchHistoryRepository, WatchSnapshot,
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::ipc::ApiError;
use crate::state::AppState;

use std::time::{Duration, Instant};

pub mod caption_style;
pub mod channel_logos;
pub mod discovery;
pub mod enrich;
pub mod epg;
pub mod models;
pub mod movies;
pub mod playback;
pub mod profile_settings;
pub mod search;
pub mod series;
pub mod settings;
pub mod title_art;
pub mod title_facts;
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

    let published = run_import(
        source_id,
        rx,
        std::sync::Arc::clone(&pool),
        |progress| {
            let _ = app.emit("playlist:import:progress", progress);
        },
        mode,
    )
    .await;
    if published && mode == ImportMode::Replace {
        refetch_series_with_parked_history(pool, source_id);
    }
    published
}

/// Series whose episode history a refresh had to park (Xtream episodes only
/// come back when re-fetched) are re-fetched in the background, so their
/// resume points reappear in Continue Watching without opening each series.
const MAX_BACKGROUND_SERIES_REFETCH: i64 = 10;

fn refetch_series_with_parked_history(pool: std::sync::Arc<sqlx::SqlitePool>, source_id: i64) {
    tokio::spawn(async move {
        let titles = match WatchHistoryRepository::new(&pool)
            .series_with_pending_episodes(source_id, MAX_BACKGROUND_SERIES_REFETCH)
            .await
        {
            Ok(titles) => titles,
            Err(e) => {
                tracing::warn!(source_id, error = %e, "could not list series to re-fetch");
                return;
            }
        };
        for title in titles {
            // Best effort: a failure leaves the history parked until the
            // series is opened.
            if let Err(e) = fetch_series_episodes(&pool, source_id, &title).await {
                tracing::debug!(source_id, error = ?e, "background episode re-fetch failed");
            }
        }
    });
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
    /// Movies: Nova's primary genre.
    #[serde(default)]
    genre: Option<String>,
    /// Movies: release years, inclusive.
    #[serde(default)]
    year_from: Option<i64>,
    #[serde(default)]
    year_to: Option<i64>,
    /// Movies: a TMDB collection.
    #[serde(default)]
    collection_id: Option<i64>,
    /// Series: still airing, or ended.
    #[serde(default)]
    status: Option<series::SeriesStatus>,
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
    let sort = filters.as_ref().map(|f| f.sort).unwrap_or_default();
    let filters = series::OwnedSeriesFilters::from(filters.unwrap_or_default());
    let limit = limit.unwrap_or(i64::MAX);
    let offset = offset.unwrap_or(0);
    SeriesRepository::new(&pool)
        .browse_series(source_id, &filters.borrow(), sort, limit, offset)
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
    let filters = series::OwnedSeriesFilters::from(filters.unwrap_or_default());
    SeriesRepository::new(&pool)
        .count_browse_series(source_id, &filters.borrow())
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
    series_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SeriesEpisode>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let (source_id, series_title) = series_key(&pool, &series_id).await?;
    fetch_series_episodes(&pool, source_id, &series_title).await
}

/// Where a canonical series (`series_id`, its entity ID) is listed now:
/// `(source_id, series_title)`. The frontend addresses series by entity, so
/// a provider rename or refresh can't make its references stale.
pub(crate) async fn series_key(
    pool: &sqlx::SqlitePool,
    series_id: &str,
) -> Result<(i64, String), ApiError> {
    CanonicalRepository::new(pool)
        .series_source(series_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("series is not listed".into()))
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
pub(crate) async fn fetch_series_episodes(
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

    let series_facts = fetched.facts.clone();
    let series_tmdb_id = fetched.tmdb_id;
    let rows: Vec<SeriesEpisode> = fetched
        .episodes
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
            // Provider numbering; Xtream reports a missing episode number as 0.
            numbering: if ep.episode > 0 {
                EpisodeNumberingKind::SeasonEpisode
            } else {
                EpisodeNumberingKind::Unknown
            },
            // Provider fields are stored verbatim (the episode title is raw);
            // the episode's `info` facts (plot, runtime, air date) ride along.
            evidence: SourceEvidence {
                provider_facts: ep.facts,
                ..SourceEvidence::default()
            },
        })
        .collect();

    // Nothing came back (e.g. provider returned an empty info blob) — keep the
    // sentinel so a later open can retry, and report empty.
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    repo.replace_series_episodes(
        source_id,
        series_title,
        &rows,
        series_facts.as_ref(),
        series_tmdb_id,
    )
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
    series_id: String,
    is_favorite: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let (source_id, series_title) = series_key(&pool, &series_id).await?;
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
    series_id: String,
    in_watchlist: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let (source_id, series_title) = series_key(&pool, &series_id).await?;
    SeriesRepository::new(&pool)
        .set_in_watchlist(source_id, &series_title, in_watchlist)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_continue_watching(
    profile_id: String,
    source_id: i64,
    section: nova_core::ContinueSection,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<ContinueWatchingItem>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    WatchHistoryRepository::new(&pool)
        .continue_watching_section(section, source_id, limit.unwrap_or(20).clamp(1, 100))
        .await
        .map_err(Into::into)
}

// ── Import worker ─────────────────────────────────────────────────────────────

const BATCH: usize = 1_000;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a candidate was not published. The candidate's transaction is always
/// rolled back before this is reported, so the prior active catalog is intact.
struct ImportFailure {
    status: FailureStatus,
    /// Stable, secret-free code persisted with the attempt.
    code: &'static str,
    /// Secret-free summary persisted with the attempt.
    reason: String,
    /// Shown to the user (may carry the provider's summarized error).
    message: String,
    warning: Option<String>,
}

impl ImportFailure {
    fn storage(e: impl std::fmt::Display) -> Self {
        Self {
            status: FailureStatus::Failed,
            code: "storage_error",
            reason: "database error while building the catalog".into(),
            message: e.to_string(),
            warning: None,
        }
    }

    fn rejected(rejection: PublishRejection, mode: ImportMode) -> Self {
        let retained = if mode == ImportMode::Replace {
            "; previous catalog retained"
        } else {
            ""
        };
        Self {
            status: FailureStatus::Rejected,
            code: rejection.code(),
            reason: rejection.to_string(),
            message: format!("Refresh rejected: {rejection}{retained}"),
            warning: None,
        }
    }
}

/// Import one source as a single candidate generation:
///
/// 1. **Build**: in one transaction, clear the source's rows (refresh), insert
///    the received entries, restore user state, compile and resolve canonical
///    entities. Readers keep seeing the previous catalog (WAL) meanwhile.
/// 2. **Validate**: the candidate must hold every received row with every
///    movie bound, and a refresh must not collapse a substantial content kind.
/// 3. **Publish**: record the active generation and commit atomically.
///
/// Any failure rolls the transaction back explicitly and is then logged as a
/// failed/rejected attempt, leaving the prior active catalog available.
async fn run_import(
    source_id: i64,
    rx: tokio::sync::mpsc::Receiver<Result<ParsedEntry, String>>,
    pool: std::sync::Arc<sqlx::SqlitePool>,
    emit: impl Fn(ImportProgress),
    mode: ImportMode,
) -> bool {
    let started_at = Instant::now();
    let started_ts = nova_core::now_ts();
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

    let result = match pool.begin().await {
        Err(e) => Err(ImportFailure::storage(e)),
        Ok(mut tx) => {
            let built = build_candidate(
                &mut tx,
                source_id,
                rx,
                &emit,
                mode,
                started_ts,
                &mut stats,
                &mut errors,
                started_at,
            )
            .await;
            match built {
                Ok(()) => {
                    let commit_started = Instant::now();
                    let committed = tx.commit().await.map_err(ImportFailure::storage);
                    stats.persist_ms += commit_started.elapsed().as_millis() as u64;
                    committed
                }
                Err(failure) => {
                    // Roll back before anything else touches the database, so
                    // the failure record never waits on this write lock.
                    if let Err(e) = tx.rollback().await {
                        tracing::warn!(source_id, error = %e, "candidate rollback failed");
                    }
                    Err(failure)
                }
            }
        }
    };
    stats.elapsed_ms = started_at.elapsed().as_millis() as u64;

    match result {
        Ok(()) => {
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
                "playlist import published"
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
        Err(failure) => {
            tracing::warn!(
                source_id,
                code = failure.code,
                parsed = stats.parsed,
                "catalog candidate not published: {}",
                failure.reason
            );
            let publication_mode = match mode {
                ImportMode::Append => PublicationMode::Append,
                ImportMode::Replace => PublicationMode::Replace,
            };
            let received = nova_core::catalog::publish::KindCounts {
                live: stats.live_count,
                movie: stats.movie_count,
                series: stats.series_count,
            };
            if let Err(e) = PublicationRepository::new(&pool)
                .record_failure(&FailedAttempt {
                    source_id,
                    mode: publication_mode,
                    status: failure.status,
                    reason_code: failure.code,
                    reason: &failure.reason,
                    counts: received,
                    started_at: started_ts,
                })
                .await
            {
                tracing::warn!(source_id, error = %e, "failed to record import failure");
            }
            emit_import_progress(
                &emit,
                source_id,
                ImportStage::Failed,
                &stats,
                Some(failure.message),
                failure.warning,
                true,
            );
            false
        }
    }
}

/// Steps 1-2 (build + validate) and the in-transaction half of step 3. The
/// caller commits on `Ok` and rolls back on `Err`.
#[allow(clippy::too_many_arguments)]
async fn build_candidate(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    source_id: i64,
    mut rx: tokio::sync::mpsc::Receiver<Result<ParsedEntry, String>>,
    emit: &impl Fn(ImportProgress),
    mode: ImportMode,
    started_ts: i64,
    stats: &mut ImportStats,
    errors: &mut Vec<String>,
    started_at: Instant,
) -> Result<(), ImportFailure> {
    let mut live_buf: Vec<Channel> = Vec::with_capacity(BATCH);
    let mut vod_buf: Vec<VodItem> = Vec::with_capacity(BATCH);
    let mut series_buf: Vec<SeriesEpisode> = Vec::with_capacity(BATCH);

    // Refresh: measure the active catalog (for the collapse check), then
    // snapshot user state and clear the source's rows inside this transaction.
    // Readers keep seeing the active catalog until commit.
    let (previous, replace_snapshot) = if mode == ImportMode::Replace {
        let previous = PublicationRepository::measure_tx(tx, source_id)
            .await
            .map_err(ImportFailure::storage)?;
        let snap = snapshot_and_clear(tx, source_id)
            .await
            .map_err(ImportFailure::storage)?;
        (Some(previous), Some(snap))
    } else {
        (None, None)
    };

    loop {
        let wait_started = Instant::now();
        let result = match tokio::time::timeout(IDLE_TIMEOUT, rx.recv()).await {
            Ok(Some(result)) => result,
            Ok(None) => break, // all producers dropped — import finished
            Err(_) => {
                stats.receive_wait_ms += wait_started.elapsed().as_millis() as u64;
                stats.failed_count += 1;
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
                    emit,
                    source_id,
                    ImportStage::Receiving,
                    stats,
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

        flush_ready_buffers(tx, &mut live_buf, &mut vod_buf, &mut series_buf, stats)
            .await
            .map_err(ImportFailure::storage)?;

        // Emit progress every 500 entries for responsive feedback without
        // saturating the frontend event bridge on very large playlists.
        if stats.parsed.is_multiple_of(500) {
            stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
            emit_import_progress(
                emit,
                source_id,
                ImportStage::Receiving,
                stats,
                None,
                warning_from_errors(errors),
                false,
            );
        }
    }

    // Any endpoint failure or receive timeout makes a replacement incomplete.
    if mode == ImportMode::Replace && !errors.is_empty() {
        return Err(ImportFailure {
            status: FailureStatus::Failed,
            code: "incomplete_refresh",
            reason: "a provider endpoint failed or stalled".into(),
            message: "Refresh incomplete; previous catalog retained".into(),
            warning: warning_from_errors(errors),
        });
    }

    if stats.parsed == 0 {
        let message = if errors.is_empty() {
            "import completed without any playlist entries".to_string()
        } else {
            errors.join("; ")
        };
        return Err(ImportFailure {
            status: FailureStatus::Failed,
            code: "no_entries",
            reason: "the provider returned no playlist entries".into(),
            message,
            warning: None,
        });
    }

    stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
    emit_import_progress(
        emit,
        source_id,
        ImportStage::Persisting,
        stats,
        None,
        warning_from_errors(errors),
        false,
    );

    flush_all_buffers(tx, &mut live_buf, &mut vod_buf, &mut series_buf, stats)
        .await
        .map_err(ImportFailure::storage)?;

    // Refresh: re-apply the snapshotted user state onto the new rows.
    if let Some(snap) = replace_snapshot {
        restore_user_state(tx, source_id, &snap)
            .await
            .map_err(ImportFailure::storage)?;
    }

    // Catalog compiler: bind this generation's movie records to canonical
    // entities, then resolve identities across every source of the profile.
    CanonicalRepository::compile_movies_tx(tx, source_id)
        .await
        .map_err(ImportFailure::storage)?;
    CanonicalRepository::resolve_movies_tx(tx)
        .await
        .map_err(ImportFailure::storage)?;
    // Series: bind this generation's series to their entities (by provider
    // series ID, else title) so series state survives renames.
    CanonicalRepository::compile_series_tx(tx, source_id)
        .await
        .map_err(ImportFailure::storage)?;
    // Episodes that came back under a different URL (M3U credential change)
    // pick up their parked history by durable key, now that this
    // generation's series are bound to their entities.
    WatchHistoryRepository::reattach_pending_for_source_tx(tx, source_id)
        .await
        .map_err(ImportFailure::storage)?;
    // Transcripts follow their title to its new row (dialogue search and
    // captions keep working after a refresh).
    nova_storage::TranscriptRepository::reattach_orphans_tx(
        tx,
        source_id,
        crate::caption_session::content_key,
    )
    .await
    .map_err(ImportFailure::storage)?;
    // Entity-owned series matches survive the refresh (and provider renames):
    // re-mirror them onto the new rows instead of re-matching.
    ExternalMatchRepository::mirror_series_for_source_tx(tx, source_id)
        .await
        .map_err(ImportFailure::storage)?;
    // Entity-owned TMDB matches survive the refresh: re-mirror them onto the
    // new rows instead of re-matching.
    ExternalMatchRepository::mirror_movies_for_source_tx(tx, source_id)
        .await
        .map_err(ImportFailure::storage)?;
    // The search projection follows the resolved identities (merges, new
    // entities, new provider titles) and publishes atomically with them.
    CatalogSearchRepository::rebuild_movies_tx(tx)
        .await
        .map_err(ImportFailure::storage)?;

    // Validate the compiled candidate before it can become active.
    let received = nova_core::catalog::publish::KindCounts {
        live: stats.live_count,
        movie: stats.movie_count,
        series: stats.series_count,
    };
    let report = PublicationRepository::candidate_report_tx(tx, source_id)
        .await
        .map_err(ImportFailure::storage)?;
    check_integrity(received, &report).map_err(|r| ImportFailure::rejected(r, mode))?;
    let candidate = PublicationRepository::measure_tx(tx, source_id)
        .await
        .map_err(ImportFailure::storage)?;
    if let Some(previous) = previous {
        check_collapse(previous, candidate).map_err(|r| ImportFailure::rejected(r, mode))?;
    }

    // Publish: mark this generation active in the same transaction.
    SourceRepository::touch_refreshed_tx(tx, source_id)
        .await
        .map_err(ImportFailure::storage)?;
    let publication_mode = match mode {
        ImportMode::Append => PublicationMode::Append,
        ImportMode::Replace => PublicationMode::Replace,
    };
    PublicationRepository::record_published_tx(
        tx,
        source_id,
        publication_mode,
        candidate,
        started_ts,
    )
    .await
    .map_err(ImportFailure::storage)?;
    Ok(())
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
    series_flags: Vec<nova_storage::SeriesFlagSnapshot>,
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
    let mut renames =
        SeriesRepository::apply_series_flags_tx(tx, source_id, &snap.series_flags).await?;
    // Renames are also visible through watched episodes, so a series with
    // progress but no favourite/watchlist is recognised too.
    for rename in
        WatchHistoryRepository::episode_series_renames_tx(tx, source_id, &snap.watch).await?
    {
        if !renames.contains(&rename) {
            renames.push(rename);
        }
    }
    // A title-keyed (M3U) series renamed by the provider keeps its entity
    // (state, match, episode progress): move its binding rather than seeding
    // a copy.
    CanonicalRepository::carry_series_renames_tx(tx, source_id, &renames).await?;
    WatchHistoryRepository::restore_tx(tx, source_id, &snap.watch).await?;
    // Parked history is re-attached after the series compiler (see
    // `build_candidate`): it reaches the entity through the new bindings.
    Ok(())
}

fn push_entry(
    e: &ParsedEntry,
    source_id: i64,
    live: &mut Vec<Channel>,
    vod: &mut Vec<VodItem>,
    series: &mut Vec<SeriesEpisode>,
) {
    let evidence = SourceEvidence::from_entry(e);
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
                category: evidence.facet(),
                raw_group: Some(e.group_title.clone()),
                logo_url: e.logo_url.clone(),
                stream_url: e.stream_url.clone(),
                is_favorite: false,
                is_in_watchlist: false,
                last_played_at: None,
                added_at: None,
                dedup_group: None,
                region,
                evidence,
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
            category: evidence.facet(),
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
            evidence,
        }),
        MediaKind::Series => series.push(SeriesEpisode {
            id: 0,
            source_id,
            series_title: nova_core::polish_title(
                e.series_title.as_deref().unwrap_or(&e.clean_title),
            ),
            category: evidence.facet(),
            // Unknown parts are stored as placeholders (the columns are NOT
            // NULL; the Xtream sentinel keeps its 0/0) and flagged by
            // `numbering`, so they are never shown as a real S01E01.
            season: i64::from(e.season.unwrap_or(1)),
            episode: i64::from(e.episode.unwrap_or(1)),
            episode_title: None,
            logo_url: e.logo_url.clone(),
            stream_url: e.stream_url.clone(),
            added_at: None,
            numbering: if e.season == Some(0) && e.episode == Some(0) {
                EpisodeNumberingKind::SeasonEpisode
            } else {
                EpisodeNumberingKind::from_parsed(e.season, e.episode)
            },
            evidence,
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
            quality_hints: Vec::new(),
            language_hints: Vec::new(),
            parser_version: None,
            provider_facts: None,
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

    // ── P10: compile → validate → atomic publish ────────────────────────────

    async fn open_db(path: &std::path::Path) -> Arc<sqlx::SqlitePool> {
        Arc::new(init_profile_db(path.to_str().unwrap()).await.unwrap())
    }

    fn movies(range: std::ops::Range<u32>) -> Vec<Result<ParsedEntry, String>> {
        range
            .map(|i| Ok(xtream_movie(&format!("Film {i}"), i, "p")))
            .collect()
    }

    fn live(name: &str) -> ParsedEntry {
        ParsedEntry {
            kind: MediaKind::Live,
            ..entry(name)
        }
    }

    async fn vod_count(pool: &sqlx::SqlitePool, source: i64) -> i64 {
        VodRepository::new(pool)
            .count_filtered(source, None, None, false, false)
            .await
            .unwrap()
    }

    /// A profile with one published source of 60 movies, one of them a
    /// favourite with resume progress. Returns (pool, source, entity).
    async fn published_library(
        path: &std::path::Path,
    ) -> (Arc<sqlx::SqlitePool>, i64, nova_core::catalog::EntityId) {
        let pool = open_db(path).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let first = import(pool.clone(), id, movies(0..60), ImportMode::Append).await;
        assert!(matches!(first.stage, ImportStage::Complete));
        let canonical = CanonicalRepository::new(&pool);
        let entity = canonical
            .list_movies(id, 1, 0, &Default::default(), ContentSort::Default)
            .await
            .unwrap()[0]
            .entity_id
            .clone();
        canonical.set_favorite(&entity, true).await.unwrap();
        canonical
            .record_progress(&entity, 42.0, Some(100.0), false, None)
            .await
            .unwrap();
        (pool, id, entity)
    }

    /// The prior generation is fully intact: rows, entity state, active publication.
    async fn assert_prior_generation(
        pool: &sqlx::SqlitePool,
        source: i64,
        entity: &nova_core::catalog::EntityId,
        active_id: i64,
    ) {
        assert_eq!(vod_count(pool, source).await, 60);
        let canonical = CanonicalRepository::new(pool);
        let detail = canonical.movie_detail(entity).await.unwrap().unwrap();
        assert!(detail.summary.is_favorite);
        assert_eq!(detail.resume.unwrap().position_sec, 42.0);
        assert!(canonical
            .resolve_playback(entity, None)
            .await
            .unwrap()
            .is_some());
        let active = PublicationRepository::new(pool)
            .active(source)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(active.id, active_id);
        assert_eq!(active.counts.movie, 60);
    }

    fn episode_entry(series: &str, season: u32, episode: u32, url: &str) -> ParsedEntry {
        ParsedEntry {
            kind: MediaKind::Series,
            series_title: Some(series.into()),
            clean_title: series.into(),
            season: Some(season),
            episode: Some(episode),
            stream_url: url.into(),
            ..entry(series)
        }
    }

    async fn episode_id(pool: &sqlx::SqlitePool, url: &str) -> i64 {
        sqlx::query_scalar("SELECT id FROM series_episodes WHERE stream_url = ?")
            .bind(url)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn m3u_episode_resume_survives_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("m3u", "test", "https://example.invalid/list.m3u")
            .await
            .unwrap()
            .id;
        let rows = || {
            vec![
                Ok(entry("A newer title listed first")),
                Ok(episode_entry(
                    "Dark",
                    1,
                    1,
                    "https://cdn.example/dark/s01e01.mp4",
                )),
                Ok(episode_entry(
                    "Dark",
                    1,
                    2,
                    "https://cdn.example/dark/s01e02.mp4",
                )),
            ]
        };
        import(pool.clone(), id, rows(), ImportMode::Append).await;
        let history = WatchHistoryRepository::new(&pool);
        let e2 = episode_id(&pool, "https://cdn.example/dark/s01e02.mp4").await;
        history
            .record_progress("episode", e2, 1234.0, Some(3000.0), false, None)
            .await
            .unwrap();

        let ok = import(pool.clone(), id, rows(), ImportMode::Replace).await;
        assert!(matches!(ok.stage, ImportStage::Complete));
        let e2_new = episode_id(&pool, "https://cdn.example/dark/s01e02.mp4").await;
        let resume = history.resume_position("episode", e2_new).await.unwrap();
        assert_eq!(
            resume.map(|r| r.position_sec),
            Some(1234.0),
            "resume survives refresh"
        );
    }

    #[tokio::test]
    async fn xtream_episode_resume_is_parked_then_reattached_after_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create(
                "xtream",
                "test",
                "http://h:80/get.php?username=u&password=old",
            )
            .await
            .unwrap()
            .id;
        let sentinel = || vec![Ok(episode_entry("Show", 0, 0, "xtream://series/42"))];
        import(pool.clone(), id, sentinel(), ImportMode::Append).await;
        // The series is opened: episodes are fetched lazily (simulated).
        let fetched = |password: &str| {
            let url = format!("http://h:80/series/u/{password}/293257.mkv");
            vec![SeriesEpisode {
                id: 0,
                source_id: id,
                series_title: "Show".into(),
                category: None,
                season: 1,
                episode: 3,
                episode_title: Some("Pilot".into()),
                logo_url: None,
                stream_url: url,
                added_at: None,
                numbering: nova_core::EpisodeNumberingKind::SeasonEpisode,
                evidence: SourceEvidence::default(),
            }]
        };
        let series = SeriesRepository::new(&pool);
        series
            .replace_series_episodes(id, "Show", &fetched("old"), None, None)
            .await
            .unwrap();
        let history = WatchHistoryRepository::new(&pool);
        let ep = episode_id(&pool, "http://h:80/series/u/old/293257.mkv").await;
        history
            .record_progress("episode", ep, 600.0, Some(2400.0), false, Some("source"))
            .await
            .unwrap();

        // Refresh: only the sentinel comes back, so the history is parked.
        let ok = import(pool.clone(), id, sentinel(), ImportMode::Replace).await;
        assert!(matches!(ok.stage, ImportStage::Complete));
        let parked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pending_episode_history")
            .fetch_one(&*pool)
            .await
            .unwrap();
        assert_eq!(parked, 1);
        assert_eq!(
            history.series_with_pending_episodes(id, 10).await.unwrap(),
            vec!["Show".to_string()],
            "queued for the background re-fetch"
        );
        let stored: Vec<String> =
            sqlx::query_scalar("SELECT episode_key FROM pending_episode_history")
                .fetch_all(&*pool)
                .await
                .unwrap();
        assert!(
            !stored[0].contains("old"),
            "no credentials in the parking table"
        );

        // Re-fetched after a credential rotation: the durable key still matches.
        series
            .replace_series_episodes(id, "Show", &fetched("new"), None, None)
            .await
            .unwrap();
        let ep_new = episode_id(&pool, "http://h:80/series/u/new/293257.mkv").await;
        let resume = history
            .resume_position("episode", ep_new)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resume.position_sec, 600.0);
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pending_episode_history")
            .fetch_one(&*pool)
            .await
            .unwrap();
        assert_eq!(left, 0, "re-attached exactly once");
        let cw = history.list_continue_watching(10).await.unwrap();
        assert!(
            cw.iter().any(|c| c.media_id == ep_new),
            "back in Continue Watching"
        );
    }

    #[tokio::test]
    async fn series_flags_survive_a_refresh_that_changes_the_parsed_title() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let series = |title: &str| ParsedEntry {
            kind: MediaKind::Series,
            series_title: Some(title.into()),
            clean_title: title.into(),
            season: Some(0),
            episode: Some(0),
            stream_url: "xtream://series/42".into(),
            ..entry("FR | Show")
        };
        // Stored under the title the previous parser produced...
        import(
            pool.clone(),
            id,
            vec![Ok(series("FR Show"))],
            ImportMode::Append,
        )
        .await;
        let repo = SeriesRepository::new(&pool);
        repo.set_favorite(id, "FR Show", true).await.unwrap();
        repo.set_in_watchlist(id, "FR Show", true).await.unwrap();
        // ...then refreshed under the new parser's title.
        let ok = import(
            pool.clone(),
            id,
            vec![Ok(series("Show"))],
            ImportMode::Replace,
        )
        .await;
        assert!(matches!(ok.stage, ImportStage::Complete));
        let list = repo
            .list_series(id, None, None, false, false, ContentSort::Default, 10, 0)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].series_title, "Show");
        assert!(
            list[0].is_favorite && list[0].is_in_watchlist,
            "flags followed the stream URL"
        );
    }

    /// A series' canonical entity, by its current title in the source.
    async fn series_entity(pool: &sqlx::SqlitePool, source: i64, title: &str) -> String {
        CanonicalRepository::new(pool)
            .series_entity(source, title)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{title} is bound"))
            .as_str()
            .to_string()
    }

    async fn series_flags(pool: &sqlx::SqlitePool, source: i64, title: &str) -> (bool, bool) {
        let list = SeriesRepository::new(pool)
            .list_series(
                source,
                None,
                None,
                false,
                false,
                ContentSort::Default,
                50,
                0,
            )
            .await
            .unwrap();
        let s = list
            .iter()
            .find(|s| s.series_title == title)
            .unwrap_or_else(|| panic!("{title} listed"));
        (s.is_favorite, s.is_in_watchlist)
    }

    fn opened_episodes(source: i64, title: &str) -> Vec<SeriesEpisode> {
        (1..=2)
            .map(|n| SeriesEpisode {
                id: 0,
                source_id: source,
                series_title: title.into(),
                category: None,
                season: 1,
                episode: n,
                episode_title: None,
                logo_url: None,
                stream_url: format!("http://h:80/series/u/p/{}.mkv", 900 + n),
                added_at: None,
                numbering: nova_core::EpisodeNumberingKind::SeasonEpisode,
                evidence: SourceEvidence::default(),
            })
            .collect()
    }

    #[tokio::test]
    async fn an_opened_series_keeps_its_entity_and_state_through_a_provider_rename() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let catalog = |title: &str| {
            vec![
                Ok(episode_entry(title, 0, 0, "xtream://series/42")),
                Ok(episode_entry("Other", 0, 0, "xtream://series/43")),
                Ok(entry("Heat")),
            ]
        };
        import(pool.clone(), id, catalog("FR Show"), ImportMode::Append).await;
        let repo = SeriesRepository::new(&pool);
        repo.set_favorite(id, "FR Show", true).await.unwrap();
        repo.set_in_watchlist(id, "FR Show", true).await.unwrap();
        let entity = series_entity(&pool, id, "FR Show").await;
        // Opened: the sentinel (and its series ID) gives way to episodes.
        repo.replace_series_episodes(id, "FR Show", &opened_episodes(id, "FR Show"), None, None)
            .await
            .unwrap();

        // The provider renames the series; only its sentinel comes back.
        let ok = import(pool.clone(), id, catalog("Show"), ImportMode::Replace).await;
        assert!(matches!(ok.stage, ImportStage::Complete));
        assert_eq!(
            series_entity(&pool, id, "Show").await,
            entity,
            "same identity"
        );
        assert_eq!(series_flags(&pool, id, "Show").await, (true, true));
        assert_eq!(series_flags(&pool, id, "Other").await, (false, false));
        let kinds: Vec<(String, i64)> = sqlx::query_as(
            "SELECT kind, COUNT(*) FROM canonical_entities GROUP BY kind ORDER BY kind",
        )
        .fetch_all(&*pool)
        .await
        .unwrap();
        assert_eq!(kinds, [("movie".into(), 1), ("series".into(), 2)]);
        let movie_docs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM movie_search")
            .fetch_one(&*pool)
            .await
            .unwrap();
        assert_eq!(movie_docs, 1, "series entities stay out of movie search");
    }

    #[tokio::test]
    async fn a_title_bound_series_upgrades_to_its_provider_id() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        // Bound while opened (no sentinel), as an upgraded profile would be.
        let opened: Vec<_> = opened_episodes(id, "Dark")
            .into_iter()
            .map(|e| Ok(episode_entry("Dark", 1, e.episode as u32, &e.stream_url)))
            .collect();
        import(pool.clone(), id, opened, ImportMode::Append).await;
        SeriesRepository::new(&pool)
            .set_favorite(id, "Dark", true)
            .await
            .unwrap();
        let entity = series_entity(&pool, id, "Dark").await;
        // The sentinel appears: same title, the binding upgrades in place.
        let sentinel = |title: &str| vec![Ok(episode_entry(title, 0, 0, "xtream://series/7"))];
        import(pool.clone(), id, sentinel("Dark"), ImportMode::Replace).await;
        assert_eq!(series_entity(&pool, id, "Dark").await, entity);
        let key: String = sqlx::query_scalar("SELECT binding_key FROM series_bindings")
            .fetch_one(&*pool)
            .await
            .unwrap();
        assert_eq!(
            key,
            nova_core::catalog::media::series_binding_key(Some("7"), "Dark")
        );
        // From then on a rename keeps identity.
        import(pool.clone(), id, sentinel("Dark (DE)"), ImportMode::Replace).await;
        assert_eq!(series_entity(&pool, id, "Dark (DE)").await, entity);
        assert_eq!(series_flags(&pool, id, "Dark (DE)").await, (true, false));
    }

    #[tokio::test]
    async fn series_toggles_are_entity_owned_across_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let catalog = || vec![Ok(episode_entry("Show", 0, 0, "xtream://series/42"))];
        import(pool.clone(), id, catalog(), ImportMode::Append).await;
        let repo = SeriesRepository::new(&pool);
        let entity_flags = || async {
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT is_favorite, is_in_watchlist FROM canonical_entities WHERE kind = 'series'",
            )
            .fetch_one(&*pool)
            .await
            .unwrap()
        };
        repo.set_favorite(id, "Show", true).await.unwrap();
        assert_eq!(entity_flags().await, (1, 0), "written to the entity");
        import(pool.clone(), id, catalog(), ImportMode::Replace).await;
        assert_eq!(series_flags(&pool, id, "Show").await, (true, false));
        // Clearing sticks too: the entity, not a stale snapshot, decides.
        repo.set_favorite(id, "Show", false).await.unwrap();
        repo.set_in_watchlist(id, "Show", true).await.unwrap();
        import(pool.clone(), id, catalog(), ImportMode::Replace).await;
        assert_eq!(series_flags(&pool, id, "Show").await, (false, true));
        assert_eq!(entity_flags().await, (0, 1));
    }

    #[tokio::test]
    async fn m3u_series_bind_by_title_and_carry_their_entity_through_a_rename() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("m3u", "test", "https://example.invalid/list.m3u")
            .await
            .unwrap()
            .id;
        let catalog = |title: &str| {
            vec![
                Ok(episode_entry(
                    title,
                    1,
                    1,
                    "https://example.invalid/s/1.mkv",
                )),
                Ok(episode_entry(
                    title,
                    1,
                    2,
                    "https://example.invalid/s/2.mkv",
                )),
            ]
        };
        import(pool.clone(), id, catalog("Show"), ImportMode::Append).await;
        let repo = SeriesRepository::new(&pool);
        repo.set_favorite(id, "Show", true).await.unwrap();
        let entity = series_entity(&pool, id, "Show").await;
        import(pool.clone(), id, catalog("Show"), ImportMode::Replace).await;
        assert_eq!(
            series_entity(&pool, id, "Show").await,
            entity,
            "same title, same entity"
        );
        // M3U has no series ID; the refresh recognises the rename through the
        // unchanged episode URLs and moves the identity, not a copy of it.
        import(pool.clone(), id, catalog("The Show"), ImportMode::Replace).await;
        assert_eq!(series_entity(&pool, id, "The Show").await, entity);
        assert_eq!(series_flags(&pool, id, "The Show").await, (true, false));
        // Cleared, then the provider reverts the title: nothing stale returns.
        repo.set_favorite(id, "The Show", false).await.unwrap();
        import(pool.clone(), id, catalog("Show"), ImportMode::Replace).await;
        assert_eq!(series_flags(&pool, id, "Show").await, (false, false));
        // An unflagged rename isn't detected (the restore snapshots flagged
        // series only), so "Show" is a fresh entity again; no entity may
        // still hold the cleared favourite.
        let stale: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM canonical_entities WHERE kind = 'series' AND is_favorite = 1",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(stale, 0, "no stale copies of cleared state");
    }

    #[tokio::test]
    async fn a_compile_without_the_sentinel_keeps_an_opened_series_identity() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let sentinel = || vec![Ok(episode_entry("Dark", 0, 0, "xtream://series/7"))];
        import(pool.clone(), id, sentinel(), ImportMode::Append).await;
        let repo = SeriesRepository::new(&pool);
        repo.set_favorite(id, "Dark", true).await.unwrap();
        let entity = series_entity(&pool, id, "Dark").await;
        // Opened: only episodes remain, bound by the provider ID.
        repo.replace_series_episodes(id, "Dark", &opened_episodes(id, "Dark"), None, None)
            .await
            .unwrap();
        // A compile outside an import (e.g. the open-time backfill) sees no
        // sentinel; it must keep the ID binding, not fork a title-keyed copy.
        let mut tx = pool.begin().await.unwrap();
        CanonicalRepository::compile_series_tx(&mut tx, id)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(series_entity(&pool, id, "Dark").await, entity);
        repo.set_in_watchlist(id, "Dark", true).await.unwrap();
        // The next refresh brings the sentinel back: same entity, both flags.
        import(pool.clone(), id, sentinel(), ImportMode::Replace).await;
        assert_eq!(series_entity(&pool, id, "Dark").await, entity);
        assert_eq!(series_flags(&pool, id, "Dark").await, (true, true));
        let series: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM canonical_entities WHERE kind = 'series'")
                .fetch_one(&*pool)
                .await
                .unwrap();
        assert_eq!(series, 1);
    }

    fn tv_match(title: &str, tmdb_id: i64, name: &str) -> nova_storage::SeriesTmdbWrite {
        nova_storage::SeriesTmdbWrite {
            series_title: title.into(),
            tmdb_id: Some(tmdb_id),
            canonical_title: Some(name.into()),
            original_title: Some(name.into()),
            title_language: Some("en-US".into()),
            poster_url: Some("https://image.tmdb.org/t/p/w500/p.jpg".into()),
            genres_csv: Some("Drama, Mystery".into()),
            primary_genre: Some("Drama".into()),
            matched: true,
            method: "trusted_id",
            confidence_bp: Some(10_000),
            matcher_version: 3,
            evidence_json: "{}".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_series_tmdb_match_is_entity_owned_and_survives_a_provider_rename() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let catalog = |title: &str| vec![Ok(episode_entry(title, 0, 0, "xtream://series/42"))];
        import(pool.clone(), id, catalog("FR Dark"), ImportMode::Append).await;
        let repo = SeriesRepository::new(&pool);
        assert_eq!(
            repo.list_for_tmdb(id, 3, 0, "en-US").await.unwrap().len(),
            1
        );
        repo.apply_tmdb(id, &[tv_match("FR Dark", 70523, "Dark")])
            .await
            .unwrap();
        let entity = series_entity(&pool, id, "FR Dark").await;
        let owned: (String, String, String) = sqlx::query_as(
            "SELECT entity_id, status, external_id FROM entity_external_matches WHERE namespace = 'tmdb:tv'",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(owned, (entity.clone(), "matched".into(), "70523".into()));
        assert!(repo
            .list_for_tmdb(id, 3, 0, "en-US")
            .await
            .unwrap()
            .is_empty());

        // Renamed by the provider: the new title shows the match (mirrored),
        // and nothing is queued for another TMDB request.
        import(pool.clone(), id, catalog("Dark"), ImportMode::Replace).await;
        let list = repo
            .list_series(id, None, None, false, false, ContentSort::Default, 10, 0)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].series_title, "Dark");
        assert_eq!(list[0].canonical_title.as_deref(), Some("Dark"));
        assert_eq!(list[0].entity_id.as_deref(), Some(entity.as_str()));
        let mirrored: (i64, String) = sqlx::query_as(
            "SELECT tmdb_id, tmdb_status FROM series_meta WHERE source_id = ? AND series_title = 'Dark'",
        )
        .bind(id)
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(mirrored, (70523, "matched".into()));
        let genre: String = sqlx::query_scalar(
            "SELECT genre FROM series_episodes WHERE source_id = ? AND series_title = 'Dark'",
        )
        .bind(id)
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(genre, "Drama", "TMDB genre mirrored onto the new rows");
        assert!(repo
            .list_for_tmdb(id, 3, 0, "en-US")
            .await
            .unwrap()
            .is_empty());
        // A newer matcher or another metadata language re-queues it.
        assert_eq!(
            repo.list_for_tmdb(id, 4, 0, "en-US").await.unwrap().len(),
            1
        );
        assert_eq!(
            repo.list_for_tmdb(id, 3, 0, "fr-FR").await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn an_unresolved_series_is_retried_once_the_provider_claims_an_id() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let catalog = vec![Ok(episode_entry("Show", 0, 0, "xtream://series/42"))];
        import(pool.clone(), id, catalog, ImportMode::Append).await;
        let repo = SeriesRepository::new(&pool);
        let unresolved = nova_storage::SeriesTmdbWrite {
            series_title: "Show".into(),
            matched: false,
            method: "search",
            matcher_version: 3,
            evidence_json: "{}".into(),
            ..Default::default()
        };
        repo.apply_tmdb(id, &[unresolved]).await.unwrap();
        let status: String = sqlx::query_scalar(
            "SELECT status FROM entity_external_matches WHERE namespace = 'tmdb:tv'",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(status, "unresolved");
        assert!(repo
            .list_for_tmdb(id, 3, 0, "en-US")
            .await
            .unwrap()
            .is_empty());
        // New evidence: the provider now claims a TMDB id for it.
        sqlx::query(
            "INSERT INTO series_provider_facts (source_id, series_title, provider_tmdb_id, updated_at) VALUES (?, 'Show', 1399, 0)",
        )
        .bind(id)
        .execute(&*pool)
        .await
        .unwrap();
        let queued = repo.list_for_tmdb(id, 3, 0, "en-US").await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].trusted_tmdb_id, Some(1399));
    }

    #[tokio::test]
    async fn a_new_series_under_an_old_title_does_not_inherit_its_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.db");
        let pool = open_db(&path).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        import(
            pool.clone(),
            id,
            vec![Ok(episode_entry("FR Dark", 0, 0, "xtream://series/42"))],
            ImportMode::Append,
        )
        .await;
        let repo = SeriesRepository::new(&pool);
        repo.apply_tmdb(id, &[tv_match("FR Dark", 70523, "Dark")])
            .await
            .unwrap();
        // Series 42 is renamed; a different series 77 takes its old title.
        import(
            pool.clone(),
            id,
            vec![
                Ok(episode_entry("Dark", 0, 0, "xtream://series/42")),
                Ok(episode_entry("FR Dark", 0, 0, "xtream://series/77")),
            ],
            ImportMode::Replace,
        )
        .await;
        let check = |pool: Arc<sqlx::SqlitePool>| async move {
            let list = SeriesRepository::new(&pool)
                .list_series(id, None, None, false, false, ContentSort::Default, 10, 0)
                .await
                .unwrap();
            let by = |t: &str| list.iter().find(|s| s.series_title == t).unwrap().clone();
            assert_eq!(by("Dark").canonical_title.as_deref(), Some("Dark"));
            assert_eq!(by("FR Dark").canonical_title, None, "no stale match shown");
            let queued = SeriesRepository::new(&pool)
                .list_for_tmdb(id, 3, 0, "en-US")
                .await
                .unwrap();
            assert_eq!(
                queued
                    .iter()
                    .map(|q| q.series_title.as_str())
                    .collect::<Vec<_>>(),
                ["FR Dark"],
                "the newcomer is matched on its own"
            );
        };
        check(pool.clone()).await;
        // Reopening must not promote a stale row onto the newcomer's entity.
        pool.close().await;
        let reopened = open_db(&path).await;
        check(reopened.clone()).await;
        let owned: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entity_external_matches WHERE namespace = 'tmdb:tv'",
        )
        .fetch_one(&*reopened)
        .await
        .unwrap();
        assert_eq!(owned, 1, "only series 42 owns a match");
    }

    #[tokio::test]
    async fn an_unresolved_decision_clears_the_series_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        import(
            pool.clone(),
            id,
            vec![Ok(episode_entry("Dark", 0, 0, "xtream://series/42"))],
            ImportMode::Append,
        )
        .await;
        let repo = SeriesRepository::new(&pool);
        repo.apply_tmdb(id, &[tv_match("Dark", 70523, "Dark")])
            .await
            .unwrap();
        let unresolved = nova_storage::SeriesTmdbWrite {
            series_title: "Dark".into(),
            matched: false,
            method: "search",
            matcher_version: 4,
            evidence_json: "{}".into(),
            ..Default::default()
        };
        repo.apply_tmdb(id, &[unresolved]).await.unwrap();
        let mirror: (Option<String>, Option<i64>, String) = sqlx::query_as(
            "SELECT canonical_title, tmdb_id, tmdb_status FROM series_meta WHERE series_title = 'Dark'",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(mirror, (None, None, "unmatched".into()));
        let genre: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT genre, genre_source FROM series_episodes WHERE series_title = 'Dark'",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(genre, (None, None), "only the TMDB genre is removed");
    }

    #[tokio::test]
    async fn a_decision_for_a_title_no_series_holds_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        // Renamed by an import while this decision was being made.
        SeriesRepository::new(&pool)
            .apply_tmdb(id, &[tv_match("Gone", 1, "Gone")])
            .await
            .unwrap();
        let rows: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM series_meta), (SELECT COUNT(*) FROM entity_external_matches)",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(rows, (0, 0), "no orphan mirror row to promote later");
    }

    #[tokio::test]
    async fn movie_user_state_apis_refuse_a_series_entity() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let catalog = vec![Ok(episode_entry("Show", 0, 0, "xtream://series/42"))];
        import(pool.clone(), id, catalog, ImportMode::Append).await;
        let series =
            nova_core::catalog::EntityId::new(series_entity(&pool, id, "Show").await).unwrap();
        let canonical = CanonicalRepository::new(&pool);
        assert!(!canonical.set_favorite(&series, true).await.unwrap());
        assert!(!canonical.set_in_watchlist(&series, true).await.unwrap());
        assert!(canonical
            .record_progress(&series, 60.0, Some(600.0), false, None)
            .await
            .is_err());
        let state: (i64, i64, i64) = sqlx::query_as(
            "SELECT is_favorite, is_in_watchlist, (SELECT COUNT(*) FROM entity_watch_state)
             FROM canonical_entities WHERE kind = 'series'",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 0, 0), "series entity untouched");
    }

    #[tokio::test]
    async fn episode_progress_survives_a_refresh_and_a_provider_rename() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let catalog = |title: &str| vec![Ok(episode_entry(title, 0, 0, "xtream://series/42"))];
        import(pool.clone(), id, catalog("FR Show"), ImportMode::Append).await;
        let series = SeriesRepository::new(&pool);
        series
            .replace_series_episodes(id, "FR Show", &opened_episodes(id, "FR Show"), None, None)
            .await
            .unwrap();
        let history = WatchHistoryRepository::new(&pool);
        let e2 = episode_id(&pool, "http://h:80/series/u/p/902.mkv").await;
        history
            .record_progress("episode", e2, 600.0, Some(2400.0), false, None)
            .await
            .unwrap();

        // Refresh under a new title: only the sentinel comes back.
        import(pool.clone(), id, catalog("Show"), ImportMode::Replace).await;
        assert_eq!(
            history.series_with_pending_episodes(id, 10).await.unwrap(),
            vec!["Show".to_string()],
            "re-fetched under the series' current title"
        );
        // Re-fetched: progress is found by the episode's durable key.
        series
            .replace_series_episodes(id, "Show", &opened_episodes(id, "Show"), None, None)
            .await
            .unwrap();
        let e2_new = episode_id(&pool, "http://h:80/series/u/p/902.mkv").await;
        let resume = history
            .resume_position("episode", e2_new)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((resume.position_sec, resume.completed), (600.0, false));
        let rail = history
            .continue_watching_section(nova_core::ContinueSection::Series, id, 10)
            .await
            .unwrap();
        assert_eq!(rail.len(), 1);
        assert_eq!((rail[0].media_id, rail[0].up_next), (e2_new, false));
        assert_eq!(rail[0].series_title.as_deref(), Some("Show"));
        let progress = history.episode_progress(id, "Show").await.unwrap();
        assert_eq!(progress.get(&e2_new), Some(&(600.0, Some(2400.0), false)));
        assert!(history
            .series_with_pending_episodes(id, 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn finishing_an_episode_offers_the_next_one() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        import(
            pool.clone(),
            id,
            vec![Ok(episode_entry("Show", 0, 0, "xtream://series/42"))],
            ImportMode::Append,
        )
        .await;
        SeriesRepository::new(&pool)
            .replace_series_episodes(id, "Show", &opened_episodes(id, "Show"), None, None)
            .await
            .unwrap();
        let e1 = episode_id(&pool, "http://h:80/series/u/p/901.mkv").await;
        let e2 = episode_id(&pool, "http://h:80/series/u/p/902.mkv").await;
        let history = WatchHistoryRepository::new(&pool);
        history
            .record_progress("episode", e1, 2400.0, Some(2400.0), true, None)
            .await
            .unwrap();
        let next = history.series_resume(id, "Show").await.unwrap().unwrap();
        assert_eq!((next.media_id, next.up_next), (e2, true));
        let state: (i64, i64, i64) =
            sqlx::query_as("SELECT season, episode, completed FROM episode_watch_state")
                .fetch_one(&*pool)
                .await
                .unwrap();
        assert_eq!(state, (1, 1, 1), "entity-owned, with its coordinates");
    }

    #[tokio::test]
    async fn an_unlisted_latest_episode_gives_way_to_the_previous_one() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("m3u", "test", "https://example.invalid/list.m3u")
            .await
            .unwrap()
            .id;
        let catalog = |second: &str| {
            vec![
                Ok(episode_entry(
                    "Show",
                    1,
                    1,
                    "https://example.invalid/s/1.mkv",
                )),
                Ok(episode_entry("Show", 1, 2, second)),
            ]
        };
        import(
            pool.clone(),
            id,
            catalog("https://example.invalid/s/2.mkv"),
            ImportMode::Append,
        )
        .await;
        let history = WatchHistoryRepository::new(&pool);
        let e1 = episode_id(&pool, "https://example.invalid/s/1.mkv").await;
        let e2 = episode_id(&pool, "https://example.invalid/s/2.mkv").await;
        history
            .record_progress("episode", e1, 300.0, Some(2400.0), false, None)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        history
            .record_progress("episode", e2, 900.0, Some(2400.0), false, None)
            .await
            .unwrap();
        // The latest episode's URL changes: its progress is orphaned.
        import(
            pool.clone(),
            id,
            catalog("https://example.invalid/s/2-new.mkv"),
            ImportMode::Replace,
        )
        .await;
        let e1_now = episode_id(&pool, "https://example.invalid/s/1.mkv").await;
        let card = history.series_resume(id, "Show").await.unwrap().unwrap();
        assert_eq!(
            (card.media_id, card.position_sec, card.up_next),
            (e1_now, 300.0, false),
            "the series stays on the rail through its previous episode"
        );
        assert!(
            history
                .series_with_pending_episodes(id, 10)
                .await
                .unwrap()
                .is_empty(),
            "an orphan of a listed series isn't worth a re-fetch"
        );
    }

    #[tokio::test]
    async fn progress_writes_wait_for_a_concurrent_writer() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("m3u", "test", "https://example.invalid/list.m3u")
            .await
            .unwrap()
            .id;
        import(
            pool.clone(),
            id,
            vec![Ok(episode_entry(
                "Show",
                1,
                1,
                "https://example.invalid/s/1.mkv",
            ))],
            ImportMode::Append,
        )
        .await;
        let ep = episode_id(&pool, "https://example.invalid/s/1.mkv").await;
        // Another connection (an import, a re-fetch) holds the write lock and
        // commits while the progress write is in flight.
        let mut writer = pool.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *writer)
            .await
            .unwrap();
        sqlx::query("UPDATE sources SET name = 'renamed' WHERE id = ?")
            .bind(id)
            .execute(&mut *writer)
            .await
            .unwrap();
        let background = pool.clone();
        let progress = tokio::spawn(async move {
            WatchHistoryRepository::new(&background)
                .record_progress("episode", ep, 120.0, Some(2400.0), false, None)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        sqlx::query("COMMIT").execute(&mut *writer).await.unwrap();
        drop(writer);
        progress
            .await
            .unwrap()
            .expect("the write waited instead of failing");
        let resume = WatchHistoryRepository::new(&pool)
            .resume_position("episode", ep)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resume.position_sec, 120.0);
    }

    #[tokio::test]
    async fn a_watched_m3u_series_keeps_its_progress_through_a_rename() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("m3u", "test", "https://example.invalid/list.m3u")
            .await
            .unwrap()
            .id;
        let catalog = |title: &str| {
            vec![
                Ok(episode_entry(
                    title,
                    1,
                    1,
                    "https://example.invalid/s/1.mkv",
                )),
                Ok(episode_entry(
                    title,
                    1,
                    2,
                    "https://example.invalid/s/2.mkv",
                )),
            ]
        };
        import(pool.clone(), id, catalog("Show"), ImportMode::Append).await;
        let entity = series_entity(&pool, id, "Show").await;
        let history = WatchHistoryRepository::new(&pool);
        let e1 = episode_id(&pool, "https://example.invalid/s/1.mkv").await;
        history
            .record_progress("episode", e1, 700.0, Some(2400.0), false, None)
            .await
            .unwrap();
        // No favourite or watchlist: the rename is seen through the episode.
        import(pool.clone(), id, catalog("The Show"), ImportMode::Replace).await;
        assert_eq!(series_entity(&pool, id, "The Show").await, entity);
        let card = history
            .series_resume(id, "The Show")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(card.position_sec, 700.0);
    }

    #[tokio::test]
    async fn re_attached_parked_history_reaches_the_series_entity() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        import(
            pool.clone(),
            id,
            vec![Ok(episode_entry("Show", 0, 0, "xtream://series/42"))],
            ImportMode::Append,
        )
        .await;
        // History parked before entity-owned progress existed.
        let key = nova_core::catalog::media::episode_binding_key("http://h:80/series/u/p/902.mkv");
        sqlx::query(
            "INSERT INTO pending_episode_history (source_id, episode_key, series_title, position_sec, duration_sec, watched_at, completed) VALUES (?, ?, 'Show', 777, 2400, 5, 0)",
        )
        .bind(id)
        .bind(&key)
        .execute(&*pool)
        .await
        .unwrap();
        let history = WatchHistoryRepository::new(&pool);
        assert_eq!(
            history.series_with_pending_episodes(id, 10).await.unwrap(),
            vec!["Show".to_string()],
            "legacy parked series are still re-fetched"
        );
        SeriesRepository::new(&pool)
            .replace_series_episodes(id, "Show", &opened_episodes(id, "Show"), None, None)
            .await
            .unwrap();
        let e2 = episode_id(&pool, "http://h:80/series/u/p/902.mkv").await;
        let owned: f64 = sqlx::query_scalar(
            "SELECT position_sec FROM episode_watch_state WHERE episode_key = ?",
        )
        .bind(&key)
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(owned, 777.0, "entity-owned through the episode row");
        let rail = history
            .continue_watching_section(nova_core::ContinueSection::Series, id, 10)
            .await
            .unwrap();
        assert_eq!(rail[0].media_id, e2);
    }

    #[tokio::test]
    async fn a_series_decision_follows_its_entity_through_a_mid_match_rename() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let catalog = |title: &str| vec![Ok(episode_entry(title, 0, 0, "xtream://series/42"))];
        import(pool.clone(), id, catalog("FR Dark"), ImportMode::Append).await;
        let repo = SeriesRepository::new(&pool);
        let queued = repo.list_for_tmdb(id, 3, 0, "en-US").await.unwrap();
        // An import renames the series while TMDB is being queried.
        import(pool.clone(), id, catalog("Dark"), ImportMode::Replace).await;
        let decision = nova_storage::SeriesTmdbWrite {
            entity_id: queued[0].entity_id.clone(),
            ..tv_match("FR Dark", 70523, "Dark")
        };
        repo.apply_tmdb(id, &[decision]).await.unwrap();
        let list = repo
            .list_series(id, None, None, false, false, ContentSort::Default, 10, 0)
            .await
            .unwrap();
        assert_eq!(list[0].series_title, "Dark");
        assert_eq!(
            list[0].canonical_title.as_deref(),
            Some("Dark"),
            "landed on its series"
        );
    }

    #[tokio::test]
    async fn the_overview_rail_reads_entity_progress_under_display_titles() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        import(
            pool.clone(),
            id,
            vec![Ok(episode_entry("FR Dark", 0, 0, "xtream://series/42"))],
            ImportMode::Append,
        )
        .await;
        let repo = SeriesRepository::new(&pool);
        repo.replace_series_episodes(id, "FR Dark", &opened_episodes(id, "FR Dark"), None, None)
            .await
            .unwrap();
        repo.apply_tmdb(id, &[tv_match("FR Dark", 70523, "Dark")])
            .await
            .unwrap();
        let e1 = episode_id(&pool, "http://h:80/series/u/p/901.mkv").await;
        let history = WatchHistoryRepository::new(&pool);
        history
            .record_progress("episode", e1, 42.0, Some(2400.0), false, None)
            .await
            .unwrap();
        // The legacy mirror row alone would say something else.
        sqlx::query("DELETE FROM watch_history")
            .execute(&*pool)
            .await
            .unwrap();
        let overview = history.list_continue_watching(10).await.unwrap();
        assert_eq!(overview.len(), 1);
        assert_eq!(
            (overview[0].media_id, overview[0].title.as_str()),
            (e1, "Dark")
        );
    }

    #[tokio::test]
    async fn resume_falls_back_to_legacy_history_without_entity_state() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("m3u", "test", "https://example.invalid/list.m3u")
            .await
            .unwrap()
            .id;
        import(
            pool.clone(),
            id,
            vec![Ok(episode_entry(
                "Show",
                1,
                1,
                "https://example.invalid/s/1.mkv",
            ))],
            ImportMode::Append,
        )
        .await;
        let e1 = episode_id(&pool, "https://example.invalid/s/1.mkv").await;
        sqlx::query(
            "INSERT INTO watch_history (media_kind, media_id, position_sec, watched_at) VALUES ('episode', ?, 88, 1)",
        )
        .bind(e1)
        .execute(&*pool)
        .await
        .unwrap();
        let resume = WatchHistoryRepository::new(&pool)
            .resume_position("episode", e1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resume.position_sec, 88.0);
    }

    #[tokio::test]
    async fn cannot_link_refuses_a_series_and_sentinel_ids_bind_numerically() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        // Two sentinels share a title: the smaller ID wins (text order would
        // pick "20").
        import(
            pool.clone(),
            id,
            vec![
                Ok(episode_entry("Show", 0, 0, "xtream://series/20")),
                Ok(episode_entry("Show", 0, 0, "xtream://series/10")),
                Ok(entry("Heat")),
            ],
            ImportMode::Append,
        )
        .await;
        let key: String = sqlx::query_scalar("SELECT binding_key FROM series_bindings")
            .fetch_one(&*pool)
            .await
            .unwrap();
        assert_eq!(
            key,
            nova_core::catalog::media::series_binding_key(Some("10"), "Show")
        );
        let series =
            nova_core::catalog::EntityId::new(series_entity(&pool, id, "Show").await).unwrap();
        let movie: String =
            sqlx::query_scalar("SELECT id FROM canonical_entities WHERE kind = 'movie'")
                .fetch_one(&*pool)
                .await
                .unwrap();
        let movie = nova_core::catalog::EntityId::new(movie).unwrap();
        assert!(!CanonicalRepository::new(&pool)
            .record_cannot_link(&movie, &series)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn parked_progress_reaches_the_entity_of_a_renamed_rotated_series() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("m3u", "test", "https://example.invalid/list.m3u")
            .await
            .unwrap()
            .id;
        let catalog = |title: &str, pass: &str| {
            vec![Ok(episode_entry(
                title,
                1,
                1,
                &format!("http://h:80/series/u/{pass}/555.mkv"),
            ))]
        };
        import(pool.clone(), id, catalog("Show", "old"), ImportMode::Append).await;
        let history = WatchHistoryRepository::new(&pool);
        let ep = episode_id(&pool, "http://h:80/series/u/old/555.mkv").await;
        history
            .record_progress("episode", ep, 900.0, Some(2400.0), false, None)
            .await
            .unwrap();
        // Renamed and credentials rotated at once: no URL links the titles, so
        // the history is parked and re-attached by the episode's durable key.
        import(
            pool.clone(),
            id,
            catalog("The Show", "new"),
            ImportMode::Replace,
        )
        .await;
        let card = history
            .continue_watching_section(nova_core::ContinueSection::Series, id, 10)
            .await
            .unwrap();
        assert_eq!(card.len(), 1, "on the rail through the new title's entity");
        assert_eq!(card[0].series_title.as_deref(), Some("The Show"));
        assert_eq!(card[0].position_sec, 900.0);
    }

    #[tokio::test]
    async fn a_series_id_resolves_to_where_the_series_is_listed_now() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let catalog = |title: &str| {
            vec![
                Ok(episode_entry(title, 0, 0, "xtream://series/42")),
                Ok(entry("Heat")),
            ]
        };
        import(pool.clone(), id, catalog("FR Show"), ImportMode::Append).await;
        let series = series_entity(&pool, id, "FR Show").await;
        assert_eq!(
            series_key(&pool, &series).await.unwrap(),
            (id, "FR Show".to_string())
        );
        // A frontend reference made before the provider renamed it still works.
        import(pool.clone(), id, catalog("Show"), ImportMode::Replace).await;
        assert_eq!(
            series_key(&pool, &series).await.unwrap(),
            (id, "Show".to_string())
        );
        let movie: String =
            sqlx::query_scalar("SELECT id FROM canonical_entities WHERE kind = 'movie'")
                .fetch_one(&*pool)
                .await
                .unwrap();
        for not_a_listed_series in [movie.as_str(), "ser-unknown"] {
            assert!(matches!(
                series_key(&pool, not_a_listed_series).await,
                Err(ApiError::NotFound(_))
            ));
        }
    }

    fn engine_state(
        token: &str,
        phase: nova_mpv_player::MpvPlaybackPhase,
        position_sec: f64,
    ) -> nova_mpv_player::MpvPlayerState {
        nova_mpv_player::MpvPlayerState {
            token: Some(token.into()),
            engine: "libmpv".into(),
            phase,
            position_sec,
            duration_sec: Some(2400.0),
            volume: 100.0,
            muted: false,
            seekable: true,
            speed: 1.0,
            audio_track_id: None,
            sub_track_id: None,
            video_track_id: None,
            tracks: Vec::new(),
            error: None,
        }
    }

    #[tokio::test]
    async fn waking_reconnects_a_playing_live_channel_and_leaves_the_rest_paused() {
        use crate::playback_coordinator::{
            PlaybackCoordinator, PlaybackEvent, PlaybackRecover, RecoverReason,
        };
        use nova_mpv_player::MpvPlaybackPhase::{Error, Paused, Playing};
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let coordinator = PlaybackCoordinator::new(nova_mpv_player::MpvEngine::new());
        coordinator.set_wake_settle_ms(20);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PlaybackRecover>();
        coordinator.start(std::sync::Arc::new(move |event| {
            if let PlaybackEvent::Recover(r) = event {
                let _ = tx.send(r);
            }
        }));
        let play = |token: &'static str, kind: &'static str, phase| {
            let (coordinator, pool) = (coordinator.clone(), pool.clone());
            async move {
                coordinator
                    .begin(token.into(), pool, kind.into(), 7, None)
                    .await;
                coordinator
                    .observe_at(engine_state(token, phase, 30.0), 1_000)
                    .await;
            }
        };
        async fn next(
            rx: &mut tokio::sync::mpsc::UnboundedReceiver<PlaybackRecover>,
        ) -> Option<PlaybackRecover> {
            tokio::time::timeout(Duration::from_millis(300), rx.recv())
                .await
                .ok()
                .flatten()
        }

        // A live channel playing through sleep reconnects once the network
        // had a moment, once.
        play("live", "live", Playing).await;
        coordinator.sleep().await;
        coordinator.wake().await;
        let r = next(&mut rx).await.expect("a reconnect");
        assert_eq!(
            (
                r.token.as_str(),
                r.media_kind.as_str(),
                r.media_id,
                r.reason,
                r.other_source
            ),
            ("live", "live", 7, RecoverReason::Wake, false)
        );
        coordinator.wake().await;
        assert!(
            next(&mut rx).await.is_none(),
            "a second wake without sleep does nothing"
        );

        // A connection dropped by sleep isn't a failing source: a channel that
        // already spent its retry gets a fresh one after the wake reconnect.
        play("live-0", "live", Playing).await;
        coordinator
            .observe_at(engine_state("live-0", Error, 30.0), 2_000)
            .await;
        assert_eq!(next(&mut rx).await.unwrap().reason, RecoverReason::Failure);
        play("live-1", "live", Playing).await; // the recovery's reopen
        coordinator.sleep().await;
        coordinator.wake().await;
        assert_eq!(next(&mut rx).await.unwrap().reason, RecoverReason::Wake);
        play("live-2", "live", Playing).await; // the wake's reopen
        coordinator
            .observe_at(engine_state("live-2", Error, 30.0), 3_000)
            .await;
        let failure = next(&mut rx).await.expect("a fresh retry after the wake");
        assert_eq!(failure.reason, RecoverReason::Failure);

        // A movie stays paused for the user; so does a live channel the user
        // had paused.
        play("vod", "vod", Playing).await;
        coordinator.sleep().await;
        coordinator.wake().await;
        play("paused-live", "live", Paused).await;
        coordinator.sleep().await;
        coordinator.wake().await;
        assert!(next(&mut rx).await.is_none());

        // A live channel replaced between sleep and wake isn't reconnected.
        play("live-a", "live", Playing).await;
        coordinator.sleep().await;
        play("live-b", "live", Playing).await;
        coordinator.wake().await;
        assert!(next(&mut rx).await.is_none());

        // Sleeping again before the reconnect fires cancels it.
        play("live-c", "live", Playing).await;
        coordinator.sleep().await;
        coordinator.wake().await;
        coordinator.sleep().await;
        assert!(
            next(&mut rx).await.is_none(),
            "no reconnect into a sleeping Mac"
        );
    }

    #[tokio::test]
    async fn the_playback_coordinator_owns_resume_progress_completion_and_next() {
        use crate::playback_coordinator::{PlaybackCoordinator, PlaybackEvent};
        use nova_mpv_player::MpvPlaybackPhase::{Ended, Playing};

        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("m3u", "test", "https://example.invalid/list.m3u")
            .await
            .unwrap()
            .id;
        let catalog = (1..=3)
            .map(|n| {
                Ok(episode_entry(
                    "Show",
                    1,
                    n,
                    &format!("https://example.invalid/s/{n}.mkv"),
                ))
            })
            .collect();
        import(pool.clone(), id, catalog, ImportMode::Append).await;
        let ep = |n: u32| {
            let pool = pool.clone();
            async move { episode_id(&pool, &format!("https://example.invalid/s/{n}.mkv")).await }
        };
        let (e1, e2, e3) = (ep(1).await, ep(2).await, ep(3).await);
        let history = WatchHistoryRepository::new(&pool);
        history
            .record_progress("episode", e1, 600.0, Some(2400.0), false, None)
            .await
            .unwrap();
        let coordinator = PlaybackCoordinator::new(nova_mpv_player::MpvEngine::new());
        let (ended_tx, mut ended_rx) = tokio::sync::mpsc::unbounded_channel();
        coordinator.start(std::sync::Arc::new(move |event| {
            if let PlaybackEvent::Ended(p) = event {
                let _ = ended_tx.send(*p);
            }
        }));

        // E1 resumes: the coordinator asks for the seek (rejected here: no
        // engine session) and never writes the start position while the
        // resume hasn't taken effect, not even when closed.
        coordinator
            .begin("tok-1".into(), pool.clone(), "episode".into(), e1, None)
            .await;
        coordinator
            .observe(engine_state("tok-1", Playing, 0.0))
            .await;
        coordinator
            .observe(engine_state("tok-1", Playing, 0.5))
            .await;
        coordinator.finish(Some("tok-1")).await;
        let e1_resume = history
            .resume_position("episode", e1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(e1_resume.position_sec, 600.0, "resume point kept");

        // E2 plays: progress is written by Rust, snapshots of another token
        // are ignored, and the end marks it finished and names E3.
        coordinator
            .begin("tok-2".into(), pool.clone(), "episode".into(), e2, None)
            .await;
        coordinator
            .observe(engine_state("tok-2", Playing, 42.0))
            .await;
        let written = history
            .resume_position("episode", e2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((written.position_sec, written.completed), (42.0, false));
        coordinator
            .observe(engine_state("stale", Ended, 2400.0))
            .await;
        coordinator.finish(Some("stale")).await;
        let still_playing = history
            .resume_position("episode", e2)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !still_playing.completed,
            "a replaced session's snapshot is ignored"
        );
        coordinator
            .observe(engine_state("tok-2", Ended, 2400.0))
            .await;
        let ended = tokio::time::timeout(std::time::Duration::from_secs(5), ended_rx.recv())
            .await
            .expect("playback:ended announced")
            .unwrap();
        assert_eq!(ended.token, "tok-2");
        assert!(ended.autoplay, "the profile default");
        assert_eq!(ended.next_episode.map(|e| e.id), Some(e3));
        let done = history
            .resume_position("episode", e2)
            .await
            .unwrap()
            .unwrap();
        assert!(done.completed);
    }

    #[tokio::test]
    async fn a_failing_source_is_retried_then_replaced_and_remembered() {
        use crate::playback_coordinator::{PlaybackCoordinator, PlaybackEvent, PlaybackRecover};
        use nova_mpv_player::MpvPlaybackPhase::{Error, Playing};

        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let sources = SourceRepository::new(&pool);
        let heat = |raw: &str, quality: &[&str], id: u32| ParsedEntry {
            clean_title: "Heat".into(),
            year: Some(1995),
            quality_hints: quality.iter().map(|q| q.to_string()).collect(),
            ..xtream_movie(raw, id, "p")
        };
        // One film, two providers: the full-HD copy ranks first.
        let a = sources
            .create("xtream", "a", "http://a/get.php")
            .await
            .unwrap()
            .id;
        let b = sources
            .create("xtream", "b", "http://b/get.php")
            .await
            .unwrap()
            .id;
        import(
            pool.clone(),
            a,
            vec![Ok(heat("Heat (1995) FHD", &["FHD"], 1))],
            ImportMode::Append,
        )
        .await;
        import(
            pool.clone(),
            b,
            vec![Ok(heat("Heat (1995)", &[], 2))],
            ImportMode::Append,
        )
        .await;
        let canonical = CanonicalRepository::new(&pool);
        let entity: String = sqlx::query_scalar(
            "SELECT id FROM canonical_entities WHERE kind = 'movie' AND state = 'active'",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        let entity = nova_core::catalog::EntityId::new(entity).unwrap();
        let row = |url: &str| {
            let pool = pool.clone();
            let url = url.to_string();
            async move {
                sqlx::query_scalar::<_, i64>("SELECT id FROM vod_items WHERE stream_url = ?")
                    .bind(url)
                    .fetch_one(&*pool)
                    .await
                    .unwrap()
            }
        };
        let (row_fhd, row_plain) = (
            row("http://host/movie/user/p/1.mkv").await,
            row("http://host/movie/user/p/2.mkv").await,
        );
        let first = canonical
            .resolve_playback(&entity, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.media_id, row_fhd, "the full-HD copy plays first");

        let coordinator = PlaybackCoordinator::new(nova_mpv_player::MpvEngine::new());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        coordinator.start(std::sync::Arc::new(move |event| {
            if let PlaybackEvent::Recover(r) = event {
                let _ = tx.send(r);
            }
        }));
        let begin = |token: &'static str, kind: &'static str, row: i64| {
            let coordinator = coordinator.clone();
            let pool = pool.clone();
            async move {
                coordinator
                    .begin(token.into(), pool, kind.into(), row, None)
                    .await
            }
        };
        let at = |token: &'static str, phase, now: i64| {
            let coordinator = coordinator.clone();
            async move {
                coordinator
                    .observe_at(engine_state(token, phase, 1.0), now)
                    .await
            }
        };
        let expect = |r: PlaybackRecover, token: &str, row: i64, attempt: u32, other: bool| {
            assert_eq!(
                (r.token.as_str(), r.media_id, r.attempt, r.other_source),
                (token, row, attempt, other)
            );
        };

        // The first source fails: one retry of it, then the other source.
        begin("t1", "vod", row_fhd).await;
        at("t1", Error, 0).await;
        expect(rx.try_recv().unwrap(), "t1", row_fhd, 1, false);
        begin("t2", "vod", row_fhd).await;
        at("t2", Error, 1_000).await;
        expect(rx.try_recv().unwrap(), "t2", row_plain, 2, true);
        // The other source plays; after 20 s it counts as working.
        begin("t3", "vod", row_plain).await;
        at("t3", Playing, 2_000).await;
        let health = || async {
            sqlx::query_as::<_, (i64, bool)>(
                "SELECT failures, last_success_at IS NOT NULL FROM variant_health h
                 JOIN stream_variants s ON s.id = h.variant_id ORDER BY s.vod_item_id",
            )
            .fetch_all(&*pool)
            .await
            .unwrap()
        };
        assert_eq!(health().await, [(2, false)], "not healthy after 1 s");
        at("t3", Playing, 23_000).await;
        assert_eq!(
            health().await,
            [(2, false), (0, true)],
            "per-source evidence"
        );
        let next = canonical
            .resolve_playback(&entity, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.media_id, row_plain, "the failing copy is demoted");

        // Later it fails too: every source gets tried once, then it stops.
        at("t3", Error, 60_000).await;
        expect(rx.try_recv().unwrap(), "t3", row_plain, 3, false);
        begin("t4", "vod", row_plain).await;
        at("t4", Error, 61_000).await;
        expect(rx.try_recv().unwrap(), "t4", row_fhd, 4, true);
        begin("t5", "vod", row_fhd).await;
        at("t5", Error, 62_000).await;
        assert!(rx.try_recv().is_err(), "no untried source left");

        // Pressing play again is the user's call: a fresh set of recoveries.
        begin("t6", "vod", row_fhd).await;
        at("t6", Error, 63_000).await;
        expect(rx.try_recv().unwrap(), "t6", row_fhd, 1, false);

        // A live channel that plays and drops again and again stays bounded.
        let mut recoveries = 0;
        for n in 0..8_i64 {
            let token: &'static str = Box::leak(format!("live-{n}").into_boxed_str());
            let start = 100_000 + n * 30_000;
            begin(token, "live", 999).await;
            at(token, Playing, start).await;
            at(token, Playing, start + 21_000).await; // healthy, then drops
            at(token, Error, start + 22_000).await;
            // The player reopens only what recovery asks for.
            if rx.try_recv().is_err() {
                break;
            }
            recoveries += 1;
        }
        assert_eq!(recoveries, 5, "at most five recoveries per ten minutes");
    }

    #[tokio::test]
    async fn deleting_a_source_keeps_series_entities() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let sources = SourceRepository::new(&pool);
        let id = sources
            .create("xtream", "test", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let catalog = vec![Ok(episode_entry("Show", 0, 0, "xtream://series/42"))];
        import(pool.clone(), id, catalog, ImportMode::Append).await;
        SeriesRepository::new(&pool)
            .set_favorite(id, "Show", true)
            .await
            .unwrap();
        sources.delete(id).await.unwrap();
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM canonical_entities WHERE kind = 'series' AND is_favorite = 1),
                    (SELECT COUNT(*) FROM series_bindings)",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(
            counts,
            (1, 0),
            "entity and state outlive the source's bindings"
        );
    }

    #[tokio::test]
    async fn import_keeps_raw_evidence_next_to_parsed_values() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_db(&dir.path().join("p.db")).await;
        let id = SourceRepository::new(&pool)
            .create("m3u", "test", "https://example.invalid/list.m3u")
            .await
            .unwrap()
            .id;
        // Real parser output, as the M3U/Xtream converters produce it.
        let parsed = |raw: &str, kind: MediaKind, group: &str| {
            let title_kind = match &kind {
                MediaKind::Live => nova_core::clean::TitleKind::Live,
                MediaKind::Movie => nova_core::clean::TitleKind::Movie,
                MediaKind::Series => nova_core::clean::TitleKind::Series,
            };
            let p = nova_core::clean::parse_source_title(raw, title_kind);
            let is_series = matches!(kind, MediaKind::Series);
            ParsedEntry {
                kind,
                name: raw.into(),
                group_title: group.into(),
                stream_url: format!("https://example.invalid/{}", raw.len()),
                country_prefix: p.country_prefix.clone(),
                clean_title: p.title.clone(),
                year: p.year,
                series_title: is_series.then(|| p.title.clone()),
                season: p.season,
                episode: p.episode,
                quality_hints: p.quality_hints.clone(),
                language_hints: p.language_hints.clone(),
                parser_version: Some(nova_core::clean::PARSER_VERSION),
                category: Some(nova_core::normalize_category(group)),
                ..entry(raw)
            }
        };
        let ok = import(
            pool.clone(),
            id,
            vec![
                Ok(parsed(
                    "FR | OPPENHEIMER [4K] MULTI (2023)",
                    MediaKind::Movie,
                    "FR| FILMS 4K",
                )),
                Ok(parsed("AR | MBC 1 FHD", MediaKind::Live, "|AR| MBC")),
                Ok(parsed(
                    "DE | Tatort - Folge 1234",
                    MediaKind::Series,
                    "DE| SERIEN",
                )),
                Ok(parsed("Breaking Bad", MediaKind::Series, "SERIES")),
            ],
            ImportMode::Append,
        )
        .await;
        assert!(matches!(ok.stage, ImportStage::Complete));

        type MovieEvidence = (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        );
        let movie: MovieEvidence =
            sqlx::query_as(
                "SELECT name, clean_title, raw_group, quality_hints, language_hints, parser_version FROM vod_items",
            )
            .fetch_one(&*pool)
            .await
            .unwrap();
        assert_eq!(
            movie.0, "FR | OPPENHEIMER [4K] MULTI (2023)",
            "raw title untouched"
        );
        assert_eq!(movie.1.as_deref(), Some("OPPENHEIMER"));
        assert_eq!(movie.2.as_deref(), Some("FR| FILMS 4K"), "raw group kept");
        assert_eq!(
            (movie.3.as_deref(), movie.4.as_deref()),
            (Some("4K"), Some("MULTI"))
        );
        assert_eq!(movie.5, Some(i64::from(nova_core::clean::PARSER_VERSION)));
        // "FR| FILMS 4K" is a provider collection, never a Nova genre.
        let taxonomy: (Option<String>, Option<String>, Option<String>) =
            sqlx::query_as("SELECT collection, genre, category FROM vod_items")
                .fetch_one(&*pool)
                .await
                .unwrap();
        assert_eq!(
            taxonomy,
            (Some("Movies".into()), None, Some("Movies".into()))
        );

        let channel: (String, Option<String>, Option<String>) =
            sqlx::query_as("SELECT name, raw_name, quality_hints FROM channels")
                .fetch_one(&*pool)
                .await
                .unwrap();
        assert_eq!(channel.0, "MBC 1");
        assert_eq!(channel.1.as_deref(), Some("AR | MBC 1 FHD"));
        assert_eq!(channel.2.as_deref(), Some("FHD"));

        let episodes = SeriesRepository::new(&pool);
        let tatort = episodes.list_episodes(id, "Tatort").await.unwrap();
        assert_eq!(tatort[0].episode, 1234);
        assert_eq!(
            tatort[0].numbering,
            nova_core::EpisodeNumberingKind::Absolute
        );
        let bb = episodes.list_episodes(id, "Breaking Bad").await.unwrap();
        assert_eq!(
            bb[0].numbering,
            nova_core::EpisodeNumberingKind::Unknown,
            "placeholder numbers are flagged, not presented as S01E01"
        );
        let raw: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT raw_name, raw_group FROM series_episodes WHERE series_title = 'Tatort'",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert_eq!(raw.0.as_deref(), Some("DE | Tatort - Folge 1234"));
        assert_eq!(raw.1.as_deref(), Some("DE| SERIEN"));
    }

    #[tokio::test]
    async fn successful_imports_publish_an_active_generation() {
        let dir = tempfile::tempdir().unwrap();
        let (pool, id, _) = published_library(&dir.path().join("p.db")).await;
        let publications = PublicationRepository::new(&pool);
        let first = publications.active(id).await.unwrap().unwrap();
        assert_eq!(
            (first.mode.as_str(), first.status.as_str()),
            ("append", "published")
        );
        assert_eq!(first.counts.movie, 60);
        let mut next = movies(0..70);
        next.push(Ok(live("News 24")));
        let ok = import(pool.clone(), id, next, ImportMode::Replace).await;
        assert!(matches!(ok.stage, ImportStage::Complete));
        let second = publications.active(id).await.unwrap().unwrap();
        assert!(second.id > first.id);
        assert_eq!(second.mode, "replace");
        assert_eq!((second.counts.movie, second.counts.live), (70, 1));
    }

    #[tokio::test]
    async fn a_collapsed_refresh_is_rejected_and_the_prior_generation_stays_active() {
        let dir = tempfile::tempdir().unwrap();
        let (pool, id, entity) = published_library(&dir.path().join("p.db")).await;
        let active = PublicationRepository::new(&pool)
            .active(id)
            .await
            .unwrap()
            .unwrap();
        sqlx::query("UPDATE sources SET last_refreshed_at = 0 WHERE id = ?")
            .bind(id)
            .execute(&*pool)
            .await
            .unwrap();

        // The VOD endpoint answers with a truncated list and no error.
        let failed = import(pool.clone(), id, movies(0..3), ImportMode::Replace).await;
        assert!(matches!(failed.stage, ImportStage::Failed));
        let message = failed.error.unwrap();
        assert!(message.starts_with("Refresh rejected"), "{message}");
        assert!(message.contains("previous catalog retained"), "{message}");

        assert_prior_generation(&pool, id, &entity, active.id).await;
        let source = SourceRepository::new(&pool).get(id).await.unwrap().unwrap();
        assert_eq!(
            source.last_refreshed_at,
            Some(0),
            "a rejected refresh is not a refresh"
        );
        let latest = &PublicationRepository::new(&pool)
            .history(id, 1)
            .await
            .unwrap()[0];
        assert_eq!(
            (latest.status.as_str(), latest.reason_code.as_deref()),
            ("rejected", Some("collapse"))
        );
        assert_eq!(latest.counts.movie, 3, "records what the provider sent");
    }

    #[tokio::test]
    async fn every_failure_kind_leaves_the_prior_generation_and_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let (pool, id, entity) = published_library(&dir.path().join("p.db")).await;
        let active = PublicationRepository::new(&pool)
            .active(id)
            .await
            .unwrap()
            .unwrap();

        // An endpoint error midway, after rows were already written.
        let mut partial = movies(0..60);
        partial.insert(30, Err("get_series: HTTP 502".into()));
        let failed = import(pool.clone(), id, partial, ImportMode::Replace).await;
        assert!(matches!(failed.stage, ImportStage::Failed));
        // A provider that returns nothing at all.
        let empty = import(pool.clone(), id, Vec::new(), ImportMode::Replace).await;
        assert!(matches!(empty.stage, ImportStage::Failed));

        assert_prior_generation(&pool, id, &entity, active.id).await;
        let history = PublicationRepository::new(&pool)
            .history(id, 10)
            .await
            .unwrap();
        let codes: Vec<_> = history
            .iter()
            .map(|p| (p.status.as_str(), p.reason_code.as_deref()))
            .collect();
        assert_eq!(
            codes,
            vec![
                ("failed", Some("no_entries")),
                ("failed", Some("incomplete_refresh")),
                ("published", None),
            ]
        );
        // Persisted reasons are secret-free summaries, never provider text.
        assert!(history
            .iter()
            .all(|p| !p.reason.as_deref().unwrap_or("").contains("502")));

        // A good refresh afterwards publishes normally.
        let ok = import(pool.clone(), id, movies(0..60), ImportMode::Replace).await;
        assert!(matches!(ok.stage, ImportStage::Complete));
        let now_active = PublicationRepository::new(&pool)
            .active(id)
            .await
            .unwrap()
            .unwrap();
        assert!(now_active.id > active.id);
        assert!(
            CanonicalRepository::new(&pool)
                .movie_detail(&entity)
                .await
                .unwrap()
                .unwrap()
                .summary
                .is_favorite
        );
    }

    #[tokio::test]
    async fn an_in_flight_candidate_is_invisible_until_it_publishes() {
        let dir = tempfile::tempdir().unwrap();
        let (pool, id, entity) = published_library(&dir.path().join("p.db")).await;
        let active = PublicationRepository::new(&pool)
            .active(id)
            .await
            .unwrap()
            .unwrap();

        let (entries, rx) = tokio::sync::mpsc::channel(2_000);
        let (events, mut progress) = tokio::sync::mpsc::unbounded_channel();
        let worker = tokio::spawn(run_import(
            id,
            rx,
            pool.clone(),
            move |p| {
                let _ = events.send(p);
            },
            ImportMode::Replace,
        ));
        // 500 entries trigger a Receiving event, emitted only after the
        // candidate has cleared the old rows and started writing.
        for e in movies(1_000..1_500) {
            entries.send(e).await.unwrap();
        }
        loop {
            let p = progress.recv().await.unwrap();
            if matches!(p.stage, ImportStage::Receiving) && p.parsed >= 500 {
                break;
            }
        }
        // Another connection still reads the active generation, intact.
        assert_prior_generation(&pool, id, &entity, active.id).await;

        for e in movies(1_500..1_600) {
            entries.send(e).await.unwrap();
        }
        drop(entries);
        assert!(worker.await.unwrap(), "candidate publishes");
        assert_eq!(vod_count(&pool, id).await, 600);
        let now_active = PublicationRepository::new(&pool)
            .active(id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(now_active.counts.movie, 600);
        // User state is entity-owned and survives the publish (the movie is
        // gone from this source, but its entity keeps favourite and resume).
        let resume = CanonicalRepository::new(&pool)
            .resume(&entity)
            .await
            .unwrap();
        assert_eq!(resume.unwrap().position_sec, 42.0);
    }

    #[tokio::test]
    async fn rejected_and_abandoned_candidates_leave_the_prior_generation_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.db");
        let (pool, id, entity) = published_library(&path).await;
        let active = PublicationRepository::new(&pool)
            .active(id)
            .await
            .unwrap()
            .unwrap();

        // 1. A rejected candidate, then reopen.
        let failed = import(pool.clone(), id, movies(0..2), ImportMode::Replace).await;
        assert!(matches!(failed.stage, ImportStage::Failed));
        pool.close().await;
        let pool = open_db(&path).await;
        assert_prior_generation(&pool, id, &entity, active.id).await;

        // 2. A candidate abandoned mid-build (app killed during refresh): the
        //    worker is aborted with its transaction open, then the pool closes.
        let (entries, rx) = tokio::sync::mpsc::channel(1_000);
        let (events, mut progress) = tokio::sync::mpsc::unbounded_channel();
        let worker = tokio::spawn(run_import(
            id,
            rx,
            pool.clone(),
            move |p| {
                let _ = events.send(p);
            },
            ImportMode::Replace,
        ));
        for e in movies(5_000..5_500) {
            entries.send(e).await.unwrap();
        }
        loop {
            let p = progress.recv().await.unwrap();
            if matches!(p.stage, ImportStage::Receiving) && p.parsed >= 500 {
                break;
            }
        }
        worker.abort();
        let _ = worker.await;
        drop(entries);
        pool.close().await;

        let pool = open_db(&path).await;
        assert_prior_generation(&pool, id, &entity, active.id).await;
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&*pool)
            .await
            .unwrap();
        assert_eq!(integrity, "ok");

        // 3. The reopened profile publishes the next valid candidate.
        let ok = import(pool.clone(), id, movies(0..61), ImportMode::Replace).await;
        assert!(matches!(ok.stage, ImportStage::Complete));
        pool.close().await;
        let pool = open_db(&path).await;
        let now_active = PublicationRepository::new(&pool)
            .active(id)
            .await
            .unwrap()
            .unwrap();
        assert!(now_active.id > active.id);
        assert_eq!(now_active.counts.movie, 61);
        assert_eq!(vod_count(&pool, id).await, 61);
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
