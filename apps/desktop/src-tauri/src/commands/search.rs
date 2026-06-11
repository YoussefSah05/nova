//! Phase 8 — transcript read + dialogue search commands.
//!
//! Both resolve the persisted transcript for the open title the same way the
//! caption/translation sessions do (stable `content_key` + audio-track slot),
//! then read its lines or run an FTS5 search scoped to it. Empty results when
//! nothing has been transcribed yet — the panel shows a friendly empty state.

use nova_storage::{PlaybackRepository, SearchRepository, TranscriptRepository};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::State;

use crate::caption_session::content_key;
use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

/// One transcript line for the companion panel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptLineDto {
    pub segment_id: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
    pub translation: Option<String>,
}

/// One dialogue-search match.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHitDto {
    pub segment_id: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
    /// `source` | `translation`
    pub kind: String,
}

/// Resolve the persisted transcript id for a media item + audio track, or `None`
/// when nothing has been transcribed yet.
async fn resolve_transcript_id(
    pool: &SqlitePool,
    media_kind: &str,
    media_id: i64,
    audio_track: i64,
) -> Result<Option<i64>, ApiError> {
    let resolved = PlaybackRepository::new(pool)
        .resolve_stream_url(media_kind, media_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let key = content_key(&resolved.upstream_url);
    let stored = TranscriptRepository::new(pool)
        .get_for_media_track(&key, media_kind, media_id, audio_track)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(stored.map(|t| t.id))
}

#[tauri::command]
pub async fn get_transcript(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    audio_track_index: Option<u32>,
    target_lang: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<TranscriptLineDto>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let track = audio_track_index.unwrap_or(0) as i64;
    let Some(transcript_id) = resolve_transcript_id(&pool, &media_kind, media_id, track).await?
    else {
        return Ok(Vec::new());
    };
    let lines = TranscriptRepository::new(&pool)
        .transcript_with_translations(
            transcript_id,
            target_lang.as_deref().unwrap_or(""),
            nova_translate::model::MODEL_ID,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(lines
        .into_iter()
        .map(|l| TranscriptLineDto {
            segment_id: l.segment_id,
            start_sec: l.start_sec,
            end_sec: l.end_sec,
            text: l.text,
            translation: l.translation,
        })
        .collect())
}

#[tauri::command]
pub async fn search_transcript(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    audio_track_index: Option<u32>,
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<SearchHitDto>, ApiError> {
    let match_expr = nova_search::fts_match(&query);
    if match_expr.is_empty() {
        return Ok(Vec::new());
    }
    let pool = require_profile_pool(&profile_id, &state).await?;
    let track = audio_track_index.unwrap_or(0) as i64;
    let Some(transcript_id) = resolve_transcript_id(&pool, &media_kind, media_id, track).await?
    else {
        return Ok(Vec::new());
    };
    let hits = SearchRepository::new(&pool)
        .search_in_transcript(transcript_id, &match_expr, 200)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(hits
        .into_iter()
        .map(|h| SearchHitDto {
            segment_id: h.segment_id,
            start_sec: h.start_sec,
            end_sec: h.end_sec,
            text: h.text,
            kind: h.kind,
        })
        .collect())
}
