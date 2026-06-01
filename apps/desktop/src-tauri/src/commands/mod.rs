use nova_core::{
    Channel, ContentSort, ContinueWatchingItem, ImportProgress, ImportStage, MediaKind,
    ParsedEntry, Profile, SeriesEpisode, SeriesSummary, Source, VodItem,
};
use nova_storage::{
    init_profile_db, ChannelRepository, EpgSourcesRepository, ProfileRepository, SeriesRepository,
    SourceRepository, VodRepository, WatchHistoryRepository, WatchSnapshot,
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::ipc::ApiError;
use crate::state::AppState;

use std::time::{Duration, Instant};

pub mod caption_style;
pub mod enrich;
pub mod epg;
pub mod playback;
pub mod profile_settings;
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
    open_profile_db(&profile.id, &state, &app).await?;

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

    // Ensure per-profile DB is open
    if state.profile_pool(&id).await.is_none() {
        open_profile_db(&id, &state, &app).await?;
    }

    *state.active_profile_id.write().await = Some(id);
    Ok(profile)
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
    start_import(pool, app, source, ImportMode::Append).await;
    Ok(source_id)
}

/// How `run_import` treats existing rows for the source.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportMode {
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
    let source = SourceRepository::new(&pool)
        .get(source_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("source {source_id} not found")))?;
    start_import(pool, app, source, ImportMode::Replace).await;
    Ok(())
}

/// Shared kickoff for add + refresh: detect Xtream vs M3U, register the EPG
/// feed, and spawn the streaming import task (returns once streaming has begun).
async fn start_import(
    pool: std::sync::Arc<sqlx::SqlitePool>,
    app: AppHandle,
    source: Source,
    mode: ImportMode,
) {
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
                        tracing::info!(source_id, url = %url_tvg, "M3U url-tvg registered");
                    }
                }
            }
        });
    }

    // Spawn import task — returns immediately, streams progress events.
    tokio::spawn(async move {
        run_import(source_id, rx, pool, app, mode).await;
    });
}

// ── Browse commands (Phase 2) ─────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelFilters {
    country: Option<String>,
    category: Option<String>,
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
    let query = normalize_filter(filters.query);
    ChannelRepository::new(&pool)
        .list(
            source_id,
            limit,
            offset,
            country.as_deref(),
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
    let query = normalize_filter(filters.query);
    ChannelRepository::new(&pool)
        .count_filtered(
            source_id,
            country.as_deref(),
            category.as_deref(),
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
    app: AppHandle,
    mode: ImportMode,
) {
    let started_at = Instant::now();
    let mut stats = ImportStats::default();
    let mut errors: Vec<String> = Vec::new();

    // Emit immediately so the UI transitions to the progress screen before
    // the first chunk even arrives.
    emit_import_progress(
        &app,
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
                &app,
                source_id,
                ImportStage::Failed,
                &stats,
                Some(e.to_string()),
                None,
                true,
            );
            return;
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
                    &app,
                    source_id,
                    ImportStage::Failed,
                    &stats,
                    Some(e.to_string()),
                    None,
                    true,
                );
                return;
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
                    &app,
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
                &app,
                source_id,
                ImportStage::Failed,
                &stats,
                Some(e.to_string()),
                None,
                true,
            );
            return;
        }

        // Emit progress every 500 entries for responsive feedback without
        // saturating the frontend event bridge on very large playlists.
        if stats.parsed.is_multiple_of(500) {
            stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
            emit_import_progress(
                &app,
                source_id,
                ImportStage::Receiving,
                &stats,
                None,
                warning_from_errors(&errors),
                false,
            );
        }
    }

    if stats.parsed == 0 {
        let error = if errors.is_empty() {
            "import completed without any playlist entries".to_string()
        } else {
            errors.join("; ")
        };
        stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_import_progress(
            &app,
            source_id,
            ImportStage::Failed,
            &stats,
            Some(error),
            None,
            true,
        );
        return;
    }

    stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
    emit_import_progress(
        &app,
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
            &app,
            source_id,
            ImportStage::Failed,
            &stats,
            Some(e.to_string()),
            None,
            true,
        );
        return;
    }

    // Refresh: re-apply the snapshotted user state onto the new rows.
    if let Some(snap) = replace_snapshot {
        if let Err(e) = restore_user_state(&mut tx, source_id, &snap).await {
            stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
            emit_import_progress(
                &app,
                source_id,
                ImportStage::Failed,
                &stats,
                Some(e.to_string()),
                None,
                true,
            );
            return;
        }
    }

    if let Err(e) = SourceRepository::touch_refreshed_tx(&mut tx, source_id).await {
        stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_import_progress(
            &app,
            source_id,
            ImportStage::Failed,
            &stats,
            Some(e.to_string()),
            None,
            true,
        );
        return;
    }

    let commit_started = Instant::now();
    if let Err(e) = tx.commit().await {
        stats.elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_import_progress(
            &app,
            source_id,
            ImportStage::Failed,
            &stats,
            Some(e.to_string()),
            None,
            true,
        );
        return;
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
        &app,
        source_id,
        ImportStage::Complete,
        &stats,
        None,
        warning_from_errors(&errors),
        true,
    );
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
    app: &AppHandle,
    source_id: i64,
    stage: ImportStage,
    stats: &ImportStats,
    error: Option<String>,
    warning: Option<String>,
    done: bool,
) {
    let _ = app.emit(
        "playlist:import:progress",
        ImportProgress {
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
        },
    );
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
        MediaKind::Live => live.push(Channel {
            id: 0,
            source_id,
            tvg_id: e.tvg_id.clone(),
            name: nova_core::polish_title(&e.clean_title),
            country: e.country_prefix.clone(),
            category: e.category.clone(),
            raw_group: Some(e.group_title.clone()),
            logo_url: e.logo_url.clone(),
            stream_url: e.stream_url.clone(),
            is_favorite: false,
            is_in_watchlist: false,
            last_played_at: None,
            added_at: None,
        }),
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
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .join("profiles")
        .join(profile_id);
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
