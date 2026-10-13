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
//! **Recognition and timing:** windows are decoded by the resident ASR worker
//! (ADR-0011), which keeps whisper.cpp loaded and runs the caption timing
//! stage around it: Silero VAD before Whisper (silent windows are never
//! decoded) and MMS CTC forced alignment after it (word times tied to the
//! audio). `whisper-cli` remains the fallback when the worker cannot start.
//!
//! **Cold start (ADR-0014):** the first window after a start is short, so the
//! first caption needs only a few seconds of audio; a title page may
//! *prepare* a session (transcribe the opening from the resume point before
//! Play), and the player's session then *adopts* it instead of starting over.
//!
//! Cues are emitted via `transcription:cues` and rendered by the React
//! `CaptionOverlay` (never mpv's native renderer), so Phase 7's translation
//! card can reuse the same stream.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::model_registry::bundled_manifest;
use crate::model_runtime::{ModelLease, ModelRuntimeManager, RuntimeCancellation, RuntimeError};
use crate::resource_policy::WorkClass;
use nova_core::model_manifest::Capability;
use nova_core::{RawTranscript, TranscriptSegment};
use nova_media::{extract_audio_window, read_wav_16k_mono};
use nova_storage::{PlaybackRepository, TranscriptRepository};
use nova_transcribe::streaming::{segments_of, DecodeRequest, TimingModels};
use nova_transcribe::whisper::normalize_whisper_lang;
use nova_transcribe::{
    default_model, ensure_model, segments_to_vtt, shift_transcript, transcribe_wav_cancellable,
    ModelDownloadProgress, TranscribeOptions, WhisperModel,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

/// New audio per window. With the lead-in below, a window is exactly
/// whisper's native 30 s context: a longer one makes whisper.cpp decode a
/// second, few-second chunk, which is slow and prone to invented text.
const WINDOW_SEC: f64 = 27.0;
/// New audio in the first window after a start: the first caption waits for
/// this much audio to download and decode, not a full window.
const FIRST_WINDOW_SEC: f64 = 8.0;
/// How far past its start position a prepared session transcribes before
/// the player adopts it.
const PREPARE_AHEAD_SEC: f64 = 45.0;
/// `horizon_cs` of a session with no horizon.
const NO_HORIZON: u64 = u64::MAX;
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
/// Tolerance for a decoded audio window reaching the actual end of a source.
const EOF_SHORTFALL_SEC: f64 = 1.0;
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
    /// Prepares cancelled before they arrived (the two calls can cross), so a
    /// late `prepare` for a closed page does not start.
    cancelled_prepares: std::sync::Mutex<std::collections::VecDeque<String>>,
}

/// How many crossed cancellations are remembered.
const CANCELLED_PREPARES_KEPT: usize = 16;

/// The session id caption events carry. Adoption switches it, so a prepared
/// session's later windows reach the player that took it over.
#[derive(Clone, Default)]
pub struct SessionIdent(Arc<std::sync::RwLock<String>>);

impl SessionIdent {
    fn get(&self) -> String {
        self.0.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn set(&self, id: &str) {
        *self.0.write().unwrap_or_else(|p| p.into_inner()) = id.to_string();
    }
}

struct SessionHandle {
    ident: SessionIdent,
    profile_id: String,
    media_kind: String,
    media_id: i64,
    track_slot: i64,
    transcript_id: Arc<AtomicI64>,
    translation_slot: Arc<Semaphore>,
    cancel: Arc<AtomicBool>,
    model_cancel: RuntimeCancellation,
    /// Live playhead in centiseconds (so it fits an atomic).
    playhead_cs: Arc<AtomicU64>,
    /// Started by a title page and not yet adopted by a player.
    preparing: AtomicBool,
    /// Transcribe no further than this (centiseconds); [`NO_HORIZON`] once
    /// a player owns the session.
    horizon_cs: AtomicU64,
    /// The confirmed speech language, for replaying cues on adoption.
    language: std::sync::Mutex<String>,
    /// The session loop has exited (complete, failed, or nothing to do):
    /// nothing may adopt it.
    finished: AtomicBool,
    /// Media duration in centiseconds once known ([`NO_HORIZON`] until then):
    /// a title page may not know it, the player that adopts the session does.
    duration_cs: AtomicU64,
}

impl SessionHandle {
    fn new(ctx: &StartCtx, preparing: bool) -> Self {
        let from_cs = (ctx.from_sec.max(0.0) * 100.0) as u64;
        Self {
            ident: ctx.ident.clone(),
            profile_id: ctx.profile_id.clone(),
            media_kind: ctx.media_kind.clone(),
            media_id: ctx.media_id,
            track_slot: ctx.audio_track.unwrap_or(0) as i64,
            transcript_id: Arc::new(AtomicI64::new(0)),
            translation_slot: Arc::new(Semaphore::new(1)),
            cancel: Arc::new(AtomicBool::new(false)),
            model_cancel: RuntimeCancellation::default(),
            playhead_cs: Arc::new(AtomicU64::new(from_cs)),
            preparing: AtomicBool::new(preparing),
            horizon_cs: AtomicU64::new(if preparing {
                from_cs + (PREPARE_AHEAD_SEC * 100.0) as u64
            } else {
                NO_HORIZON
            }),
            language: std::sync::Mutex::new(String::new()),
            finished: AtomicBool::new(false),
            duration_cs: AtomicU64::new(duration_cs(ctx.duration_sec)),
        }
    }

    /// The media duration, when a caller has supplied one.
    fn duration(&self) -> Option<f64> {
        match self.duration_cs.load(Ordering::SeqCst) {
            NO_HORIZON => None,
            cs => Some(cs as f64 / 100.0),
        }
    }

    /// Whether a new caller may take this prepared session over.
    fn adoptable_by(&self, ctx: &StartCtx) -> bool {
        self.live_prepare() && self.same_media(ctx)
    }

    /// A prepared session that is still running and unclaimed.
    fn live_prepare(&self) -> bool {
        self.preparing.load(Ordering::SeqCst)
            && !self.cancel.load(Ordering::SeqCst)
            && !self.finished.load(Ordering::SeqCst)
    }

    fn same_media(&self, ctx: &StartCtx) -> bool {
        self.profile_id == ctx.profile_id
            && self.media_kind == ctx.media_kind
            && self.media_id == ctx.media_id
            && self.track_slot == ctx.audio_track.unwrap_or(0) as i64
    }

    /// Hand the session to the player session `session_id`, which plays
    /// from `from_sec`: its events, priority, range and the duration it knows
    /// become the session's.
    fn adopt_by(&self, session_id: &str, from_sec: f64, duration_sec: Option<f64>) {
        self.ident.set(session_id);
        if duration_cs(duration_sec) != NO_HORIZON {
            self.duration_cs
                .store(duration_cs(duration_sec), Ordering::SeqCst);
        }
        self.playhead_cs
            .store((from_sec.max(0.0) * 100.0) as u64, Ordering::SeqCst);
        self.horizon_cs.store(NO_HORIZON, Ordering::SeqCst);
        self.preparing.store(false, Ordering::SeqCst);
    }

    fn halt(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.model_cancel.cancel();
    }

    /// Work class of the next window: a prepared session must not delay
    /// anything the user is waiting on.
    fn class(&self) -> WorkClass {
        if self.preparing.load(Ordering::SeqCst) {
            WorkClass::Interactive
        } else {
            WorkClass::Realtime
        }
    }
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
    /// Set by the manager; events read the session id through it.
    pub ident: SessionIdent,
}

impl CaptionSessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a session, cancelling any previous one, or adopt the session a
    /// title page prepared for the same media and audio track.
    pub async fn start(&self, ctx: StartCtx) {
        ctx.ident.set(&ctx.session_id);
        let handle = {
            let mut guard = self.active.lock().await;
            if let Some(prev) = guard.as_ref().filter(|h| h.adoptable_by(&ctx)) {
                let prev = Arc::clone(prev);
                drop(guard);
                adopt(&prev, &ctx).await;
                return;
            }
            if let Some(prev) = guard.take() {
                prev.halt();
            }
            let handle = Arc::new(SessionHandle::new(&ctx, false));
            *guard = Some(Arc::clone(&handle));
            handle
        };
        tokio::spawn(run_session(ctx, handle));
    }

    /// Transcribe the opening of a title before it plays (ADR-0014). Runs
    /// only while no player session exists; a later [`Self::start`] for the
    /// same media adopts it. Never downloads a model.
    pub async fn prepare(&self, ctx: StartCtx) {
        ctx.ident.set(&ctx.session_id);
        let handle = {
            let mut guard = self.active.lock().await;
            if self.take_cancelled(&ctx.session_id) {
                return;
            }
            match guard.as_ref() {
                Some(h) if !h.preparing.load(Ordering::SeqCst) => return,
                Some(h) if h.adoptable_by(&ctx) => return,
                _ => {}
            }
            if let Some(prev) = guard.take() {
                prev.halt();
            }
            let handle = Arc::new(SessionHandle::new(&ctx, true));
            *guard = Some(Arc::clone(&handle));
            handle
        };
        tokio::spawn(run_session(ctx, handle));
    }

    /// Stop a prepared session that no player adopted (the title page closed).
    pub async fn cancel_prepare(&self, session_id: &str) {
        let mut guard = self.active.lock().await;
        if guard
            .as_ref()
            .is_some_and(|h| h.preparing.load(Ordering::SeqCst) && h.ident.get() == session_id)
        {
            if let Some(h) = guard.take() {
                h.halt();
            }
        } else if guard.as_ref().is_none_or(|h| h.ident.get() != session_id) {
            // Not started yet (or already replaced): remember it, so the
            // prepare that is still on its way does not start.
            let mut cancelled = self
                .cancelled_prepares
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if cancelled.len() == CANCELLED_PREPARES_KEPT {
                cancelled.pop_front();
            }
            cancelled.push_back(session_id.to_string());
        }
    }

    fn take_cancelled(&self, session_id: &str) -> bool {
        let mut cancelled = self
            .cancelled_prepares
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match cancelled.iter().position(|id| id == session_id) {
            Some(i) => {
                cancelled.remove(i);
                true
            }
            None => false,
        }
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
            h.halt();
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
                h.ident.get() == session_id
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
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct CaptionStatusEvent {
    session_id: String,
    media_kind: String,
    #[cfg_attr(test, ts(type = "number"))]
    media_id: i64,
    /// `preparing` | `streaming` | `complete` | `failed`
    state: String,
    phase: Option<String>,
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

async fn run_session(ctx: StartCtx, handle: Arc<SessionHandle>) {
    let result = run_session_inner(&ctx, &handle).await;
    handle.finished.store(true, Ordering::SeqCst);
    if let Err(e) = result {
        if !handle.cancel.load(Ordering::SeqCst) {
            tracing::warn!(error = %e, "caption session failed");
            emit_status(&ctx, "failed", None, Some(e));
        }
    }
}

fn duration_cs(duration_sec: Option<f64>) -> u64 {
    duration_sec
        .filter(|d| d.is_finite() && *d > 0.0)
        .map_or(NO_HORIZON, |d| (d * 100.0) as u64)
}

/// Hand a prepared session to the player that asked for the same media: it
/// keeps its coverage and in-flight window, and its cues so far are replayed
/// under the player's session id.
async fn adopt(handle: &SessionHandle, ctx: &StartCtx) {
    handle.adopt_by(&ctx.session_id, ctx.from_sec, ctx.duration_sec);
    let id = handle.transcript_id.load(Ordering::SeqCst);
    if id > 0 {
        let language = handle
            .language
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Ok(stored) = TranscriptRepository::new(&ctx.pool)
            .segments_with_ids(id)
            .await
        {
            tracing::debug!(
                transcript_id = id,
                cues = stored.len(),
                "adopted prepared captions"
            );
            emit_cues(ctx, id, &language, &stored);
        }
    } else {
        tracing::debug!("adopted prepared captions before their first speech");
    }
    emit_phase(ctx, "streaming", "Captions prepared before playback", None);
}

async fn run_session_inner(ctx: &StartCtx, handle: &SessionHandle) -> Result<(), String> {
    let cancel = handle.cancel.as_ref();
    let model_cancel = &handle.model_cancel;
    let playhead_cs = handle.playhead_cs.as_ref();
    let active_transcript = handle.transcript_id.as_ref();
    emit_phase(ctx, "preparing", "Preparing model", None);

    // Prefer a registered available location; retain the previous internal
    // download behavior when no local copy exists. A prepared session never
    // downloads: it only uses a model that is already there.
    let model = default_model();
    match ctx
        .runtime
        .acquire_at(&ctx.model_root, model.id, handle.class(), model_cancel)
        .await
    {
        Ok(_lease) => {}
        Err(_) if handle.preparing.load(Ordering::SeqCst) => return Ok(()),
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
        .or_else(|| resolved.source_lang.clone())
        .as_deref()
        .and_then(normalize_whisper_lang);

    let repo = TranscriptRepository::new(&ctx.pool);
    let mut duration = ctx
        .duration_sec
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(f64::INFINITY);

    // Load any cached transcript for this track and replay its cues immediately.
    let mut transcript_id: Option<i64> = None;
    let mut language = lang_hint.clone().unwrap_or_default();
    let mut language_confirmed = false;
    let mut covered: Vec<(f64, f64)> = Vec::new();
    let mut prompt_tail: Option<String> = None;
    if let Some(existing) = repo
        .get_for_media_track(&content_key, &ctx.media_kind, ctx.media_id, track_slot)
        .await
        .map_err(|e| e.to_string())?
    {
        transcript_id = Some(existing.id);
        active_transcript.store(existing.id, Ordering::SeqCst);
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
        language = cached_speech_language(
            &existing.language,
            !segments.is_empty(),
            lang_hint.as_deref(),
        );
        if !segments.is_empty() {
            language_confirmed = true;
            set_language(handle, &language);
            prompt_tail = tail_prompt(&segments);
            let stored = repo
                .segments_with_ids(existing.id)
                .await
                .map_err(|e| e.to_string())?;
            emit_cues(ctx, existing.id, &language, &stored);
        }
    }

    emit_status(ctx, "streaming", Some(covered_total(&covered)), None);

    let mut failures = WindowFailures::default();
    let mut candidate_eof: Option<f64> = None;
    let mut use_worker = true;
    let mut first_window = true;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        let playhead = playhead_cs.load(Ordering::SeqCst) as f64 / 100.0;
        // A player that adopted a prepared session may know the duration
        // the title page did not.
        if !duration.is_finite() {
            if let Some(known) = handle.duration() {
                duration = known;
            }
        }
        let horizon = match handle.horizon_cs.load(Ordering::SeqCst) {
            NO_HORIZON => f64::INFINITY,
            cs => cs as f64 / 100.0,
        };

        let limit = (playhead + PREFETCH_AHEAD_SEC).min(duration).min(horizon);
        let Some(gap_start) = next_gap(&covered, playhead, limit) else {
            // Buffered far enough ahead, or everything in range is covered.
            if covers_to(&covered, duration) {
                if let Some(id) = transcript_id {
                    let _ = repo.mark_complete(id).await;
                }
                emit_status(ctx, "complete", Some(covered_total(&covered)), None);
                return Ok(());
            }
            // A prepared session idles at its horizon until a player adopts
            // it or its title page closes.
            tokio::time::sleep(Duration::from_millis(400)).await;
            continue;
        };

        // Extract a window starting at the gap, with a short lead-in for context.
        let work = tempfile::Builder::new()
            .prefix("nova-asr-win-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let wav = work.path().join("win.wav");
        // The first window after a (re)start is short, so the first caption
        // only waits for a few seconds of audio.
        let span = if first_window && !point_covered(&covered, gap_start) {
            FIRST_WINDOW_SEC
        } else {
            WINDOW_SEC
        };
        let (audio_start, window_end, audio_dur) = window_bounds(gap_start, duration, span);

        emit_phase(
            ctx,
            "streaming",
            "Extracting audio",
            Some(covered_total(&covered)),
        );
        let extracted = tokio::select! {
            biased;
            _ = model_cancel.cancelled() => return Ok(()),
            result = extract_audio_window(
                &resolved.upstream_url,
                &wav,
                audio_start,
                audio_dur,
                ctx.audio_track,
                EXTRACT_DEADLINE,
            ) => result,
        };
        let decoded_sec = match extracted {
            Ok(seconds) => seconds,
            Err(error) => {
                emit_phase(
                    ctx,
                    "streaming",
                    "Retrying audio extraction",
                    Some(covered_total(&covered)),
                );
                if retry_window(
                    &mut failures,
                    gap_start,
                    format!("audio extraction: {error}"),
                    model_cancel,
                )
                .await?
                {
                    continue;
                }
                return Ok(());
            }
        };
        if decoded_sec <= EPS {
            // Successful demux with an empty data chunk is EOF only after
            // reaching an established endpoint. An empty first window or early
            // seek on a declared finite movie is not successful caption work.
            if point_covered(&covered, audio_start)
                && (!duration.is_finite() || audio_start >= duration - EOF_SHORTFALL_SEC)
            {
                if covers_to(&covered, duration.min(gap_start)) {
                    if let Some(id) = transcript_id {
                        let _ = repo.mark_complete(id).await;
                    }
                }
                emit_status(ctx, "complete", Some(covered_total(&covered)), None);
                return Ok(());
            }
            emit_phase(
                ctx,
                "streaming",
                "Retrying empty audio",
                Some(covered_total(&covered)),
            );
            if retry_window(
                &mut failures,
                gap_start,
                "No audio samples decoded at the requested position".into(),
                model_cancel,
            )
            .await?
            {
                continue;
            }
            return Ok(());
        }
        let decoded_end = audio_start + decoded_sec;
        let short_window = decoded_sec + EPS < audio_dur;
        let final_window = duration.is_finite() && window_end >= duration - EPS;
        if short_window {
            // A partial upstream response in the middle of a known movie is
            // not EOF. Unknown-length media requires repeatable decoded EOF
            // evidence, not the last speech timestamp or a few silent windows.
            if short_window_needs_retry(duration, window_end, decoded_end, candidate_eof) {
                candidate_eof = Some(decoded_end);
                emit_phase(
                    ctx,
                    "streaming",
                    "Retrying incomplete audio window",
                    Some(covered_total(&covered)),
                );
                if retry_window(
                    &mut failures,
                    gap_start,
                    "Decoded audio ended before the requested window".into(),
                    model_cancel,
                )
                .await?
                {
                    continue;
                }
                return Ok(());
            }
        }
        let reached_eof = short_window || final_window;
        if short_window && (!duration.is_finite() || final_window) {
            duration = duration.min(decoded_end);
        }
        candidate_eof = None;
        let window_end = window_end.min(decoded_end);

        let opts = TranscribeOptions {
            language: normalize_whisper_lang(&language).or_else(|| lang_hint.clone()),
            model_id: model.id.to_string(),
            dtw_preset: model.dtw_preset.to_string(),
            initial_prompt: prompt_tail.clone(),
            max_len: MAX_LEN_CHARS,
        };
        emit_phase(
            ctx,
            "streaming",
            "Waiting for model",
            Some(covered_total(&covered)),
        );
        let class = handle.class();
        let lease = ctx
            .runtime
            .acquire_at(&ctx.model_root, model.id, class, model_cancel)
            .await
            .map_err(|e| e.to_string())?;
        emit_phase(
            ctx,
            "streaming",
            "Recognizing and aligning speech",
            Some(covered_total(&covered)),
        );
        let window = Window {
            work,
            wav,
            audio_start,
        };
        let result = recognize(
            ctx,
            &model,
            lease,
            class,
            window,
            opts,
            &mut use_worker,
            model_cancel,
        )
        .await;
        let raw = match result {
            Ok(t) => t,
            Err(e) => {
                emit_phase(
                    ctx,
                    "streaming",
                    "Retrying speech recognition",
                    Some(covered_total(&covered)),
                );
                if retry_window(
                    &mut failures,
                    gap_start,
                    format!("whisper: {e}"),
                    model_cancel,
                )
                .await?
                {
                    continue;
                }
                return Ok(());
            }
        };
        let mut raw = raw;
        first_window = false;
        failures = WindowFailures::default();
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }

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
        let (keep, advance_to) = split_complete(&raw.segments, gap_start, window_end, reached_eof);

        // Whisper may guess a language even for silence. Learn it only from
        // confirmed speech; keep silent coverage in memory until then.
        if !language_confirmed && !keep.is_empty() {
            language = speech_language(&raw.language, lang_hint.as_deref());
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
            repo.confirm_empty_language(
                id,
                &language,
                &vtt_path.to_string_lossy(),
                &json_path.to_string_lossy(),
            )
            .await
            .map_err(|e| e.to_string())?;
            for &(start, end) in &covered {
                repo.record_window(id, start, end)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            language_confirmed = true;
            set_language(handle, &language);
        }

        if !keep.is_empty() {
            // Coverage and stitching above work on where speech ends; the
            // stored cues also get time to be read.
            let mut keep = keep;
            nova_align::pad_for_reading(&mut keep);
            let id = transcript_id.expect("confirmed speech has a transcript");
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
        mark_covered(&repo, &mut covered, transcript_id, gap_start, advance_to).await;

        emit_status(ctx, "streaming", Some(covered_total(&covered)), None);
    }
}

fn speech_language(detected: &str, hint: Option<&str>) -> String {
    hint.and_then(normalize_whisper_lang)
        .or_else(|| normalize_whisper_lang(detected))
        .unwrap_or_else(|| "und".into())
}

fn cached_speech_language(stored: &str, has_speech: bool, hint: Option<&str>) -> String {
    if has_speech {
        stored.into()
    } else {
        hint.and_then(normalize_whisper_lang).unwrap_or_default()
    }
}

fn window_bounds(gap_start: f64, duration: f64, span: f64) -> (f64, f64, f64) {
    let audio_start = (gap_start - OVERLAP_SEC).max(0.0);
    let window_end = (gap_start + span).min(duration);
    (audio_start, window_end, window_end - audio_start)
}

fn set_language(handle: &SessionHandle, language: &str) {
    *handle.language.lock().unwrap_or_else(|p| p.into_inner()) = language.to_string();
}

/// One extracted audio window. `work` owns the temporary directory.
struct Window {
    work: tempfile::TempDir,
    wav: PathBuf,
    /// Media time of the window's first sample.
    audio_start: f64,
}

/// The caption timing models available right now, leased for one decode so
/// they cannot be removed underneath it. Missing or disabled ones are left
/// out: timing then falls back to Whisper's own (DTW) word times.
async fn timing_models(
    ctx: &StartCtx,
    class: WorkClass,
    cancel: &RuntimeCancellation,
) -> (Vec<ModelLease>, Option<TimingModels>) {
    let Ok(manifest) = bundled_manifest() else {
        return (Vec::new(), None);
    };
    let mut leases = Vec::new();
    let mut models = TimingModels::default();
    for capability in [Capability::Alignment, Capability::VoiceActivity] {
        let Some(entry) = manifest.models.iter().find(|e| e.capability == capability) else {
            continue;
        };
        let Ok(lease) = ctx
            .runtime
            .acquire_at(&ctx.model_root, &entry.id, class, cancel)
            .await
        else {
            continue;
        };
        let Some(dir) = lease.companion_dir().map(Path::to_path_buf) else {
            continue;
        };
        match capability {
            Capability::Alignment => models.aligner = Some(dir),
            _ => models.vad = Some(dir),
        }
        leases.push(lease);
    }
    let any = models.aligner.is_some() || models.vad.is_some();
    (leases, any.then_some(models))
}

/// Recognize one window, in absolute media time. The resident worker is
/// preferred (no model load per window, caption timing stage); when it
/// cannot start, this and later windows use `whisper-cli`.
#[allow(clippy::too_many_arguments)]
async fn recognize(
    ctx: &StartCtx,
    model: &WhisperModel,
    lease: ModelLease,
    class: WorkClass,
    window: Window,
    opts: TranscribeOptions,
    use_worker: &mut bool,
    cancel: &RuntimeCancellation,
) -> Result<RawTranscript, String> {
    if *use_worker {
        let pcm = read_wav_16k_mono(&window.wav)
            .await
            .map_err(|e| format!("reading audio: {e}"))?;
        let language = opts.language.as_deref().and_then(normalize_whisper_lang);
        let (timing_leases, timing) = timing_models(ctx, class, cancel).await;
        let request = DecodeRequest {
            samples: pcm.len(),
            offset_sec: window.audio_start,
            // Arabic decodes without prompt history (see whisper.rs).
            prompt: opts
                .initial_prompt
                .clone()
                .filter(|_| language.as_deref() != Some("ar"))
                .unwrap_or_default(),
            language: language.clone(),
            max_len: MAX_LEN_CHARS,
            timing,
        };
        let decoded = lease
            .run_blocking_at(class, cancel, move |lease| {
                let _timing_leases = timing_leases;
                lease
                    .speech_engine()
                    .map(|engine| engine.decode(&request, &pcm))
            })
            .await
            .map_err(|e| e.to_string())?;
        match decoded {
            Ok(Ok(decoded)) => {
                if let Some(report) = &decoded.timing {
                    tracing::debug!(
                        aligned = report.aligned_words,
                        interpolated = report.interpolated_words,
                        dropped = report.dropped_segments,
                        skipped_silence = report.skipped_silence,
                        timing_ms = report.timing_ms,
                        decode_ms = decoded.decode_ms,
                        error = report.error.as_deref().unwrap_or(""),
                        "caption window timed"
                    );
                }
                return Ok(RawTranscript {
                    language: decoded
                        .language
                        .clone()
                        .or(language)
                        .unwrap_or_else(|| "und".into()),
                    model: model.id.to_string(),
                    segments: segments_of(&decoded.words),
                    duration_processed_sec: None,
                });
            }
            // The worker started but this decode failed: the next attempt
            // gets a fresh worker.
            Ok(Err(e)) => return Err(e.to_string()),
            Err(e) => {
                tracing::warn!(error = %e, "ASR worker unavailable; using whisper-cli");
                *use_worker = false;
            }
        }
        let lease = ctx
            .runtime
            .acquire_at(&ctx.model_root, model.id, class, cancel)
            .await
            .map_err(|e| e.to_string())?;
        return recognize_cli(lease, class, window, opts, cancel).await;
    }
    recognize_cli(lease, class, window, opts, cancel).await
}

async fn recognize_cli(
    lease: ModelLease,
    class: WorkClass,
    window: Window,
    opts: TranscribeOptions,
    cancel: &RuntimeCancellation,
) -> Result<RawTranscript, String> {
    let child_cancel = cancel.clone();
    let audio_start = window.audio_start;
    let mut raw = lease
        .run_async_at(class, cancel, move |lease| async move {
            let Window { work, wav, .. } = window;
            let out_base = work.path().join("win");
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
        .map_err(|e| e.to_string())??;
    shift_transcript(&mut raw, audio_start);
    Ok(raw)
}

fn short_window_needs_retry(
    duration: f64,
    _window_end: f64,
    decoded_end: f64,
    candidate_eof: Option<f64>,
) -> bool {
    if duration.is_finite() {
        // Audio can legitimately finish slightly before the declared video
        // tail. Anything materially earlier is incomplete extraction, even
        // when the requested span was the final window.
        decoded_end < duration - EOF_SHORTFALL_SEC
    } else {
        candidate_eof.is_none_or(|end| (end - decoded_end).abs() > EPS)
    }
}

/// Failures leave the gap untouched, so transient upstream/model failures can
/// recover and a later session can retry after the bounded attempt budget.
#[derive(Default)]
struct WindowFailures {
    gap: Option<f64>,
    attempts: u8,
}

impl WindowFailures {
    fn record(&mut self, gap: f64) -> bool {
        if self.gap.is_none_or(|previous| (previous - gap).abs() > EPS) {
            self.gap = Some(gap);
            self.attempts = 0;
        }
        self.attempts += 1;
        self.attempts < 3
    }
}

async fn retry_window(
    failures: &mut WindowFailures,
    gap: f64,
    error: String,
    cancellation: &RuntimeCancellation,
) -> Result<bool, String> {
    if cancellation.is_cancelled() {
        return Ok(false);
    }
    if !failures.record(gap) {
        return Err(format!(
            "{error} (after 3 attempts at {gap:.1}s; this gap remains retryable)"
        ));
    }
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Ok(false),
        _ = tokio::time::sleep(Duration::from_millis(500 * u64::from(failures.attempts))) => Ok(true),
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
    reached_eof: bool,
) -> (Vec<TranscriptSegment>, f64) {
    if segments.is_empty() {
        // Nothing here (silence / extraction gap): cover the whole span.
        return (Vec::new(), window_end);
    }
    let max_end = segments.iter().fold(0.0_f64, |m, s| m.max(s.end_sec));
    if reached_eof {
        return (segments.to_vec(), window_end);
    }

    let cut = window_end - TRAILING_GUARD_SEC;
    let complete: Vec<TranscriptSegment> = segments
        .iter()
        .filter(|s| s.end_sec <= cut)
        .cloned()
        .collect();
    if complete.len() == segments.len() {
        // The decoded remainder is silence, not EOF. It is safe coverage and
        // need not be extracted/recognized again on the next window.
        (complete, window_end)
    } else if let Some(last) = complete.last() {
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
            session_id: ctx.ident.get(),
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
        CaptionStatusEvent {
            session_id: ctx.ident.get(),
            media_kind: ctx.media_kind.clone(),
            media_id: ctx.media_id,
            state: state.to_string(),
            phase: None,
            covered_sec,
            error,
        },
    );
}

fn emit_phase(ctx: &StartCtx, state: &str, phase: &str, covered_sec: Option<f64>) {
    let _ = ctx.app.emit(
        "transcription:status",
        CaptionStatusEvent {
            session_id: ctx.ident.get(),
            media_kind: ctx.media_kind.clone(),
            media_id: ctx.media_id,
            state: state.into(),
            phase: Some(phase.into()),
            covered_sec,
            error: None,
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

    #[test]
    fn empty_cached_windows_do_not_force_a_guessed_language_onto_later_speech() {
        let language = cached_speech_language("en", false, None);
        assert_eq!(normalize_whisper_lang(&language), None);
        assert_eq!(speech_language("ar", None), "ar");
        assert_eq!(cached_speech_language("en", false, Some("ar_EG")), "ar");
        assert_eq!(cached_speech_language("fr", true, None), "fr");
        assert_eq!(speech_language("en", Some("ar_EG")), "ar");
    }

    fn handle(session: &str, media_id: i64, preparing: bool) -> SessionHandle {
        let ident = SessionIdent::default();
        ident.set(session);
        SessionHandle {
            ident,
            profile_id: "profile".into(),
            media_kind: "vod".into(),
            media_id,
            track_slot: 0,
            transcript_id: Arc::new(AtomicI64::new(7)),
            translation_slot: Arc::new(Semaphore::new(1)),
            cancel: Arc::new(AtomicBool::new(false)),
            model_cancel: RuntimeCancellation::default(),
            playhead_cs: Arc::new(AtomicU64::new(6_000)),
            preparing: AtomicBool::new(preparing),
            horizon_cs: AtomicU64::new(if preparing { 10_500 } else { NO_HORIZON }),
            language: std::sync::Mutex::new(String::new()),
            finished: AtomicBool::new(false),
            duration_cs: AtomicU64::new(NO_HORIZON),
        }
    }

    #[tokio::test]
    async fn an_adopted_session_belongs_to_the_player() {
        let manager = CaptionSessionManager::new();
        let prepared = Arc::new(handle("page", 7, true));
        assert_eq!(prepared.class(), WorkClass::Interactive);
        *manager.active.lock().await = Some(Arc::clone(&prepared));

        assert_eq!(prepared.duration(), None);
        prepared.adopt_by("player", 61.5, Some(734.08));
        assert_eq!(prepared.duration(), Some(734.08));
        assert_eq!(prepared.ident.get(), "player");
        assert_eq!(prepared.class(), WorkClass::Realtime);
        assert_eq!(prepared.horizon_cs.load(Ordering::SeqCst), NO_HORIZON);
        assert_eq!(prepared.playhead_cs.load(Ordering::SeqCst), 6_150);
        // Translation follows the player's id, and the page can no longer
        // cancel what the player now owns.
        assert!(manager
            .translation_context("player", "profile", 7)
            .await
            .is_some());
        manager.cancel_prepare("page").await;
        manager.cancel_prepare("player").await;
        assert!(!prepared.cancel.load(Ordering::SeqCst));
        assert!(manager.active.lock().await.is_some());
    }

    #[test]
    fn a_prepared_session_that_already_exited_cannot_be_adopted() {
        let prepared = handle("page", 7, true);
        assert!(prepared.live_prepare());
        // e.g. the Whisper model was missing: the loop returned at once.
        prepared.finished.store(true, Ordering::SeqCst);
        assert!(!prepared.live_prepare());
        let halted = handle("page", 7, true);
        halted.halt();
        assert!(!halted.live_prepare());
    }

    #[tokio::test]
    async fn a_cancel_that_overtakes_its_prepare_is_remembered() {
        let manager = CaptionSessionManager::new();
        manager.cancel_prepare("page").await;
        assert!(
            manager.take_cancelled("page"),
            "the late prepare is refused"
        );
        assert!(!manager.take_cancelled("page"), "once");
        // A player session that adopted the id is not a pending prepare.
        *manager.active.lock().await = Some(Arc::new(handle("player", 7, false)));
        manager.cancel_prepare("player").await;
        assert!(!manager.take_cancelled("player"));
        for i in 0..40 {
            manager.cancel_prepare(&format!("p{i}")).await;
        }
        assert!(!manager.take_cancelled("p0"), "bounded memory");
        assert!(manager.take_cancelled("p39"));
    }

    #[tokio::test]
    async fn only_the_page_that_prepared_a_session_cancels_it() {
        let manager = CaptionSessionManager::new();
        let prepared = Arc::new(handle("page", 7, true));
        *manager.active.lock().await = Some(Arc::clone(&prepared));
        manager.cancel_prepare("another page").await;
        assert!(!prepared.cancel.load(Ordering::SeqCst));
        manager.cancel_prepare("page").await;
        assert!(prepared.cancel.load(Ordering::SeqCst));
        assert!(prepared.model_cancel.is_cancelled());
        assert!(manager.active.lock().await.is_none());

        let playing = Arc::new(handle("player", 7, false));
        *manager.active.lock().await = Some(Arc::clone(&playing));
        manager.cancel_prepare("player").await;
        assert!(
            !playing.cancel.load(Ordering::SeqCst),
            "a playing session is not a prepare"
        );
    }

    #[tokio::test]
    async fn translation_is_scoped_bounded_and_cancelled_with_the_caption_session() {
        let manager = CaptionSessionManager::new();
        *manager.active.lock().await = Some(Arc::new(handle("session", 7, false)));
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
    fn final_and_late_windows_are_bounded_by_the_actual_media_duration() {
        assert_eq!(
            window_bounds(7_185.0, 7_200.0, WINDOW_SEC),
            (7_182.0, 7_200.0, 18.0)
        );
        assert_eq!(
            window_bounds(7_185.0, f64::INFINITY, WINDOW_SEC),
            (7_182.0, 7_212.0, 30.0),
            "lead-in plus window is whisper's 30 s context"
        );
        assert_eq!(window_bounds(0.0, 11.0, WINDOW_SEC), (0.0, 11.0, 11.0));
        assert_eq!(
            window_bounds(600.0, f64::INFINITY, FIRST_WINDOW_SEC),
            (597.0, 608.0, 11.0)
        );
    }

    #[test]
    fn partial_upstream_audio_cannot_truncate_a_known_movie() {
        assert!(short_window_needs_retry(7_200.0, 63.0, 42.0, None));
        assert!(
            short_window_needs_retry(7_200.0, 63.0, 42.0, Some(42.0)),
            "repeatable early short data still is not a declared finite EOF"
        );
        assert!(
            short_window_needs_retry(7_200.0, 7_200.0, 7_183.0, None),
            "a partial final window cannot discard seventeen seconds"
        );
        assert!(!short_window_needs_retry(7_200.0, 7_200.0, 7_199.5, None));
        assert!(short_window_needs_retry(f64::INFINITY, 63.0, 42.0, None));
        assert!(short_window_needs_retry(
            f64::INFINITY,
            63.0,
            42.0,
            Some(43.0)
        ));
        assert!(!short_window_needs_retry(
            f64::INFINITY,
            63.0,
            42.0,
            Some(42.0)
        ));
    }

    #[test]
    fn silent_windows_remain_decoded_coverage_without_implying_eof() {
        let mut covered = Vec::new();
        for k in 0..4 {
            let start = k as f64 * WINDOW_SEC;
            let (_, end) = split_complete(&[], start, start + WINDOW_SEC, false);
            merge_interval(&mut covered, start, end);
        }
        let end = 4.0 * WINDOW_SEC;
        assert_eq!(covered, vec![(0.0, end)]);
        assert!(!covers_to(&covered, f64::INFINITY));
        assert_eq!(next_gap(&covered, end, 210.0), Some(end));
        let (_, end) = split_complete(
            &[seg(120.0, 125.0, "dialogue after music")],
            120.0,
            150.0,
            false,
        );
        assert_eq!(end, 150.0, "decoded silence after speech is safe coverage");
    }

    #[tokio::test]
    async fn failures_retry_without_covering_the_gap_and_cancel_during_backoff() {
        let coverage = vec![(0.0, 30.0)];
        let mut failures = WindowFailures::default();
        assert!(failures.record(30.0));
        assert!(failures.record(30.0));
        assert!(
            !failures.record(30.0),
            "three failed attempts exhaust the budget"
        );
        assert_eq!(next_gap(&coverage, 0.0, 90.0), Some(30.0));
        assert!(failures.record(60.0), "a new gap has a separate budget");
        let cancel = RuntimeCancellation::default();
        let other = cancel.clone();
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            other.cancel();
        });
        let started = std::time::Instant::now();
        assert!(
            !retry_window(&mut failures, 90.0, "transient failure".into(), &cancel)
                .await
                .unwrap()
        );
        cancel_task.await.unwrap();
        assert!(started.elapsed() < Duration::from_millis(400));
        assert_eq!(next_gap(&coverage, 0.0, 90.0), Some(30.0));
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
        let (keep, advance) = split_complete(&segs, gap, window_end, false);
        assert_eq!(keep.len(), 2, "trailing truncated segment dropped");
        assert!((advance - 24.0).abs() < 1e-9, "cursor at last complete end");
    }

    #[test]
    fn split_complete_keeps_all_at_decoded_eof() {
        // Actual decoded PCM ends at 11s; speech ends earlier at 9s.
        let segs = [seg(0.5, 5.0, "a"), seg(5.2, 9.0, "b")];
        let (keep, advance) = split_complete(&segs, 0.0, 11.0, true);
        assert_eq!(keep.len(), 2);
        assert!((advance - 11.0).abs() < 1e-9);
    }

    #[test]
    fn split_complete_forces_progress_on_one_long_segment() {
        // A single segment spans the whole window with no clean break.
        let segs = [seg(0.2, 29.8, "an unbroken monologue")];
        let (keep, advance) = split_complete(&segs, 0.0, 30.0, false);
        assert_eq!(keep.len(), 1);
        assert!(advance >= MIN_ADVANCE_SEC);
    }

    #[test]
    fn split_complete_empty_covers_whole_span() {
        let (keep, advance) = split_complete(&[], 10.0, 40.0, false);
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
