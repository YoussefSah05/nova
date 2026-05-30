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
use nova_mpv_player::{MpvOpenOptions, MpvOpenResult, MpvPlayerState};
use nova_storage::{PlaybackRepository, ResumePosition, WatchHistoryRepository};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::airplay::AirPlayStatus;
use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

#[tauri::command]
pub fn get_proxy_port(state: State<'_, AppState>) -> u16 {
    state.hls_proxy_port
}

#[tauri::command]
pub async fn mpv_open_stream(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    title: String,
    state: State<'_, AppState>,
) -> Result<MpvOpenResult, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let resolved = PlaybackRepository::new(&pool)
        .resolve_stream_url(&media_kind, media_id)
        .await?;

    state
        .mpv_engine
        .open(MpvOpenOptions {
            url: resolved.upstream_url,
            title,
            user_agent: Some("Mozilla/5.0 (compatible; nova-iptv/1.0)".to_string()),
        })
        .map_err(|e| ApiError::Internal(format!("native player failed: {e}")))
}

#[tauri::command]
pub async fn mpv_close(token: String, state: State<'_, AppState>) -> Result<(), ApiError> {
    state
        .mpv_engine
        .close(&token)
        .map_err(|e| ApiError::Internal(format!("native player close failed: {e}")))
}

#[tauri::command]
pub async fn mpv_player_state(
    token: Option<String>,
    state: State<'_, AppState>,
) -> Result<MpvPlayerState, ApiError> {
    state
        .mpv_engine
        .state(token.as_deref())
        .map_err(|e| ApiError::Internal(format!("native player state failed: {e}")))
}

#[tauri::command]
pub async fn mpv_toggle_pause(token: String, state: State<'_, AppState>) -> Result<(), ApiError> {
    state
        .mpv_engine
        .toggle_pause(&token)
        .map_err(|e| ApiError::Internal(format!("native player pause failed: {e}")))
}

#[tauri::command]
pub async fn mpv_seek_relative(
    token: String,
    delta_sec: f64,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .mpv_engine
        .seek_relative(&token, delta_sec)
        .map_err(|e| ApiError::Internal(format!("native player seek failed: {e}")))
}

#[tauri::command]
pub async fn mpv_seek_absolute(
    token: String,
    position_sec: f64,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .mpv_engine
        .seek_absolute(&token, position_sec)
        .map_err(|e| ApiError::Internal(format!("native player seek failed: {e}")))
}

#[tauri::command]
pub async fn mpv_seek_live_edge(token: String, state: State<'_, AppState>) -> Result<(), ApiError> {
    state
        .mpv_engine
        .seek_live_edge(&token)
        .map_err(|e| ApiError::Internal(format!("native player live seek failed: {e}")))
}

#[tauri::command]
pub async fn mpv_set_volume(
    token: String,
    volume: f64,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .mpv_engine
        .set_volume(&token, volume)
        .map_err(|e| ApiError::Internal(format!("native player volume failed: {e}")))
}

#[tauri::command]
pub async fn mpv_set_mute(
    token: String,
    muted: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .mpv_engine
        .set_mute(&token, muted)
        .map_err(|e| ApiError::Internal(format!("native player mute failed: {e}")))
}

#[tauri::command]
pub async fn mpv_set_audio_track(
    token: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .mpv_engine
        .set_audio_track(&token, &id)
        .map_err(|e| ApiError::Internal(format!("native player audio track failed: {e}")))
}

#[tauri::command]
pub async fn mpv_set_subtitle_track(
    token: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .mpv_engine
        .set_subtitle_track(&token, &id)
        .map_err(|e| ApiError::Internal(format!("native player subtitle track failed: {e}")))
}

#[tauri::command]
pub async fn mpv_set_video_track(
    token: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .mpv_engine
        .set_video_track(&token, &id)
        .map_err(|e| ApiError::Internal(format!("native player video track failed: {e}")))
}

#[tauri::command]
pub async fn mpv_set_speed(
    token: String,
    speed: f64,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .mpv_engine
        .set_speed(&token, speed)
        .map_err(|e| ApiError::Internal(format!("native player speed failed: {e}")))
}

#[tauri::command]
pub async fn mpv_frame_step(
    token: String,
    forward: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .mpv_engine
        .frame_step(&token, forward)
        .map_err(|e| ApiError::Internal(format!("native player frame step failed: {e}")))
}

#[tauri::command]
pub async fn mpv_request_redraw(state: State<'_, AppState>) -> Result<(), ApiError> {
    state
        .mpv_engine
        .request_redraw()
        .map_err(|e| ApiError::Internal(format!("native player redraw failed: {e}")))
}

#[tauri::command]
pub async fn mpv_set_video_rect(
    rect: Option<nova_mpv_player::VideoRect>,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .mpv_engine
        .set_video_rect(rect)
        .map_err(|e| ApiError::Internal(format!("native player video rect failed: {e}")))
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

// -- AirPlay (parallel AVPlayer route) ---------------------------------------

/// Returned by [`airplay_engage`]. `proxy_token` lets the frontend release the
/// proxy session (via [`airplay_disengage`]) when AirPlay ends.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AirPlayEngageResult {
    pub proxy_token: String,
}

/// Position (or hide, with `None`) the native AirPlay route-picker overlay so
/// it sits over the HTML AirPlay button. Coordinates are CSS px, window-relative.
#[tauri::command]
pub async fn airplay_set_picker_rect(
    rect: Option<nova_mpv_player::VideoRect>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state.airplay.set_picker_rect(&app, rect);
    Ok(())
}

/// Begin AirPlay for the given media at `position_sec`. Resolves the upstream
/// URL, registers a proxy session (so hostile IPTV becomes Apple-TV-friendly
/// HLS), and hands the proxy URL to the AVFoundation engine. The player starts
/// muted; the frontend unmutes (and pauses mpv) once `airplay:status.engaged`
/// confirms the external route activated.
#[tauri::command]
pub async fn airplay_engage(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    position_sec: f64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AirPlayEngageResult, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let resolved = PlaybackRepository::new(&pool)
        .resolve_stream_url(&media_kind, media_id)
        .await?;

    let is_live = media_kind == "live";
    let info = state
        .hls_sessions
        .register(resolved.upstream_url.clone(), is_live, None)
        .await
        .map_err(|e| ApiError::Internal(format!("airplay session failed: {e}")))?;

    let url = match info.kind {
        PlaybackKind::Hls => format!(
            "http://127.0.0.1:{}/stream/{}/master.m3u8",
            state.hls_proxy_port, info.token,
        ),
        PlaybackKind::File => format!(
            "http://127.0.0.1:{}/proxy?url={}",
            state.hls_proxy_port,
            urlencode(&resolved.upstream_url),
        ),
    };

    state.airplay.engage(&app, url, position_sec.max(0.0));
    Ok(AirPlayEngageResult {
        proxy_token: info.token,
    })
}

/// End AirPlay: clear the AVFoundation item and release the proxy session.
#[tauri::command]
pub async fn airplay_disengage(
    proxy_token: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state.airplay.disengage(&app);
    if let Some(token) = proxy_token {
        state.hls_sessions.release(&token).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn airplay_toggle_pause(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state.airplay.toggle_pause(&app);
    Ok(())
}

#[tauri::command]
pub async fn airplay_seek_absolute(
    position_sec: f64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state.airplay.seek_absolute(&app, position_sec);
    Ok(())
}

#[tauri::command]
pub async fn airplay_set_volume(
    volume: f64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state.airplay.set_volume(&app, volume);
    Ok(())
}

#[tauri::command]
pub async fn airplay_set_mute(
    muted: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state.airplay.set_muted(&app, muted);
    Ok(())
}

#[tauri::command]
pub async fn airplay_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AirPlayStatus, ApiError> {
    Ok(state.airplay.status(&app))
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
