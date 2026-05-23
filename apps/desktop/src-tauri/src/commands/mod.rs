use nova_core::{
    Channel, ContentSort, ContinueWatchingItem, ImportProgress, ImportStage, MediaKind,
    ParsedEntry, Profile, SeriesEpisode, SeriesSummary, Source, VodItem,
};
use nova_storage::{
    init_profile_db, ChannelRepository, EpgSourcesRepository, ProfileRepository, SeriesRepository,
    SourceRepository, VodRepository, WatchHistoryRepository,
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::ipc::ApiError;
use crate::state::AppState;

use std::time::{Duration, Instant};

pub mod epg;
pub mod playback;

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

    // Spawn import task — returns immediately, streams progress events
    tokio::spawn(async move {
        run_import(source_id, rx, pool, app).await;
    });

    Ok(source_id)
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

#[tauri::command]
pub async fn list_series_episodes(
    profile_id: String,
    source_id: i64,
    series_title: String,
    state: State<'_, AppState>,
) -> Result<Vec<SeriesEpisode>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    SeriesRepository::new(&pool)
        .list_episodes(source_id, &series_title)
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
            name: e.clean_title.clone(),
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
            clean_title: Some(e.clean_title.clone()),
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
            series_title: e
                .series_title
                .clone()
                .unwrap_or_else(|| e.clean_title.clone()),
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
