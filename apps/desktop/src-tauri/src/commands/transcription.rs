//! Phase 6 — VOD transcription commands.
//!
//! `enqueue_transcription` is the entry point: it no-ops if a transcript
//! already exists, otherwise enqueues a `transcribe_vod` job on the profile's
//! [`JobQueue`] and kicks it. The job handler ([`run_transcription`]) resolves
//! the upstream URL, extracts 16 kHz mono audio, ensures the Whisper model is
//! present, runs `whisper-cli`, writes a WebVTT, and persists the transcript.
//!
//! Captions reach the player via mpv `sub-add` of the on-disk `.vtt` (Phase 5
//! libmpv deviation) — not the proxy `/captions` endpoint.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nova_jobs::{JobFuture, JobHandler, JobQueue};
use nova_media::extract_audio_16k_mono;
use nova_storage::{Job, JobRepository, PlaybackRepository, TranscriptRepository};
use nova_transcribe::{
    default_model, ensure_model, is_installed, segments_to_vtt, transcribe_wav,
    ModelDownloadProgress, TranscribeOptions,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

/// Generous bound on audio extraction — a feature-length VOD takes a while to
/// demux. Only there to release a hung upstream, not to cap legitimate runs.
const EXTRACT_DEADLINE: Duration = Duration::from_secs(45 * 60);

// ── Event payloads ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptionProgress {
    media_kind: String,
    media_id: i64,
    /// `extracting` | `transcribing` | `persisting` | `complete` | `failed`
    stage: String,
    /// 0..=100, best-effort.
    pct: u8,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptionComplete {
    media_kind: String,
    media_id: i64,
    language: String,
    vtt_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDownloadEvent {
    artifact: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

// ── Job payload ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscribePayload {
    media_kind: String,
    media_id: i64,
}

// ── Command responses ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueResult {
    /// `None` when a transcript already exists (nothing was enqueued).
    pub job_id: Option<i64>,
    pub already_done: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptStatus {
    /// `none` | `queued` | `running` | `done` | `failed`
    pub state: String,
    pub vtt_path: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub installed: bool,
    pub model_id: String,
}

/// A caption cue for the frontend `CaptionOverlay` (`{ start, end, text }`).
#[derive(Debug, Clone, Serialize)]
pub struct CaptionCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptCues {
    pub language: String,
    pub cues: Vec<CaptionCue>,
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn enqueue_transcription(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<EnqueueResult, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;

    // Already transcribed → nothing to do.
    if TranscriptRepository::new(&pool)
        .get_for_media(&media_kind, media_id)
        .await?
        .is_some()
    {
        return Ok(EnqueueResult {
            job_id: None,
            already_done: true,
        });
    }

    // An active job for this media → return it rather than duplicating.
    if let Some(id) = active_job_for(&pool, &media_kind, media_id).await? {
        return Ok(EnqueueResult {
            job_id: Some(id),
            already_done: false,
        });
    }

    let queue = get_or_create_queue(&state, &app, &profile_id, pool).await?;
    let payload = serde_json::to_string(&TranscribePayload {
        media_kind,
        media_id,
    })
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let job_id = queue
        .enqueue(&payload, 0)
        .await
        .map_err(ApiError::Internal)?;
    queue.kick();

    let _ = app.emit("jobs:queue:changed", ());
    Ok(EnqueueResult {
        job_id: Some(job_id),
        already_done: false,
    })
}

#[tauri::command]
pub async fn get_transcript_status(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    state: State<'_, AppState>,
) -> Result<TranscriptStatus, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;

    if let Some(t) = TranscriptRepository::new(&pool)
        .get_for_media(&media_kind, media_id)
        .await?
    {
        return Ok(TranscriptStatus {
            state: "done".into(),
            vtt_path: Some(t.vtt_path),
            language: Some(t.language),
        });
    }

    // Fall back to the job table for in-flight / failed work.
    let job_state = job_state_for(&pool, &media_kind, media_id).await?;
    Ok(TranscriptStatus {
        state: job_state.unwrap_or_else(|| "none".into()),
        vtt_path: None,
        language: None,
    })
}

/// Return the transcript's cues for rendering in the React caption overlay.
/// Cues are the stored segments (start/end/text); the overlay — not mpv — owns
/// rendering so the Phase 7 translation card can share the same pipeline.
#[tauri::command]
pub async fn get_transcript_cues(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    state: State<'_, AppState>,
) -> Result<TranscriptCues, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let repo = TranscriptRepository::new(&pool);
    let transcript = repo
        .get_for_media(&media_kind, media_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("no transcript for media".into()))?;
    let segments = repo.get_segments(transcript.id).await?;
    Ok(TranscriptCues {
        language: transcript.language,
        cues: segments
            .into_iter()
            .map(|s| CaptionCue {
                start: s.start_sec,
                end: s.end_sec,
                text: s.text,
            })
            .collect(),
    })
}

#[tauri::command]
pub async fn get_model_status(app: AppHandle) -> Result<ModelStatus, ApiError> {
    let models_dir = whisper_models_dir(&app)?;
    let model = default_model();
    Ok(ModelStatus {
        installed: is_installed(&models_dir, &model),
        model_id: model.id.to_string(),
    })
}

#[tauri::command]
pub async fn download_whisper_model(app: AppHandle) -> Result<(), ApiError> {
    let models_dir = whisper_models_dir(&app)?;
    let model = default_model();
    let app_for_progress = app.clone();
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
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

// ── Queue + handler wiring ────────────────────────────────────────────────────

/// Get the profile's transcription queue, building it (and its handler) on
/// first use. Sharing the queue keeps the per-profile concurrency cap of one.
async fn get_or_create_queue(
    state: &AppState,
    app: &AppHandle,
    profile_id: &str,
    pool: Arc<SqlitePool>,
) -> Result<Arc<JobQueue>, ApiError> {
    if let Some(q) = state.transcription_queues.read().await.get(profile_id) {
        return Ok(Arc::clone(q));
    }
    let handler: Arc<dyn JobHandler> = Arc::new(TranscribeHandler {
        app: app.clone(),
        pool: Arc::clone(&pool),
        models_dir: whisper_models_dir(app)?,
        transcripts_dir: transcripts_dir(app)?,
    });
    let queue = Arc::new(JobQueue::new(pool, "transcribe_vod", handler));
    state
        .transcription_queues
        .write()
        .await
        .insert(profile_id.to_string(), Arc::clone(&queue));
    Ok(queue)
}

struct TranscribeHandler {
    app: AppHandle,
    pool: Arc<SqlitePool>,
    models_dir: PathBuf,
    transcripts_dir: PathBuf,
}

impl JobHandler for TranscribeHandler {
    fn handle(&self, job: Job) -> JobFuture {
        let app = self.app.clone();
        let pool = Arc::clone(&self.pool);
        let models_dir = self.models_dir.clone();
        let transcripts_dir = self.transcripts_dir.clone();
        Box::pin(async move {
            let payload: TranscribePayload =
                serde_json::from_str(&job.payload).map_err(|e| e.to_string())?;
            run_transcription(&app, &pool, &models_dir, &transcripts_dir, &payload).await
        })
    }
}

/// The full transcription pipeline for one media item. Emits progress events
/// throughout; on error, emits a `failed` progress event before returning the
/// message (the queue records it in `jobs.last_error`).
async fn run_transcription(
    app: &AppHandle,
    pool: &SqlitePool,
    models_dir: &Path,
    transcripts_dir: &Path,
    payload: &TranscribePayload,
) -> Result<(), String> {
    let result = run_transcription_inner(app, pool, models_dir, transcripts_dir, payload).await;
    if let Err(msg) = &result {
        emit_progress(app, payload, "failed", 0, Some(msg.clone()));
    }
    result
}

async fn run_transcription_inner(
    app: &AppHandle,
    pool: &SqlitePool,
    models_dir: &Path,
    transcripts_dir: &Path,
    payload: &TranscribePayload,
) -> Result<(), String> {
    let media_kind = payload.media_kind.as_str();
    let media_id = payload.media_id;

    // 1. Resolve the upstream URL (and any known source language).
    emit_progress(app, payload, "extracting", 0, None);
    let resolved = PlaybackRepository::new(pool)
        .resolve_stream_url(media_kind, media_id)
        .await
        .map_err(|e| format!("resolve stream: {e}"))?;

    // 2. Extract 16 kHz mono audio to a temp WAV.
    let work = tempfile::Builder::new()
        .prefix("nova-asr-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let wav = work.path().join("audio.wav");
    extract_audio_16k_mono(&resolved.upstream_url, &wav, EXTRACT_DEADLINE)
        .await
        .map_err(|e| format!("audio extraction: {e}"))?;

    // 3. Ensure the model is present (downloads on first ever run).
    let model = default_model();
    let app_for_model = app.clone();
    let paths = ensure_model(models_dir, &model, move |p: ModelDownloadProgress| {
        let _ = app_for_model.emit(
            "model:download:progress",
            ModelDownloadEvent {
                artifact: p.artifact.to_string(),
                downloaded_bytes: p.downloaded_bytes,
                total_bytes: p.total_bytes,
            },
        );
    })
    .await
    .map_err(|e| format!("model: {e}"))?;

    // 4. Run whisper-cli, streaming progress.
    emit_progress(app, payload, "transcribing", 0, None);
    let out_base = work.path().join("transcript");
    let opts = TranscribeOptions {
        language: resolved.source_lang.clone(),
        model_id: model.id.to_string(),
    };
    let app_for_pct = app.clone();
    let payload_for_pct = payload.clone();
    let raw = transcribe_wav(&wav, &paths.ggml, &out_base, &opts, move |pct| {
        emit_progress(&app_for_pct, &payload_for_pct, "transcribing", pct, None);
    })
    .await
    .map_err(|e| format!("whisper: {e}"))?;

    // 5. Render VTT, persist transcript + segments, keep the raw JSON.
    emit_progress(app, payload, "persisting", 100, None);
    std::fs::create_dir_all(transcripts_dir).map_err(|e| e.to_string())?;
    let base = format!("{media_kind}-{media_id}-{}", raw.language);
    let vtt_path = transcripts_dir.join(format!("{base}.vtt"));
    let json_path = transcripts_dir.join(format!("{base}.json"));
    std::fs::write(&vtt_path, segments_to_vtt(&raw.segments)).map_err(|e| e.to_string())?;
    let _ = std::fs::copy(out_base.with_extension("json"), &json_path);

    let vtt_path_str = vtt_path.to_string_lossy().to_string();
    let json_path_str = json_path.to_string_lossy().to_string();
    TranscriptRepository::new(pool)
        .replace_transcript(
            media_kind,
            media_id,
            &raw.language,
            &raw.model,
            &vtt_path_str,
            &json_path_str,
            raw.duration_processed_sec,
            &raw.segments,
        )
        .await
        .map_err(|e| format!("persist transcript: {e}"))?;

    // 6. Done.
    emit_progress(app, payload, "complete", 100, None);
    let _ = app.emit(
        "transcription:complete",
        TranscriptionComplete {
            media_kind: media_kind.to_string(),
            media_id,
            language: raw.language,
            vtt_path: vtt_path_str,
        },
    );
    let _ = app.emit("jobs:queue:changed", ());
    Ok(())
}

fn emit_progress(
    app: &AppHandle,
    payload: &TranscribePayload,
    stage: &str,
    pct: u8,
    error: Option<String>,
) {
    let _ = app.emit(
        "transcription:progress",
        TranscriptionProgress {
            media_kind: payload.media_kind.clone(),
            media_id: payload.media_id,
            stage: stage.to_string(),
            pct,
            error,
        },
    );
}

// ── Job lookup helpers ────────────────────────────────────────────────────────

/// Job id of an active (`queued`/`running`) transcription for this media.
async fn active_job_for(
    pool: &SqlitePool,
    media_kind: &str,
    media_id: i64,
) -> Result<Option<i64>, ApiError> {
    for state in ["running", "queued"] {
        if let Some(job) = find_job(pool, state, media_kind, media_id).await? {
            return Ok(Some(job));
        }
    }
    Ok(None)
}

/// Best label for the job table state of this media's transcription.
async fn job_state_for(
    pool: &SqlitePool,
    media_kind: &str,
    media_id: i64,
) -> Result<Option<String>, ApiError> {
    for state in ["running", "queued", "failed"] {
        if find_job(pool, state, media_kind, media_id).await?.is_some() {
            return Ok(Some(state.to_string()));
        }
    }
    Ok(None)
}

async fn find_job(
    pool: &SqlitePool,
    state: &str,
    media_kind: &str,
    media_id: i64,
) -> Result<Option<i64>, ApiError> {
    let jobs = JobRepository::new(pool)
        .list_by_state(state)
        .await
        .map_err(ApiError::from)?;
    for job in jobs {
        if job.kind != "transcribe_vod" {
            continue;
        }
        if let Ok(p) = serde_json::from_str::<TranscribePayload>(&job.payload) {
            if p.media_kind == media_kind && p.media_id == media_id {
                return Ok(Some(job.id));
            }
        }
    }
    Ok(None)
}

// ── Paths ─────────────────────────────────────────────────────────────────────

fn whisper_models_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    Ok(app_data_dir(app)?.join("models").join("whisper"))
}

fn transcripts_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    Ok(app_data_dir(app)?.join("transcripts"))
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    app.path()
        .app_data_dir()
        .map_err(|e| ApiError::Internal(e.to_string()))
}
