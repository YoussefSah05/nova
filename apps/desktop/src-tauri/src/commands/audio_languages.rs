//! Detail-page audio tags for one playable version; provider URLs stay in Rust.

use std::time::Duration;

use nova_storage::PlaybackRepository;
use serde::Serialize;
use tauri::State;

use crate::ipc::ApiError;
use crate::model_runtime::RuntimeCancellation;
use crate::resource_policy::WorkClass;
use crate::state::AppState;

use super::require_profile_pool;

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct AudioLanguages {
    pub languages: Vec<String>,
    pub unknown_tracks: u32,
    pub track_count: u32,
    pub tracks: Vec<AudioTrackInfo>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct AudioTrackInfo {
    pub audio_index: u32,
    pub language: Option<String>,
}

#[tauri::command]
pub async fn get_audio_languages(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    state: State<'_, AppState>,
) -> Result<AudioLanguages, ApiError> {
    if !matches!(media_kind.as_str(), "vod" | "episode" | "live") || media_id <= 0 {
        return Err(ApiError::InvalidInput(
            "invalid audio metadata target".into(),
        ));
    }
    let pool = require_profile_pool(&profile_id, &state).await?;
    let resolved = PlaybackRepository::new(&pool)
        .resolve_stream_url(&media_kind, media_id)
        .await?;
    let admission = state.model_runtime.resource_admission();
    let cancellation = RuntimeCancellation::default();
    // Details render immediately. This interactive request shares bounded job
    // capacity, and its queue wait plus network probe cannot run indefinitely.
    let info = tokio::time::timeout(Duration::from_secs(8), async {
        let _slot = admission
            .acquire_job_slot(WorkClass::Interactive, &cancellation)
            .await
            .map_err(|_| ApiError::Internal("audio metadata cancelled".into()))?;
        nova_media::probe::probe_audio_languages(&resolved.upstream_url, Duration::from_secs(6))
            .await
            .map_err(|_| ApiError::Internal("audio languages could not be checked".into()))
    })
    .await
    .map_err(|_| ApiError::Internal("audio language check timed out".into()))??;
    Ok(AudioLanguages {
        languages: info.languages,
        unknown_tracks: info.unknown_tracks,
        track_count: info.track_count,
        tracks: info
            .tracks
            .into_iter()
            .map(|track| AudioTrackInfo {
                audio_index: track.audio_index,
                language: track.language,
            })
            .collect(),
    })
}
