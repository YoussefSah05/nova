//! Phase 5 — Playback commands.
//!
//! - `get_proxy_port` lets the frontend build proxied stream URLs.
//! - `record_watch_progress` is called periodically while the player is open
//!   so the Continue Watching shelf and the player's resume hint stay current.
//! - `get_resume_position` returns the last position for a media item, used
//!   when opening the player to seek to where the user left off.

use nova_storage::{ResumePosition, WatchHistoryRepository};
use serde::Serialize;
use tauri::State;

use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

#[tauri::command]
pub fn get_proxy_port(state: State<'_, AppState>) -> u16 {
    state.hls_proxy_port
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeInfo {
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    pub completed: bool,
}

impl From<ResumePosition> for ResumeInfo {
    fn from(r: ResumePosition) -> Self {
        Self {
            position_sec: r.position_sec,
            duration_sec: r.duration_sec,
            completed: r.completed,
        }
    }
}

#[tauri::command]
pub async fn record_watch_progress(
    profile_id: String,
    media_kind: String, // 'vod' | 'episode' | 'live'
    media_id: i64,
    position_sec: f64,
    duration_sec: Option<f64>,
    completed: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    WatchHistoryRepository::new(&pool)
        .record_progress(&media_kind, media_id, position_sec, duration_sec, completed)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_resume_position(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    state: State<'_, AppState>,
) -> Result<Option<ResumeInfo>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let info = WatchHistoryRepository::new(&pool)
        .resume_position(&media_kind, media_id)
        .await
        .map_err(ApiError::from)?
        .map(ResumeInfo::from);
    Ok(info)
}
