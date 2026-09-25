//! Phase 7 — VOD translation commands.
//!
//! A [`TranslationSessionManager`](crate::translation_session) runs parallel to
//! the caption session: `start_translation_session` translates source segments
//! at the playhead into the chosen reading language and emits `translation:cues`
//! / `translation:status`; `set_translation_playhead` anchors it; and
//! `stop_translation_session` cancels it. The NLLB model is fetched once via
//! `download_nllb_model` (the UI prompts first). Cues render through the same
//! React `CaptionOverlay` as source captions — that's the translation card.

use std::path::PathBuf;

use nova_translate::{model as nllb_model, TARGET_LANGUAGES};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::transcription::app_data_dir;
use crate::ipc::ApiError;
use crate::state::AppState;
use crate::translation_session::StartCtx;

use super::require_profile_pool;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDownloadEvent {
    artifact: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub installed: bool,
    pub model_id: String,
}

/// A language offered in the "Translate to" picker.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetLanguageDto {
    pub label: String,
    pub code: String,
}

// ── Model commands (mirror the whisper model commands) ──────────────────────────

#[tauri::command]
pub async fn get_nllb_model_status(app: AppHandle) -> Result<ModelStatus, ApiError> {
    let models_dir = nllb_models_dir(&app)?;
    Ok(ModelStatus {
        installed: nllb_model::is_installed(&models_dir),
        model_id: nllb_model::MODEL_ID.to_string(),
    })
}

#[tauri::command]
pub async fn download_nllb_model(app: AppHandle) -> Result<(), ApiError> {
    let models_dir = nllb_models_dir(&app)?;
    let app_for_progress = app.clone();
    nllb_model::ensure_model(&models_dir, move |p| {
        let _ = app_for_progress.emit(
            "nllb:download:progress",
            ModelDownloadEvent {
                artifact: p.artifact.to_string(),
                downloaded_bytes: p.downloaded_bytes,
                total_bytes: p.total_bytes,
            },
        );
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn list_target_languages() -> Result<Vec<TargetLanguageDto>, ApiError> {
    Ok(TARGET_LANGUAGES
        .iter()
        .map(|t| TargetLanguageDto {
            label: t.label.to_string(),
            code: t.bcp47.to_string(),
        })
        .collect())
}

// ── Streaming translation session commands ──────────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn start_translation_session(
    session_id: String,
    profile_id: String,
    media_kind: String,
    media_id: i64,
    from_sec: f64,
    duration_sec: Option<f64>,
    audio_track_index: Option<u32>,
    target_lang: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let ctx = StartCtx {
        session_id,
        app: app.clone(),
        pool,
        media_kind,
        media_id,
        from_sec,
        duration_sec,
        audio_track: audio_track_index,
        target_lang,
        models_dir: nllb_models_dir(&app)?,
    };
    state.translation_sessions.start(ctx).await;
    Ok(())
}

#[tauri::command]
pub async fn set_translation_playhead(
    sec: f64,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state.translation_sessions.set_playhead(sec).await;
    Ok(())
}

#[tauri::command]
pub async fn stop_translation_session(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.translation_sessions.stop().await;
    Ok(())
}

// ── Paths ─────────────────────────────────────────────────────────────────────

pub(crate) fn nllb_models_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    Ok(app_data_dir(app)?.join("models").join("nllb"))
}
