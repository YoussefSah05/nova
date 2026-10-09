//! Streaming caption session (Phase 6.1 / 6.2).
//!
//! Instead of transcribing a whole movie before showing anything, a session
//! transcribes windows starting at the current playhead, emits cues as each
//! window finishes, prefetches a little ahead, and persists each window so a
//! re-watch is instant. There is exactly one active session (the open player);
//! starting a new one cancels the previous.
//!
//! **Windowing (6.2):** rather than a fixed 30s grid that hard-cuts sentences,
//! a sliding cursor extracts up to [`WINDOW_SEC`] of audio (with a short
//! [`OVERLAP_SEC`] lead-in for context) and advances only to the end of the last
//! *complete* segment whisper produced — the trailing, possibly-truncated
//! sentence is re-read in the next window with the previous tail as a `--prompt`.
//! Coverage is tracked as real `[start, end)` intervals, so windows align to
//! speech, not the clock. Token timestamps are DTW-aligned (see `whisper.rs`).
//!
//! **Overlap stitch (6.3):** segments are kept by where they *end* (anything
//! past the confirmed frontier `gap_start`), not where they start, then their
//! already-shown leading words are trimmed off — a LocalAgreement-style stable
//! prefix. A start-based drop loses boundary-spanning segments in long,
//! pauseless dialogue (the new content past the boundary vanishes); the
//! end-based keep is lossless. Whisper emits caption-sized segments via
//! `--max-len`/`--split-on-word` ([`MAX_LEN_CHARS`]) so cues stay short and the
//! stitch boundaries are fine-grained.
//!
//! Cues are emitted via `transcription:cues` and rendered by the React
//! `CaptionOverlay` (never mpv's native renderer), so Phase 7's translation
//! card can reuse the same stream.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::model_runtime::{ModelRuntimeManager, RuntimeCancellation, RuntimeError};
use crate::resource_policy::WorkClass;
use nova_core::TranscriptSegment;
use nova_media::extract_audio_window;
use nova_storage::{PlaybackRepository, TranscriptRepository};
use nova_transcribe::{
    default_model, ensure_model, segments_to_vtt, shift_transcript, transcribe_wav_cancellable,
    ModelDownloadProgress, TranscribeOptions,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

/// Max audio handed to whisper per window. 30s is whisper's native context length.
const WINDOW_SEC: f64 = 30.0;
/// Audio pulled before each window's start so the model has lexical context and
/// boundary words are heard twice (2–3s is the long-form best practice). The
/// overlap is trimmed off on append so nothing is duplicated.
const OVERLAP_SEC: f64 = 3.0;
/// Caption-sized segment cap (chars) passed to whisper as `--max-len` (split on
/// word). Keeps cues short in dense speech and yields finer stitch boundaries.
const MAX_LEN_CHARS: u32 = 60;
/// Segments ending within this of the window's tail are treated as possibly
/// truncated by the hard 30s cut, so they're left for the next window to re-read.
const TRAILING_GUARD_SEC: f64 = 1.5;
/// If whisper found less audio than the window span by more than this, we've hit
/// the end of the file and every segment is complete.
const EOF_SHORTFALL_SEC: f64 = 2.0;
/// Floor on per-window progress so a window dominated by one long segment can't
/// stall the cursor.
const MIN_ADVANCE_SEC: f64 = 5.0;
/// Stop transcribing once this far ahead of the playhead; resume as it catches up.
const PREFETCH_AHEAD_SEC: f64 = 90.0;
/// Per-window ffmpeg budget. A ~32s window extracts in well under this.
const EXTRACT_DEADLINE: Duration = Duration::from_secs(120);
/// Coverage comparison epsilon (seconds).
const EPS: f64 = 0.05;
/// Words of the previous window's tail fed forward as the next `--prompt`.
const PROMPT_TAIL_WORDS: usize = 12;

#[derive(Default)]
pub struct CaptionSessionManager {
    active: Mutex<Option<Arc<SessionHandle>>>,
}

struct SessionHandle {
    session_id: String,
    profile_id: String,
    transcript_id: Arc<AtomicI64>,
    translation_slot: Arc<Semaphore>,
    cancel: Arc<AtomicBool>,
    model_cancel: RuntimeCancellation,
    /// Live playhead in centiseconds (so it fits an atomic).
    playhead_cs: Arc<AtomicU64>,
}

/// Everything a session needs to run, assembled by the command layer.
pub struct StartCtx {
    pub session_id: String,
    pub profile_id: String,
    pub app: AppHandle,
    pub pool: Arc<SqlitePool>,
    pub media_kind: String,
    pub media_id: i64,
    pub from_sec: f64,
    pub duration_sec: Option<f64>,
    /// 0-based audio-stream index to transcribe (multi-track content). `None`
    /// uses ffmpeg's default stream and the default track slot (0).
    pub audio_track: Option<u32>,
    /// Language of the chosen audio track (from the player's track metadata),
    /// used as the transcription hint. `None` lets whisper auto-detect.
    pub track_lang: Option<String>,
    pub models_dir: PathBuf,
    pub model_root: PathBuf,
    pub runtime: Arc<ModelRuntimeManager>,
    pub transcripts_dir: PathBuf,
}

impl CaptionSessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a session, cancelling any previous one.
    pub async fn start(&self, ctx: StartCtx) {
        let cancel = Arc::new(AtomicBool::new(false));
        let model_cancel = RuntimeCancellation::default();
        let playhead_cs = Arc::new(AtomicU64::new((ctx.from_sec.max(0.0) * 100.0) as u64));
        let transcript_id = Arc::new(AtomicI64::new(0));
        let handle = Arc::new(SessionHandle {
            session_id: ctx.session_id.clone(),
            profile_id: ctx.profile_id.clone(),
            transcript_id: Arc::clone(&transcript_id),
            translation_slot: Arc::new(Semaphore::new(1)),
            cancel: Arc::clone(&cancel),
            model_cancel: model_cancel.clone(),
            playhead_cs: Arc::clone(&playhead_cs),
        });

        {
            let mut guard = self.active.lock().await;
            if let Some(prev) = guard.take() {
                prev.cancel.store(true, Ordering::SeqCst);
                prev.model_cancel.cancel();
            }
            *guard = Some(Arc::clone(&handle));
        }

        tokio::spawn(run_session(
            ctx,
            cancel,
            model_cancel,
            playhead_cs,
            transcript_id,
        ));
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
            h.model_cancel.cancel();
        }
    }

    /// Translation borrows the caption session's cancellation, so closing the
    /// player or switching media/track also cancels waiting/in-flight inference.
    pub async fn translation_context(
        &self,
        session_id: &str,
        profile_id: &str,
        transcript_id: i64,
    ) -> Option<(RuntimeCancellation, OwnedSemaphorePermit)> {
        self.active
            .lock()
            .await
            .as_ref()
            .filter(|h| {
                h.session_id == session_id
                    && h.profile_id == profile_id
                    && transcript_id > 0
                    && h.transcript_id.load(Ordering::SeqCst) == transcript_id
            })
            .and_then(|h| {
                h.translation_slot
                    .clone()
                    .try_acquire_owned()
                    .ok()
                    .map(|permit| (h.model_cancel.clone(), permit))
            })
    }
}

// ── Event payloads ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct CaptionCue {
    #[cfg_attr(test, ts(type = "number"))]
    pub segment_id: i64,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct CuesEvent {
    session_id: String,
    media_kind: String,
    #[cfg_attr(test, ts(type = "number"))]
    media_id: i64,
    #[cfg_attr(test, ts(type = "number"))]
    transcript_id: i64,
    language: String,
    cues: Vec<CaptionCue>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    session_id: String,
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

async fn run_session(
    ctx: StartCtx,
    cancel: Arc<AtomicBool>,
    model_cancel: RuntimeCancellation,
    playhead_cs: Arc<AtomicU64>,
    active_transcript: Arc<AtomicI64>,
) {
    if let Err(e) = run_session_inner(
        &ctx,
        &cancel,
        &model_cancel,
        &playhead_cs,
        &active_transcript,
    )
    .await
    {
        if !cancel.load(Ordering::SeqCst) {
            emit_status(&ctx, "failed", None, Some(e));
        }
    }
}

async fn run_session_inner(
    ctx: &StartCtx,
    cancel: &AtomicBool,
    model_cancel: &RuntimeCancellation,
    playhead_cs: &AtomicU64,
    active_transcript: &AtomicI64,
) -> Result<(), String> {
    emit_status(ctx, "preparing", None, None);

    // Prefer a registered available location; retain the previous internal
    // download behavior when no local copy exists.
    let model = default_model();
    match ctx
        .runtime
        .acquire_at(&ctx.model_root, model.id, WorkClass::Realtime, model_cancel)
        .await
    {
        Ok(_lease) => {}
        Err(RuntimeError::Unavailable) => {
            let app_for_model = ctx.app.clone();
            let models_dir = ctx.models_dir.clone();
            let download_model = model.clone();
            ctx.runtime
                .mutate(model.id, async move {
                    ensure_model(
                        &models_dir,
                        &download_model,
                        move |p: ModelDownloadProgress| {
                            let _ = app_for_model.emit(
                                "model:download:progress",
                                ModelDownloadEvent {
                                    artifact: p.artifact.to_string(),
                                    downloaded_bytes: p.downloaded_bytes,
                                    total_bytes: p.total_bytes,
                                },
                            );
                        },
                    )
                    .await
                    .map_err(|e| format!("model: {e}"))
                })
                .await
                .map_err(|e| e.to_string())??;
        }
        Err(e) => return Err(e.to_string()),
    }
    if cancel.load(Ordering::SeqCst) {
        return Ok(());
    }

    // Resolve the upstream URL (+ any known source language).
    let resolved = PlaybackRepository::new(&ctx.pool)
        .resolve_stream_url(&ctx.media_kind, ctx.media_id)
        .await
        .map_err(|e| format!("resolve stream: {e}"))?;

    // Stable cache identity (survives a media re-import) + the per-track slot.
    let content_key = content_key(&resolved.upstream_url);
    let track_slot = ctx.audio_track.unwrap_or(0) as i64;
    // Per-track language hint: prefer the chosen track's tag, else the media's.
    let lang_hint = ctx
        .track_lang
        .clone()
        .or_else(|| resolved.source_lang.clone());

    let repo = TranscriptRepository::new(&ctx.pool);
    let duration = ctx
        .duration_sec
        .filter(|d| *d > 0.0)
        .unwrap_or(f64::INFINITY);

    // Load any cached transcript for this track and replay its cues immediately.
    let mut transcript_id: Option<i64> = None;
    let mut language = lang_hint.clone().unwrap_or_default();
    let mut covered: Vec<(f64, f64)> = Vec::new();
    let mut prompt_tail: Option<String> = None;
    if let Some(existing) = repo
        .get_for_media_track(&content_key, &ctx.media_kind, ctx.media_id, track_slot)
        .await
        .map_err(|e| e.to_string())?
    {
        transcript_id = Some(existing.id);
        active_transcript.store(existing.id, Ordering::SeqCst);
        language = existing.language.clone();
        for (s, e) in repo
            .covered_windows(existing.id)
            .await
            .map_err(|e| e.to_string())?
        {
            merge_interval(&mut covered, s, e);
        }
        let segments = repo
            .get_segments(existing.id)
            .await
            .map_err(|e| e.to_string())?;
        if !segments.is_empty() {
            prompt_tail = tail_prompt(&segments);
            let stored = repo
                .segments_with_ids(existing.id)
                .await
                .map_err(|e| e.to_string())?;
            emit_cues(ctx, existing.id, &language, &stored);
        }
    }

    emit_status(ctx, "streaming", Some(covered_total(&covered)), None);

    let mut empty_streak = 0u32;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        let playhead = playhead_cs.load(Ordering::SeqCst) as f64 / 100.0;

        let limit = (playhead + PREFETCH_AHEAD_SEC).min(duration);
        let Some(gap_start) = next_gap(&covered, playhead, limit) else {
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

        // Extract a window starting at the gap, with a short lead-in for context.
        let work = tempfile::Builder::new()
            .prefix("nova-asr-win-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let wav = work.path().join("win.wav");
        let audio_start = (gap_start - OVERLAP_SEC).max(0.0);
        let window_end = gap_start + WINDOW_SEC;
        let audio_dur = window_end - audio_start;

        if let Err(e) = extract_audio_window(
            &resolved.upstream_url,
            &wav,
            audio_start,
            audio_dur,
            ctx.audio_track,
            EXTRACT_DEADLINE,
        )
        .await
        {
            // First window failing is fatal; later failures are likely EOF —
            // mark the span covered so we advance instead of looping.
            if covered.is_empty() {
                return Err(format!("audio extraction: {e}"));
            }
            mark_covered(&repo, &mut covered, transcript_id, gap_start, window_end).await;
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
            language: lang_hint.clone(),
            model_id: model.id.to_string(),
            dtw_preset: model.dtw_preset.to_string(),
            initial_prompt: prompt_tail.clone(),
            max_len: MAX_LEN_CHARS,
        };
        let lease = ctx
            .runtime
            .acquire_at(&ctx.model_root, model.id, WorkClass::Realtime, model_cancel)
            .await
            .map_err(|e| e.to_string())?;
        let child_cancel = model_cancel.clone();
        let result = lease
            .run_async_at(WorkClass::Realtime, model_cancel, move |lease| async move {
                let _work = work;
                let model_path = lease.whisper_path().ok_or("wrong model backend")?;
                transcribe_wav_cancellable(
                    &wav,
                    model_path,
                    &out_base,
                    &opts,
                    |_pct| {},
                    child_cancel.cancelled(),
                )
                .await
                .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?;
        let mut raw = match result {
            Ok(t) => t,
            Err(e) => {
                if covered.is_empty() {
                    return Err(format!("whisper: {e}"));
                }
                mark_covered(&repo, &mut covered, transcript_id, gap_start, window_end).await;
                continue;
            }
        };
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }

        shift_transcript(&mut raw, audio_start);
        // Overlap stitch (LocalAgreement-style confirmed prefix). Keep any
        // segment that reaches past the confirmed frontier `gap_start` — even
        // one that *starts* inside the lead-in overlap and *extends* into new
        // dialogue (common in pauseless speech) — then trim the already-shown
        // leading words off it. A start-based drop here loses that new content.
        raw.segments
            .retain(|s| s.end_sec > gap_start + EPS && !point_covered(&covered, midpoint(s)));
        for s in &mut raw.segments {
            if s.start_sec < gap_start {
                trim_leading_words(s, gap_start);
            }
        }
        raw.segments.retain(|s| !s.text.trim().is_empty());

        // Decide which segments are "complete" and how far the cursor advances.
        let (keep, advance_to) = split_complete(&raw.segments, gap_start, window_end);

        if keep.is_empty() {
            empty_streak += 1;
        } else {
            empty_streak = 0;
        }

        // Create the transcript row lazily once we know the language.
        if transcript_id.is_none() {
            if raw.language != "und" && !raw.language.is_empty() {
                language = raw.language.clone();
            }
            let base = transcript_basename(&ctx.media_kind, ctx.media_id, track_slot, &language);
            let vtt_path = ctx.transcripts_dir.join(format!("{base}.vtt"));
            let json_path = ctx.transcripts_dir.join(format!("{base}.json"));
            let _ = std::fs::create_dir_all(&ctx.transcripts_dir);
            let id = repo
                .get_or_create_transcript(
                    &content_key,
                    &ctx.media_kind,
                    ctx.media_id,
                    track_slot,
                    &language,
                    model.id,
                    &vtt_path.to_string_lossy(),
                    &json_path.to_string_lossy(),
                )
                .await
                .map_err(|e| e.to_string())?;
            transcript_id = Some(id);
            active_transcript.store(id, Ordering::SeqCst);
        }
        let id = transcript_id.expect("transcript id set above");

        if !keep.is_empty() {
            let ids = repo
                .append_segments(id, &keep)
                .await
                .map_err(|e| e.to_string())?;
            prompt_tail = tail_prompt(&keep);
            let stored: Vec<_> = keep
                .iter()
                .zip(ids)
                .map(|(s, id)| nova_storage::SourceSegment {
                    id,
                    start_sec: s.start_sec,
                    end_sec: s.end_sec,
                    text: s.text.clone(),
                })
                .collect();
            emit_cues(ctx, id, &language, &stored);
            refresh_vtt(
                &repo,
                id,
                &ctx.transcripts_dir,
                &ctx.media_kind,
                ctx.media_id,
                track_slot,
                &language,
            )
            .await;
        }
        mark_covered(&repo, &mut covered, Some(id), gap_start, advance_to).await;

        emit_status(ctx, "streaming", Some(covered_total(&covered)), None);

        // EOF heuristic when duration is unknown.
        if duration.is_infinite() && empty_streak >= 3 {
            let _ = repo.mark_complete(id).await;
            emit_status(ctx, "complete", Some(covered_total(&covered)), None);
            return Ok(());
        }
    }
}

/// Earliest uncovered instant in `[from, limit)`. `None` means fully covered up
/// to `limit`. `covered` must be sorted & merged (see [`merge_interval`]).
fn next_gap(covered: &[(f64, f64)], from: f64, limit: f64) -> Option<f64> {
    let mut cursor = from.max(0.0);
    for &(s, e) in covered {
        if e <= cursor + EPS {
            continue; // entirely behind the cursor
        }
        if s > cursor + EPS {
            break; // gap opens at `cursor`, before this interval
        }
        cursor = cursor.max(e); // covered through `e`; jump past it
    }
    (cursor < limit - EPS).then_some(cursor)
}

/// Whether `x` falls inside any covered interval.
fn point_covered(covered: &[(f64, f64)], x: f64) -> bool {
    covered.iter().any(|&(s, e)| x >= s - EPS && x <= e + EPS)
}

fn midpoint(s: &TranscriptSegment) -> f64 {
    (s.start_sec + s.end_sec) / 2.0
}

/// Trim the overlap off a boundary-spanning segment: drop leading words that end
/// at/before `gap_start` (already shown by the previous window) and re-base the
/// segment on the survivors. Uses DTW word timings when present; otherwise just
/// clips `start_sec` (the text may repeat ~1 word — rare and harmless). Leaves
/// the segment's `text` empty if every word was trimmed, so the caller drops it.
fn trim_leading_words(s: &mut TranscriptSegment, gap_start: f64) {
    match &mut s.words {
        Some(words) => {
            words.retain(|w| w.end_sec > gap_start + EPS);
            if let Some(first) = words.first() {
                s.start_sec = first.start_sec;
                s.text = words
                    .iter()
                    .map(|w| w.word.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
            } else {
                s.text.clear();
            }
        }
        None => {
            // No word timings: keep the text, just clip the displayed start.
            s.start_sec = s.start_sec.max(gap_start);
        }
    }
}

/// Split a window's (already-deduped) segments into the ones to keep/emit now
/// and the absolute time the coverage cursor should advance to. The trailing
/// segment(s) within [`TRAILING_GUARD_SEC`] of the hard window cut are dropped
/// (likely truncated) and re-read by the next window — unless we've hit EOF, in
/// which case every segment is complete.
fn split_complete(
    segments: &[TranscriptSegment],
    gap_start: f64,
    window_end: f64,
) -> (Vec<TranscriptSegment>, f64) {
    if segments.is_empty() {
        // Nothing here (silence / extraction gap): cover the whole span.
        return (Vec::new(), window_end);
    }
    let max_end = segments.iter().fold(0.0_f64, |m, s| m.max(s.end_sec));
    let reached_eof = max_end < window_end - EOF_SHORTFALL_SEC;
    if reached_eof {
        return (segments.to_vec(), max_end);
    }

    let cut = window_end - TRAILING_GUARD_SEC;
    let complete: Vec<TranscriptSegment> = segments
        .iter()
        .filter(|s| s.end_sec <= cut)
        .cloned()
        .collect();
    if let Some(last) = complete.last() {
        (complete.clone(), last.end_sec.max(gap_start + EPS))
    } else {
        // One long segment fills the window with no clean break — keep it all
        // and make forced progress so the cursor can't stall.
        let advance = max_end.max(gap_start + MIN_ADVANCE_SEC);
        (segments.to_vec(), advance)
    }
}

/// Last few words of the most recent segments, for the next window's `--prompt`.
fn tail_prompt(segments: &[TranscriptSegment]) -> Option<String> {
    let text: String = segments
        .iter()
        .rev()
        .take(3)
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" ");
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let start = words.len().saturating_sub(PROMPT_TAIL_WORDS);
    Some(words[start..].join(" "))
}

/// Merge `[start, end)` into a sorted, non-overlapping coverage set.
fn merge_interval(covered: &mut Vec<(f64, f64)>, start: f64, end: f64) {
    if end <= start + EPS {
        return;
    }
    covered.push((start, end));
    covered.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(covered.len());
    for &(s, e) in covered.iter() {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 + EPS {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    *covered = merged;
}

/// Record `[start, advance_to)` as covered in memory and, when a transcript
/// exists, persisted for future sessions. Best-effort persistence.
async fn mark_covered(
    repo: &TranscriptRepository<'_>,
    covered: &mut Vec<(f64, f64)>,
    transcript_id: Option<i64>,
    start: f64,
    advance_to: f64,
) {
    if advance_to <= start + EPS {
        return;
    }
    merge_interval(covered, start, advance_to);
    if let Some(id) = transcript_id {
        let _ = repo.record_window(id, start, advance_to).await;
    }
}

/// Total covered seconds (intervals are merged, so they don't overlap).
fn covered_total(covered: &[(f64, f64)]) -> f64 {
    covered.iter().map(|(s, e)| e - s).sum()
}

/// True if coverage reaches `duration` (the whole file is transcribed).
fn covers_to(covered: &[(f64, f64)], duration: f64) -> bool {
    duration.is_finite() && next_gap(covered, 0.0, duration - EPS).is_none()
}

async fn refresh_vtt(
    repo: &TranscriptRepository<'_>,
    transcript_id: i64,
    dir: &std::path::Path,
    media_kind: &str,
    media_id: i64,
    audio_track: i64,
    language: &str,
) {
    if let Ok(segments) = repo.get_segments(transcript_id).await {
        let vtt = segments_to_vtt(&segments);
        let path = dir.join(format!(
            "{}.vtt",
            transcript_basename(media_kind, media_id, audio_track, language)
        ));
        let _ = std::fs::write(path, vtt);
    }
}

/// On-disk artifact basename, namespaced by audio track so multi-track
/// transcripts don't clobber each other.
fn transcript_basename(
    media_kind: &str,
    media_id: i64,
    audio_track: i64,
    language: &str,
) -> String {
    format!("{media_kind}-{media_id}-t{audio_track}-{language}")
}

/// Stable cache key for a media item: a SHA-256 of its upstream stream URL.
/// Survives a playlist refresh (which renumbers `media_id`) as long as the
/// provider keeps the URL stable, and keeps any credentials in the URL opaque.
pub(crate) fn content_key(upstream_url: &str) -> String {
    let digest = Sha256::digest(upstream_url.as_bytes());
    hex_lower(&digest)
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn emit_cues(
    ctx: &StartCtx,
    transcript_id: i64,
    language: &str,
    segments: &[nova_storage::SourceSegment],
) {
    let cues = segments
        .iter()
        .map(|s| CaptionCue {
            segment_id: s.id,
            start: s.start_sec,
            end: s.end_sec,
            text: s.text.clone(),
        })
        .collect();
    let _ = ctx.app.emit(
        "transcription:cues",
        CuesEvent {
            session_id: ctx.session_id.clone(),
            media_kind: ctx.media_kind.clone(),
            media_id: ctx.media_id,
            transcript_id,
            language: language.to_string(),
            cues,
        },
    );
}

fn emit_status(ctx: &StartCtx, state: &str, covered_sec: Option<f64>, error: Option<String>) {
    let _ = ctx.app.emit(
        "transcription:status",
        StatusEvent {
            session_id: ctx.session_id.clone(),
            media_kind: ctx.media_kind.clone(),
            media_id: ctx.media_id,
            state: state.to_string(),
            covered_sec,
            error,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            seq: 0,
            start_sec: start,
            end_sec: end,
            text: text.to_string(),
            words: None,
        }
    }

    #[tokio::test]
    async fn translation_is_scoped_bounded_and_cancelled_with_the_caption_session() {
        let manager = CaptionSessionManager::new();
        let cancel = RuntimeCancellation::default();
        *manager.active.lock().await = Some(Arc::new(SessionHandle {
            session_id: "session".into(),
            profile_id: "profile".into(),
            transcript_id: Arc::new(AtomicI64::new(7)),
            translation_slot: Arc::new(Semaphore::new(1)),
            cancel: Arc::new(AtomicBool::new(false)),
            model_cancel: cancel.clone(),
            playhead_cs: Arc::new(AtomicU64::new(0)),
        }));
        assert!(manager
            .translation_context("old", "profile", 7)
            .await
            .is_none());
        assert!(manager
            .translation_context("session", "other", 7)
            .await
            .is_none());
        assert!(manager
            .translation_context("session", "profile", 8)
            .await
            .is_none());
        let (borrowed, permit) = manager
            .translation_context("session", "profile", 7)
            .await
            .unwrap();
        assert!(manager
            .translation_context("session", "profile", 7)
            .await
            .is_none());
        drop(permit);
        assert!(manager
            .translation_context("session", "profile", 7)
            .await
            .is_some());
        manager.stop().await;
        assert!(borrowed.is_cancelled());
        assert!(manager
            .translation_context("session", "profile", 7)
            .await
            .is_none());
    }

    #[test]
    fn merge_interval_coalesces_adjacent_and_overlapping() {
        let mut c = Vec::new();
        merge_interval(&mut c, 0.0, 10.0);
        merge_interval(&mut c, 30.0, 40.0);
        merge_interval(&mut c, 9.5, 31.0); // bridges the gap
        assert_eq!(c, vec![(0.0, 40.0)]);
    }

    #[test]
    fn next_gap_finds_first_hole_after_playhead() {
        let c = vec![(0.0, 20.0), (40.0, 60.0)];
        // Inside the first covered interval → jump to its end.
        assert_eq!(next_gap(&c, 5.0, 200.0), Some(20.0));
        // In the hole → return the playhead itself.
        assert_eq!(next_gap(&c, 25.0, 200.0), Some(25.0));
        // Past everything covered.
        assert_eq!(next_gap(&c, 50.0, 200.0), Some(60.0));
    }

    #[test]
    fn next_gap_none_when_covered_to_limit() {
        let c = vec![(0.0, 100.0)];
        assert_eq!(next_gap(&c, 10.0, 100.0), None);
    }

    #[test]
    fn covers_to_requires_full_coverage_not_just_prefetch_horizon() {
        // A single 95s window must NOT report a 600s file as complete, even
        // though it exceeds the 90s prefetch horizon.
        let c = vec![(0.0, 95.0)];
        assert!(!covers_to(&c, 600.0));
        assert!(covers_to(&[(0.0, 600.0)], 600.0));
    }

    #[test]
    fn split_complete_drops_truncated_tail_segment() {
        let gap = 0.0;
        let window_end = 30.0;
        // Two clean segments plus one butting against the 30s cut.
        let segs = [
            seg(0.5, 5.0, "first"),
            seg(5.2, 24.0, "second"),
            seg(24.5, 29.9, "cut off"),
        ];
        let (keep, advance) = split_complete(&segs, gap, window_end);
        assert_eq!(keep.len(), 2, "trailing truncated segment dropped");
        assert!((advance - 24.0).abs() < 1e-9, "cursor at last complete end");
    }

    #[test]
    fn split_complete_keeps_all_at_eof() {
        // Whisper found much less than the 30s window → end of file.
        let segs = [seg(0.5, 5.0, "a"), seg(5.2, 9.0, "b")];
        let (keep, advance) = split_complete(&segs, 0.0, 30.0);
        assert_eq!(keep.len(), 2);
        assert!((advance - 9.0).abs() < 1e-9);
    }

    #[test]
    fn split_complete_forces_progress_on_one_long_segment() {
        // A single segment spans the whole window with no clean break.
        let segs = [seg(0.2, 29.8, "an unbroken monologue")];
        let (keep, advance) = split_complete(&segs, 0.0, 30.0);
        assert_eq!(keep.len(), 1);
        assert!(advance >= MIN_ADVANCE_SEC);
    }

    #[test]
    fn split_complete_empty_covers_whole_span() {
        let (keep, advance) = split_complete(&[], 10.0, 40.0);
        assert!(keep.is_empty());
        assert!((advance - 40.0).abs() < 1e-9);
    }

    fn word(start: f64, end: f64, w: &str) -> nova_core::TranscriptWord {
        nova_core::TranscriptWord {
            start_sec: start,
            end_sec: end,
            word: w.to_string(),
        }
    }

    #[test]
    fn trim_leading_words_drops_overlap_and_rebases() {
        // Segment [26→30] spanning the boundary at gap_start=28.
        let mut s = TranscriptSegment {
            seq: 0,
            start_sec: 26.0,
            end_sec: 30.0,
            text: "already shown brand new words".to_string(),
            words: Some(vec![
                word(26.0, 26.8, "already"),
                word(26.8, 27.6, "shown"),
                word(28.2, 28.9, "brand"),
                word(29.0, 29.5, "new"),
                word(29.6, 30.0, "words"),
            ]),
        };
        trim_leading_words(&mut s, 28.0);
        assert_eq!(s.text, "brand new words");
        assert!(
            (s.start_sec - 28.2).abs() < 1e-9,
            "re-based to first surviving word"
        );
    }

    #[test]
    fn trim_leading_words_empties_when_all_overlap() {
        let mut s = TranscriptSegment {
            seq: 0,
            start_sec: 26.0,
            end_sec: 27.5,
            text: "all old".to_string(),
            words: Some(vec![word(26.0, 26.8, "all"), word(26.9, 27.5, "old")]),
        };
        trim_leading_words(&mut s, 28.0);
        assert!(
            s.text.is_empty(),
            "fully-overlapped segment is emptied so the caller drops it"
        );
    }

    #[test]
    fn trim_leading_words_clips_start_without_word_timings() {
        let mut s = seg(26.0, 30.0, "no word timings here");
        trim_leading_words(&mut s, 28.0);
        assert!((s.start_sec - 28.0).abs() < 1e-9);
        assert_eq!(
            s.text, "no word timings here",
            "text kept when no words to trim"
        );
    }

    #[test]
    fn tail_prompt_takes_last_words() {
        let segs = [
            seg(0.0, 1.0, "one two three"),
            seg(
                1.0,
                2.0,
                "four five six seven eight nine ten eleven twelve thirteen",
            ),
        ];
        let p = tail_prompt(&segs).unwrap();
        let n = p.split_whitespace().count();
        assert!(
            n <= PROMPT_TAIL_WORDS,
            "tail capped at {PROMPT_TAIL_WORDS} words, got {n}"
        );
        assert!(p.ends_with("thirteen"));
    }
}
