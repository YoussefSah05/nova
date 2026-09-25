//! Settings-section commands (cleanup phase): profile editing, source
//! management, whisper-model + transcription-cache maintenance, and app info.

use std::path::Path;

use nova_core::Profile;
use nova_storage::{ProfileRepository, SourceRepository, TranscriptRepository};
use nova_transcribe::{default_model, model_paths};
use serde::Serialize;
use tauri::{AppHandle, State};

use super::require_profile_pool;
use super::transcription::{app_data_dir, transcripts_dir, whisper_models_dir};
use crate::ipc::ApiError;
use crate::state::AppState;

// ── Profile ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn update_profile(
    id: String,
    name: String,
    avatar_color: String,
    state: State<'_, AppState>,
) -> Result<Profile, ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::Conflict("name cannot be empty".into()));
    }
    ProfileRepository::new(&state.pool)
        .update(&id, name.trim(), &avatar_color)
        .await
        .map_err(Into::into)
}

// ── Sources ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn delete_source(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    SourceRepository::new(&pool)
        .delete(source_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_source_auto_refresh(
    profile_id: String,
    source_id: i64,
    hours: i64,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    SourceRepository::new(&pool)
        .set_auto_refresh(source_id, hours.max(0))
        .await
        .map_err(Into::into)
}

// ── Transcription model + cache ────────────────────────────────────────────────

#[tauri::command]
pub async fn remove_whisper_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let dir = whisper_models_dir(&app)?;
    let paths = model_paths(&dir, &default_model());
    state
        .model_runtime
        .mutate(default_model().id, async move {
            for result in [
                tokio::fs::remove_file(&paths.ggml).await,
                tokio::fs::remove_dir_all(&paths.coreml_dir).await,
            ] {
                if let Err(error) = result {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(ApiError::Internal(
                            "could not remove model artifacts".into(),
                        ));
                    }
                }
            }
            Ok(())
        })
        .await
        .map_err(super::models::runtime_api_error)?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub transcripts: i64,
    pub bytes: u64,
}

#[tauri::command]
pub async fn get_transcription_cache_stats(
    profile_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CacheStats, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let transcripts = TranscriptRepository::new(&pool).count().await?;
    let bytes = dir_size(&transcripts_dir(&app)?);
    Ok(CacheStats { transcripts, bytes })
}

#[tauri::command]
pub async fn clear_transcription_cache(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let paths = TranscriptRepository::new(&pool).clear_all().await?;
    for p in paths {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

// ── App info ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub transcripts_bytes: u64,
    pub models_bytes: u64,
}

#[tauri::command]
pub async fn get_app_info(app: AppHandle) -> Result<AppInfo, ApiError> {
    let data = app_data_dir(&app)?;
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        data_dir: data.to_string_lossy().into_owned(),
        transcripts_bytes: dir_size(&transcripts_dir(&app)?),
        models_bytes: dir_size(&whisper_models_dir(&app)?),
    })
}

/// Recursive on-disk size of `dir` in bytes (0 if it doesn't exist).
fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            total += dir_size(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}
