//! Streaming caption session (Phase 6.1).
//!
//! Instead of transcribing a whole movie before showing anything, a session
//! transcribes ~30s windows starting at the current playhead, emits cues as
//! each window finishes, prefetches a little ahead, and persists each window so
//! a re-watch is instant. There is exactly one active session (the open
//! player); starting a new one cancels the previous.
//!
//! Cues are emitted via `transcription:cues` and rendered by the React
//! `CaptionOverlay` (never mpv's native renderer), so Phase 7's translation
//! card can reuse the same stream.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nova_core::TranscriptSegment;
use nova_media::extract_audio_window;
use nova_storage::{PlaybackRepository, TranscriptRepository};
use nova_transcribe::{
    default_model, ensure_model, segments_to_vtt, shift_transcript, transcribe_wav,
    ModelDownloadProgress, TranscribeOptions,
};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

/// Window the transcriber works in. 30s is whisper's native context length.
const WINDOW_SEC: f64 = 30.0;
/// Extra audio pulled before each window so the first word isn't clipped.
const LEAD_IN_SEC: f64 = 1.0;
/// Stop transcribing once this far ahead of the playhead; resume as it catches up.
const PREFETCH_AHEAD_SEC: f64 = 90.0;
/// Per-window ffmpeg budget. A 31s window extracts in well under this.
const EXTRACT_DEADLINE: Duration = Duration::from_secs(120);
/// Coverage comparison epsilon (seconds).
const EPS: f64 = 0.05;

#[derive(Default)]
pub struct CaptionSessionManager {
    active: Mutex<Option<Arc<SessionHandle>>>,
}

struct SessionHandle {
    cancel: Arc<AtomicBool>,
    /// Live playhead in centiseconds (so it fits an atomic).
    playhead_cs: Arc<AtomicU64>,
}

/// Everything a session needs to run, assembled by the command layer.
pub struct StartCtx {
    pub app: AppHandle,
    pub pool: Arc<SqlitePool>,
    pub media_kind: String,
    pub media_id: i64,
    pub from_sec: f64,
    pub duration_sec: Option<f64>,
    pub models_dir: PathBuf,
    pub transcripts_dir: PathBuf,
}

impl CaptionSessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a session, cancelling any previous one.
    pub async fn start(&self, ctx: StartCtx) {
        let cancel = Arc::new(AtomicBool::new(false));
        let playhead_cs = Arc::new(AtomicU64::new((ctx.from_sec.max(0.0) * 100.0) as u64));
        let handle = Arc::new(SessionHandle {
            cancel: Arc::clone(&cancel),
            playhead_cs: Arc::clone(&playhead_cs),
        });

        {
            let mut guard = self.active.lock().await;
            if let Some(prev) = guard.take() {
                prev.cancel.store(true, Ordering::SeqCst);
            }
            *guard = Some(Arc::clone(&handle));
        }

        tokio::spawn(run_session(ctx, cancel, playhead_cs));
    }

    /// Update the live playhead so the session prioritises/throttles correctly.
    pub async fn set_playhead(&self, sec: f64) {
        if let Some(h) = self.active.lock().await.as_ref() {
            h.playhead_cs
                .store((sec.max(0.0) * 100.0) as u64, Ordering::SeqCst);
        }
    }

    /// Cancel the active session (player closed / captions turned off).
    pub async fn stop(&self) {
        if let Some(h) = self.active.lock().await.take() {
            h.cancel.store(true, Ordering::SeqCst);
        }
    }
}

// ── Event payloads ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct CueDto {
    start: f64,
    end: f64,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CuesEvent {
    media_kind: String,
    media_id: i64,
    language: String,
    cues: Vec<CueDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    media_kind: String,
    media_id: i64,
    /// `preparing` | `streaming` | `complete` | `failed`
    state: String,
    covered_sec: Option<f64>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDownloadEvent {
    artifact: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

// ── The session loop ──────────────────────────────────────────────────────────

async fn run_session(ctx: StartCtx, cancel: Arc<AtomicBool>, playhead_cs: Arc<AtomicU64>) {
    if let Err(e) = run_session_inner(&ctx, &cancel, &playhead_cs).await {
        if !cancel.load(Ordering::SeqCst) {
            emit_status(&ctx, "failed", None, Some(e));
        }
    }
}

async fn run_session_inner(
    ctx: &StartCtx,
    cancel: &AtomicBool,
    playhead_cs: &AtomicU64,
) -> Result<(), String> {
    emit_status(ctx, "preparing", None, None);

    // Ensure the model is present (the UI normally downloads it first; this is
    // a defensive no-op when already installed).
    let model = default_model();
    let app_for_model = ctx.app.clone();
    let paths = ensure_model(&ctx.models_dir, &model, move |p: ModelDownloadProgress| {
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
    if cancel.load(Ordering::SeqCst) {
        return Ok(());
    }

    // Resolve the upstream URL (+ any known source language).
    let resolved = PlaybackRepository::new(&ctx.pool)
        .resolve_stream_url(&ctx.media_kind, ctx.media_id)
        .await
        .map_err(|e| format!("resolve stream: {e}"))?;

    let repo = TranscriptRepository::new(&ctx.pool);
    let duration = ctx
        .duration_sec
        .filter(|d| *d > 0.0)
        .unwrap_or(f64::INFINITY);

    // Load any cached transcript and replay its cues immediately.
    let mut transcript_id: Option<i64> = None;
    let mut language = resolved.source_lang.clone().unwrap_or_default();
    let mut covered: Vec<(f64, f64)> = Vec::new();
    if let Some(existing) = repo
        .get_for_media(&ctx.media_kind, ctx.media_id)
        .await
        .map_err(|e| e.to_string())?
    {
        transcript_id = Some(existing.id);
        language = existing.language.clone();
        covered = repo
            .covered_windows(existing.id)
            .await
            .map_err(|e| e.to_string())?;
        let segments = repo
            .get_segments(existing.id)
            .await
            .map_err(|e| e.to_string())?;
        if !segments.is_empty() {
            emit_cues(ctx, &language, &segments);
        }
    }

    emit_status(ctx, "streaming", Some(covered_total(&covered)), None);

    let mut empty_streak = 0u32;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        let playhead = playhead_cs.load(Ordering::SeqCst) as f64 / 100.0;

        let Some(w) = next_window(&covered, playhead, duration) else {
            // Buffered far enough ahead, or everything in range is covered.
            if covers_to(&covered, duration) {
                if let Some(id) = transcript_id {
                    let _ = repo.mark_complete(id).await;
                }
                emit_status(ctx, "complete", Some(covered_total(&covered)), None);
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
            continue;
        };

        // Extract + transcribe this window.
        let work = tempfile::Builder::new()
            .prefix("nova-asr-win-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let wav = work.path().join("win.wav");
        let audio_start = (w - LEAD_IN_SEC).max(0.0);
        let audio_dur = (w + WINDOW_SEC) - audio_start;

        if let Err(e) = extract_audio_window(
            &resolved.upstream_url,
            &wav,
            audio_start,
            audio_dur,
            EXTRACT_DEADLINE,
        )
        .await
        {
            // First window failing is fatal; later failures are likely EOF —
            // mark the window covered so we advance instead of looping.
            if covered.is_empty() {
                return Err(format!("audio extraction: {e}"));
            }
            mark_covered(&repo, &mut covered, transcript_id, w).await;
            empty_streak += 1;
            if empty_streak >= 3 {
                if let Some(id) = transcript_id {
                    let _ = repo.mark_complete(id).await;
                }
                emit_status(ctx, "complete", Some(covered_total(&covered)), None);
                return Ok(());
            }
            continue;
        }

        let out_base = work.path().join("win");
        let opts = TranscribeOptions {
            language: resolved.source_lang.clone(),
            model_id: model.id.to_string(),
        };
        let mut raw = match transcribe_wav(&wav, &paths.ggml, &out_base, &opts, |_pct| {}).await {
            Ok(t) => t,
            Err(e) => {
                if covered.is_empty() {
                    return Err(format!("whisper: {e}"));
                }
                mark_covered(&repo, &mut covered, transcript_id, w).await;
                continue;
            }
        };
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }

        shift_transcript(&mut raw, audio_start);
        // Drop the lead-in overlap so it isn't duplicated from the prior window.
        raw.segments.retain(|s| s.start_sec >= w - 0.25);

        if raw.segments.is_empty() {
            empty_streak += 1;
        } else {
            empty_streak = 0;
        }

        // Create the transcript row lazily once we know the language.
        if transcript_id.is_none() {
            if raw.language != "und" && !raw.language.is_empty() {
                language = raw.language.clone();
            }
            let base = format!("{}-{}-{}", ctx.media_kind, ctx.media_id, language);
            let vtt_path = ctx.transcripts_dir.join(format!("{base}.vtt"));
            let json_path = ctx.transcripts_dir.join(format!("{base}.json"));
            let _ = std::fs::create_dir_all(&ctx.transcripts_dir);
            let id = repo
                .get_or_create_transcript(
                    &ctx.media_kind,
                    ctx.media_id,
                    &language,
                    model.id,
                    &vtt_path.to_string_lossy(),
                    &json_path.to_string_lossy(),
                )
                .await
                .map_err(|e| e.to_string())?;
            transcript_id = Some(id);
        }
        let id = transcript_id.expect("transcript id set above");

        if !raw.segments.is_empty() {
            repo.append_segments(id, &raw.segments)
                .await
                .map_err(|e| e.to_string())?;
            emit_cues(ctx, &language, &raw.segments);
            refresh_vtt(
                &repo,
                id,
                &ctx.transcripts_dir,
                &ctx.media_kind,
                ctx.media_id,
                &language,
            )
            .await;
        }
        mark_covered(&repo, &mut covered, Some(id), w).await;

        emit_status(ctx, "streaming", Some(covered_total(&covered)), None);

        // EOF heuristic when duration is unknown.
        if duration.is_infinite() && empty_streak >= 3 {
            let _ = repo.mark_complete(id).await;
            emit_status(ctx, "complete", Some(covered_total(&covered)), None);
            return Ok(());
        }
    }
}

/// Pick the earliest uncovered window at/after the playhead, within the prefetch
/// horizon and the media duration. `None` means "buffered enough for now".
fn next_window(covered: &[(f64, f64)], playhead: f64, duration: f64) -> Option<f64> {
    let start = (playhead.max(0.0) / WINDOW_SEC).floor() * WINDOW_SEC;
    let limit = (playhead + PREFETCH_AHEAD_SEC).min(duration);
    let mut w = start;
    while w < limit {
        if w < duration && !window_covered(covered, w) {
            return Some(w);
        }
        w += WINDOW_SEC;
    }
    None
}

fn window_covered(covered: &[(f64, f64)], w: f64) -> bool {
    covered
        .iter()
        .any(|(s, e)| *s <= w + EPS && *e >= w + WINDOW_SEC - EPS)
}

/// Record a window as covered both in memory (drives loop decisions) and, when
/// a transcript exists, persisted for future sessions. Best-effort persistence.
async fn mark_covered(
    repo: &TranscriptRepository<'_>,
    covered: &mut Vec<(f64, f64)>,
    transcript_id: Option<i64>,
    w: f64,
) {
    covered.push((w, w + WINDOW_SEC));
    if let Some(id) = transcript_id {
        let _ = repo.record_window(id, w, w + WINDOW_SEC).await;
    }
}

/// Total covered seconds (sum of window lengths; windows don't overlap).
fn covered_total(covered: &[(f64, f64)]) -> f64 {
    covered.iter().map(|(s, e)| e - s).sum()
}

/// True if the covered windows reach `duration` (the whole file is transcribed).
fn covers_to(covered: &[(f64, f64)], duration: f64) -> bool {
    if !duration.is_finite() {
        return false;
    }
    let mut w = 0.0;
    while w < duration - EPS {
        if !window_covered(covered, w) {
            return false;
        }
        w += WINDOW_SEC;
    }
    true
}

async fn refresh_vtt(
    repo: &TranscriptRepository<'_>,
    transcript_id: i64,
    dir: &std::path::Path,
    media_kind: &str,
    media_id: i64,
    language: &str,
) {
    if let Ok(segments) = repo.get_segments(transcript_id).await {
        let vtt = segments_to_vtt(&segments);
        let path = dir.join(format!("{media_kind}-{media_id}-{language}.vtt"));
        let _ = std::fs::write(path, vtt);
    }
}

fn emit_cues(ctx: &StartCtx, language: &str, segments: &[TranscriptSegment]) {
    let cues = segments
        .iter()
        .map(|s| CueDto {
            start: s.start_sec,
            end: s.end_sec,
            text: s.text.clone(),
        })
        .collect();
    let _ = ctx.app.emit(
        "transcription:cues",
        CuesEvent {
            media_kind: ctx.media_kind.clone(),
            media_id: ctx.media_id,
            language: language.to_string(),
            cues,
        },
    );
}

fn emit_status(ctx: &StartCtx, state: &str, covered_sec: Option<f64>, error: Option<String>) {
    let _ = ctx.app.emit(
        "transcription:status",
        StatusEvent {
            media_kind: ctx.media_kind.clone(),
            media_id: ctx.media_id,
            state: state.to_string(),
            covered_sec,
            error,
        },
    );
}
