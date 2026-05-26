//! Phase 5 - Playback commands.
//!
//! - `resolve_stream` is the entry point: given `(media_kind, media_id)` it
//!   looks up the upstream URL in the per-profile DB, asks the HLS proxy to
//!   classify and (if needed) start a remuxer, and returns the URL + kind
//!   the player should consume.
//! - `stop_playback` releases the proxy session when the player closes -
//!   killing any ffmpeg child and removing its temp dir.
//! - `record_watch_progress` is called periodically while the player is open
//!   so the Continue Watching shelf and the player's resume hint stay current.
//!   `caption_mode` is the user's caption selection (always 'off' in Phase 5).
//! - `get_resume_position` returns the last position for a media item, used
//!   when opening the player to seek to where the user left off.
//! - `get_proxy_port` is kept for compatibility while the frontend transitions
//!   to `resolve_stream`; will be removed once nothing references it.

use nova_hls_proxy::{PlaybackKind, PlaybackRoute, SessionInfo};
use nova_storage::{PlaybackRepository, ResumePosition, WatchHistoryRepository};
use serde::Serialize;
use tauri::State;

use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

#[tauri::command]
pub fn get_proxy_port(state: State<'_, AppState>) -> u16 {
    state.hls_proxy_port
}

/// Response from [`resolve_stream`]. `kind` tells the player whether to
/// attach hls.js (`"hls"`) or use the native `<video>` element (`"file"`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedStream {
    pub url: String,
    pub kind: PlaybackKind,
    pub route: PlaybackRoute,
    pub token: String,
    pub supports_seek: bool,
    pub duration_sec: Option<f64>,
    pub source_lang: Option<String>,
    pub container: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub fallback_reason: Option<String>,
    pub caption_state: CaptionState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionState {
    pub source: CaptionAssetState,
    pub translation: CaptionAssetState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionAssetState {
    Unavailable,
    Pending,
    Ready,
}

impl CaptionState {
    fn unavailable() -> Self {
        Self {
            source: CaptionAssetState::Unavailable,
            translation: CaptionAssetState::Unavailable,
        }
    }
}

#[tauri::command]
pub async fn resolve_stream(
    profile_id: String,
    media_kind: String, // 'live' | 'vod' | 'episode'
    media_id: i64,
    route_override: Option<PlaybackRoute>,
    state: State<'_, AppState>,
) -> Result<ResolvedStream, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let resolved = PlaybackRepository::new(&pool)
        .resolve_stream_url(&media_kind, media_id)
        .await?;

    let is_live = media_kind == "live";
    let SessionInfo {
        token,
        kind,
        route,
        supports_seek,
        duration_sec,
        container,
        video_codec,
        audio_codec,
        width,
        height,
        fallback_reason,
    } = state
        .hls_sessions
        .register(resolved.upstream_url.clone(), is_live, route_override)
        .await
        .map_err(|e| ApiError::Internal(format!("playback session failed: {e}")))?;

    let url = match kind {
        PlaybackKind::Hls => format!(
            "http://127.0.0.1:{}/stream/{}/master.m3u8",
            state.hls_proxy_port, token,
        ),
        PlaybackKind::File => format!(
            "http://127.0.0.1:{}/proxy?url={}",
            state.hls_proxy_port,
            urlencode(&resolved.upstream_url),
        ),
    };

    Ok(ResolvedStream {
        url,
        kind,
        route,
        token,
        supports_seek,
        duration_sec,
        source_lang: resolved.source_lang,
        container,
        video_codec,
        audio_codec,
        width,
        height,
        fallback_reason,
        caption_state: CaptionState::unavailable(),
    })
}

#[tauri::command]
pub async fn stop_playback(token: String, state: State<'_, AppState>) -> Result<(), ApiError> {
    state.hls_sessions.release(&token).await;
    Ok(())
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
#[allow(clippy::too_many_arguments)] // Tauri commands take flat args by design.
pub async fn record_watch_progress(
    profile_id: String,
    media_kind: String, // 'vod' | 'episode' | 'live'
    media_id: i64,
    position_sec: f64,
    duration_sec: Option<f64>,
    completed: bool,
    caption_mode: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    WatchHistoryRepository::new(&pool)
        .record_progress(
            &media_kind,
            media_id,
            position_sec,
            duration_sec,
            completed,
            caption_mode.as_deref(),
        )
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

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
