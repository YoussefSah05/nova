//! Phase 4 — EPG (Electronic Program Guide) commands.
//!
//! Sourcing: provider feeds first (Xtream `xmltv.php`, M3U `url-tvg`) are
//! auto-registered during `add_source`; community fallbacks (iptv-org, epg.pw,
//! …) get a manual `add_epg_url` entry point so users can paste any XMLTV URL.
//!
//! Refresh: a manual `epg_refresh` command per source. The refresh worker
//! streams the XMLTV body, parses programmes via `nova_epg::parse`, replaces
//! all programmes for the touched channels, and prunes anything older than
//! one day. Progress is emitted on the `epg:refresh:progress` event channel.

use std::sync::Arc;
use std::time::Duration;

use nova_core::now_ts;
use nova_epg::Program;
use nova_storage::{EpgSourcesRepository, NowAndNext, ProgramsRepository};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, State};

use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

const REFRESH_BATCH: usize = 500;

// ── Event payload ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpgRefreshProgress {
    pub source_id: i64,
    pub epg_source_id: i64,
    pub stage: EpgRefreshStage,
    pub inserted: u64,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EpgRefreshStage {
    Downloading,
    Parsing,
    Complete,
    Failed,
}

fn emit(app: &AppHandle, payload: EpgRefreshProgress) {
    let _ = app.emit("epg:refresh:progress", payload);
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// Now-and-next for a list of channel TVG IDs. Returns one entry per ID —
/// entries without EPG data still come back with both fields `None` so the
/// frontend can render a stable list.
#[tauri::command]
pub async fn epg_now_and_next(
    profile_id: String,
    tvg_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<NowAndNext>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let now = now_ts();
    ProgramsRepository::new(&pool)
        .now_and_next_batch(&tvg_ids, now)
        .await
        .map_err(Into::into)
}

/// All programmes for a list of channel TVG IDs whose [start, stop) interval
/// overlaps `[from_utc, to_utc)`. Used by the full grid guide view.
#[tauri::command]
pub async fn epg_range(
    profile_id: String,
    tvg_ids: Vec<String>,
    from_utc: i64,
    to_utc: i64,
    state: State<'_, AppState>,
) -> Result<Vec<Program>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ProgramsRepository::new(&pool)
        .range(&tvg_ids, from_utc, to_utc)
        .await
        .map_err(Into::into)
}

/// Manually register an XMLTV URL for a source. Use this to wire up community
/// EPG feeds (iptv-org, epg.pw, EPGTalk, etc.) for sources that don't ship
/// their own EPG, or to supplement provider EPG with extra channel coverage.
#[tauri::command]
pub async fn add_epg_url(
    profile_id: String,
    source_id: i64,
    url: String,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    EpgSourcesRepository::new(&pool)
        .upsert(source_id, url.trim(), "community")
        .await
        .map_err(Into::into)
}

/// Trigger an EPG refresh for every registered XMLTV URL for the source.
/// Runs in a spawned task and emits progress events; returns immediately so
/// the UI doesn't block on a multi-minute download.
#[tauri::command]
pub async fn epg_refresh(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    state
        .jobs_for_profile(&profile_id, pool, app)
        .await
        .epg_refresh(source_id)
        .await
        .map_err(ApiError::Internal)
}

// ── Refresh worker ────────────────────────────────────────────────────────────

pub(crate) async fn run_epg_refresh_job(
    source_id: i64,
    pool: Arc<SqlitePool>,
    app: AppHandle,
    cancellation: &crate::model_runtime::RuntimeCancellation,
) -> Result<(), String> {
    let epg_sources = EpgSourcesRepository::new(&pool)
        .list_for_source(source_id)
        .await
        .map_err(|_| "could not list EPG sources".to_string())?;
    if epg_sources.is_empty() {
        return Ok(());
    }
    let mut complete = true;
    for es in epg_sources {
        // Each feed commits on its own; stop between feeds once cancelled.
        if cancellation.is_cancelled() {
            return Err("EPG refresh cancelled".into());
        }
        complete &= run_refresh(source_id, es.id, &es.url, &pool, app.clone()).await;
    }
    // Keep one day of history after all source transactions complete.
    let cutoff = now_ts() - 86_400;
    ProgramsRepository::new(&pool)
        .prune_before(cutoff)
        .await
        .map_err(|_| "EPG prune failed".to_string())?;
    if complete {
        Ok(())
    } else {
        Err("EPG refresh failed".into())
    }
}

async fn run_refresh(
    source_id: i64,
    epg_source_id: i64,
    url: &str,
    pool: &Arc<SqlitePool>,
    app: AppHandle,
) -> bool {
    let started_at = std::time::Instant::now();
    emit(
        &app,
        EpgRefreshProgress {
            source_id,
            epg_source_id,
            stage: EpgRefreshStage::Downloading,
            inserted: 0,
            elapsed_ms: 0,
            error: None,
        },
    );

    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .user_agent("Mozilla/5.0 (compatible; nova-iptv/1.0)")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return fail(
                &app,
                source_id,
                epg_source_id,
                started_at,
                e.to_string(),
                pool,
            )
            .await
        }
    };

    let bytes = match client.get(url).send().await {
        Ok(r) if r.status().is_success() => match r.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return fail(
                    &app,
                    source_id,
                    epg_source_id,
                    started_at,
                    e.without_url().to_string(),
                    pool,
                )
                .await
            }
        },
        Ok(r) => {
            return fail(
                &app,
                source_id,
                epg_source_id,
                started_at,
                format!("HTTP {}", r.status()),
                pool,
            )
            .await;
        }
        Err(e) => {
            return fail(
                &app,
                source_id,
                epg_source_id,
                started_at,
                e.without_url().to_string(),
                pool,
            )
            .await
        }
    };

    emit(
        &app,
        EpgRefreshProgress {
            source_id,
            epg_source_id,
            stage: EpgRefreshStage::Parsing,
            inserted: 0,
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            error: None,
        },
    );

    // quick-xml is sync — run the streaming parser on a blocking thread.
    let parse_result = tokio::task::spawn_blocking(move || -> Result<Vec<Program>, String> {
        let cursor = std::io::Cursor::new(bytes);
        let reader = std::io::BufReader::new(cursor);
        let mut programs = Vec::new();
        for p in nova_epg::parse(reader) {
            match p {
                Ok(p) => programs.push(p),
                Err(e) => {
                    // Soft-fail individual programmes — broken element shouldn't
                    // discard the entire feed.
                    tracing::warn!(error = %e, "EPG parse: skipped malformed programme");
                }
            }
        }
        Ok(programs)
    })
    .await;

    let programs = match parse_result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return fail(&app, source_id, epg_source_id, started_at, e, pool).await,
        Err(e) => {
            return fail(
                &app,
                source_id,
                epg_source_id,
                started_at,
                e.to_string(),
                pool,
            )
            .await
        }
    };

    // Collect the unique channel IDs we just got data for and clear them
    // before re-inserting. Keeps the table in sync with upstream rather than
    // accumulating duplicates run after run.
    let touched_channels: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for p in &programs {
            if seen.insert(p.channel_tvg_id.clone()) {
                out.push(p.channel_tvg_id.clone());
            }
        }
        out
    };

    let insert_result: Result<u64, String> = async {
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        nova_storage::ProgramsRepository::delete_for_channels_tx(&mut tx, &touched_channels)
            .await
            .map_err(|e| e.to_string())?;
        let mut total: u64 = 0;
        for chunk in programs.chunks(REFRESH_BATCH) {
            total += nova_storage::ProgramsRepository::insert_batch_tx(&mut tx, chunk)
                .await
                .map_err(|e| e.to_string())?;
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(total)
    }
    .await;

    match insert_result {
        Ok(inserted) => {
            if let Err(e) = EpgSourcesRepository::new(pool)
                .touch_success(epg_source_id)
                .await
            {
                tracing::warn!(error = %e, "epg_refresh: touch_success failed");
            }
            emit(
                &app,
                EpgRefreshProgress {
                    source_id,
                    epg_source_id,
                    stage: EpgRefreshStage::Complete,
                    inserted,
                    elapsed_ms: started_at.elapsed().as_millis() as u64,
                    error: None,
                },
            );
            tracing::info!(
                source_id,
                epg_source_id,
                inserted,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "EPG refresh complete"
            );
            true
        }
        Err(e) => fail(&app, source_id, epg_source_id, started_at, e, pool).await,
    }
}

async fn fail(
    app: &AppHandle,
    source_id: i64,
    epg_source_id: i64,
    started_at: std::time::Instant,
    error: String,
    pool: &Arc<SqlitePool>,
) -> bool {
    tracing::warn!(source_id, epg_source_id, error = %error, "EPG refresh failed");
    if let Err(e) = EpgSourcesRepository::new(pool)
        .touch_failure(epg_source_id, &error)
        .await
    {
        tracing::warn!(error = %e, "epg_refresh: touch_failure failed");
    }
    emit(
        app,
        EpgRefreshProgress {
            source_id,
            epg_source_id,
            stage: EpgRefreshStage::Failed,
            inserted: 0,
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            error: Some(error),
        },
    );
    false
}
