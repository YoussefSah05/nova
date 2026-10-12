//! Phase 6 — VOD transcription commands.
//!
//! Captions stream in via a [`CaptionSessionManager`](crate::caption_session):
//! `start_caption_session` begins transcribing ~30s windows at the playhead and
//! emits `transcription:cues` / `transcription:status`; `set_caption_playhead`
//! keeps it anchored to playback; `stop_caption_session` cancels it. The model
//! is fetched once via `download_whisper_model` (the UI prompts first).
//!
//! Cues render through the React `CaptionOverlay`, not mpv's native renderer.

use std::path::PathBuf;

use nova_transcribe::{default_model, ensure_model, ModelDownloadProgress};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::caption_session::StartCtx;
use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelDownloadEvent {
    pub(crate) artifact: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub(crate) downloaded_bytes: u64,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub(crate) total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub installed: bool,
    pub model_id: String,
    pub availability: String,
}

// ── Model commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_model_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ModelStatus, ApiError> {
    let model = default_model();
    let observed = super::models::observe_internal_model(&app, &state.pool, model.id).await?;
    let available = state
        .model_runtime
        .is_available(&app_data_dir(&app)?, model.id)
        .await
        .map_err(super::models::runtime_api_error)?;
    Ok(ModelStatus {
        installed: available,
        model_id: observed.descriptor.id.as_str().to_string(),
        availability: if available {
            "available"
        } else {
            observed.availability.as_str()
        }
        .to_string(),
    })
}

#[tauri::command]
pub async fn download_whisper_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let models_dir = whisper_models_dir(&app)?;
    let model = default_model();
    let app_for_progress = app.clone();
    state
        .model_runtime
        .mutate(model.id, async move {
            ensure_model(&models_dir, &model, move |p: ModelDownloadProgress| {
                let _ = app_for_progress.emit(
                    "model:download:progress",
                    ModelDownloadEvent {
                        artifact: p.artifact.to_string(),
                        downloaded_bytes: p.downloaded_bytes,
                        total_bytes: p.total_bytes,
                    },
                );
            })
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))
        })
        .await
        .map_err(super::models::runtime_api_error)??;
    Ok(())
}

// ── Streaming caption session commands ──────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn start_caption_session(
    session_id: String,
    profile_id: String,
    media_kind: String,
    media_id: i64,
    from_sec: f64,
    duration_sec: Option<f64>,
    audio_track_index: Option<u32>,
    track_lang: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let ctx = StartCtx {
        session_id,
        profile_id,
        app: app.clone(),
        pool,
        media_kind,
        media_id,
        from_sec,
        duration_sec,
        audio_track: audio_track_index,
        track_lang,
        models_dir: whisper_models_dir(&app)?,
        model_root: app_data_dir(&app)?,
        runtime: state.model_runtime.clone(),
        transcripts_dir: transcripts_dir(&app)?,
    };
    state.caption_sessions.start(ctx).await;
    Ok(())
}

#[tauri::command]
pub async fn set_caption_playhead(sec: f64, state: State<'_, AppState>) -> Result<(), ApiError> {
    state.caption_sessions.set_playhead(sec).await;
    Ok(())
}

#[tauri::command]
pub async fn stop_caption_session(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.caption_sessions.stop().await;
    Ok(())
}

// ── Paths ─────────────────────────────────────────────────────────────────────

pub(crate) fn whisper_models_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    Ok(app_data_dir(app)?.join("models").join("whisper"))
}

pub(crate) fn transcripts_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    Ok(app_data_dir(app)?.join("transcripts"))
}

pub(crate) fn app_data_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    app.path()
        .app_data_dir()
        .map_err(|e| ApiError::Internal(e.to_string()))
}
