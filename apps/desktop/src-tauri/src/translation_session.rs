//! Streaming translation session (Phase 7).
//!
//! Mirrors the caption session, but instead of transcribing it **consumes the
//! source segments the caption session persists** (`transcript_segments`),
//! translates the ones at/ahead of the playhead with NLLB-200, caches each in
//! `transcript_translations`, and emits `translation:cues` — the same event
//! shape the source captions use, so the React `CaptionOverlay` renders the
//! middle-box "translation card" with no extra plumbing.
//!
//! It depends on the source caption session running (translation/dual mode runs
//! both): when the source hasn't reached the playhead yet, this polls and waits.
//! Cached translations replay instantly on (re)start, so re-watching is free.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nova_storage::{
    PlaybackRepository, SourceSegment, TranscriptRepository, TranslationRepository, TranslationRow,
};
use nova_translate::{model as nllb_model, to_nllb};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::caption_session::content_key;
use crate::model_runtime::{ModelRuntimeManager, RuntimeCancellation, RuntimeError};
use crate::resource_policy::WorkClass;

/// Stop translating once this far ahead of the playhead; resume as it catches up.
const PREFETCH_AHEAD_SEC: f64 = 90.0;
/// Translate segments whose end is at least this far behind the playhead, so a
/// line that just appeared on screen still gets translated.
const LOOKBACK_SEC: f64 = 5.0;
/// Segments per inference call (matches `nllb::MAX_BATCH`).
const BATCH: usize = 32;

#[derive(Default)]
pub struct TranslationSessionManager {
    active: Mutex<Option<Arc<Handle>>>,
}

struct Handle {
    cancel: Arc<AtomicBool>,
    model_cancel: RuntimeCancellation,
    playhead_cs: Arc<AtomicU64>,
}

/// Everything a translation session needs, assembled by the command layer.
pub struct StartCtx {
    pub session_id: String,
    pub app: AppHandle,
    pub pool: Arc<SqlitePool>,
    pub media_kind: String,
    pub media_id: i64,
    pub from_sec: f64,
    pub duration_sec: Option<f64>,
    pub audio_track: Option<u32>,
    /// Reading language (BCP-47) to translate into.
    pub target_lang: String,
    /// NLLB model directory.
    pub models_dir: PathBuf,
    pub model_root: PathBuf,
    pub runtime: Arc<ModelRuntimeManager>,
}

impl TranslationSessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start(&self, ctx: StartCtx) {
        let cancel = Arc::new(AtomicBool::new(false));
        let model_cancel = RuntimeCancellation::default();
        let playhead_cs = Arc::new(AtomicU64::new((ctx.from_sec.max(0.0) * 100.0) as u64));
        let handle = Arc::new(Handle {
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
        tokio::spawn(run_session(ctx, cancel, model_cancel, playhead_cs));
    }

    pub async fn set_playhead(&self, sec: f64) {
        if let Some(h) = self.active.lock().await.as_ref() {
            h.playhead_cs
                .store((sec.max(0.0) * 100.0) as u64, Ordering::SeqCst);
        }
    }

    pub async fn stop(&self) {
        if let Some(h) = self.active.lock().await.take() {
            h.cancel.store(true, Ordering::SeqCst);
            h.model_cancel.cancel();
        }
    }
}

// ── Event payloads (mirror the caption session shapes) ──────────────────────────

#[derive(Debug, Clone, Serialize)]
struct CueDto {
    start: f64,
    end: f64,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CuesEvent {
    session_id: String,
    media_kind: String,
    media_id: i64,
    /// The *target* language (what the cue text is in).
    language: String,
    cues: Vec<CueDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    session_id: String,
    media_kind: String,
    media_id: i64,
    /// `preparing` | `streaming` | `failed`
    state: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDownloadEvent {
    artifact: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

// ── Session loop ────────────────────────────────────────────────────────────────

async fn run_session(
    ctx: StartCtx,
    cancel: Arc<AtomicBool>,
    model_cancel: RuntimeCancellation,
    playhead_cs: Arc<AtomicU64>,
) {
    if let Err(e) = run_session_inner(&ctx, &cancel, &model_cancel, &playhead_cs).await {
        if !cancel.load(Ordering::SeqCst) {
            emit_status(&ctx, "failed", Some(e));
        }
    }
}

async fn run_session_inner(
    ctx: &StartCtx,
    cancel: &AtomicBool,
    model_cancel: &RuntimeCancellation,
    playhead_cs: &AtomicU64,
) -> Result<(), String> {
    emit_status(ctx, "preparing", None);

    // Map the target language up-front; a bad code is a hard, immediate failure.
    let tgt_nllb = to_nllb(&ctx.target_lang)
        .ok_or_else(|| format!("unsupported target language: {}", ctx.target_lang))?;

    // The existing NLLB adapter's persisted backend ID. Replay cached captions
    // without loading a model; obtain a runtime lease only for pending inference.
    let backend = nllb_model::MODEL_ID.to_string();

    // Resolve the source transcript identity (same content_key + track the
    // caption session uses).
    let resolved = PlaybackRepository::new(&ctx.pool)
        .resolve_stream_url(&ctx.media_kind, ctx.media_id)
        .await
        .map_err(|e| format!("resolve stream: {e}"))?;
    let key = content_key(&resolved.upstream_url);
    let track_slot = ctx.audio_track.unwrap_or(0) as i64;
    let repo = TranscriptRepository::new(&ctx.pool);
    let trepo = TranslationRepository::new(&ctx.pool);
    let duration = ctx
        .duration_sec
        .filter(|d| *d > 0.0)
        .unwrap_or(f64::INFINITY);

    emit_status(ctx, "streaming", None);

    // Segment ids we've already emitted a translated cue for (avoids re-emitting
    // the whole cached set every tick).
    let mut emitted: std::collections::HashSet<i64> = std::collections::HashSet::new();

    loop {
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }

        // The caption session may not have created the transcript yet.
        let Some(src) = repo
            .get_for_media_track(&key, &ctx.media_kind, ctx.media_id, track_slot)
            .await
            .map_err(|e| e.to_string())?
        else {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        };

        // Need a known, mappable source language before we can translate.
        let Some(src_nllb) = to_nllb(&src.language) else {
            if src.language.trim().is_empty() {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
            return Err(format!("source language not supported: {}", src.language));
        };

        let segments = repo
            .segments_with_ids(src.id)
            .await
            .map_err(|e| e.to_string())?;
        let cached = trepo
            .cached(src.id, &ctx.target_lang, &backend)
            .await
            .map_err(|e| e.to_string())?;

        // Replay any cached translations we haven't emitted yet (instant on
        // re-watch / restart).
        emit_cached(ctx, &segments, &cached, &mut emitted);

        let playhead = playhead_cs.load(Ordering::SeqCst) as f64 / 100.0;
        let limit = (playhead + PREFETCH_AHEAD_SEC).min(duration);

        // Untranslated segments within the active window, earliest first.
        let pending: Vec<&SourceSegment> = segments
            .iter()
            .filter(|s| !cached.contains_key(&s.id))
            .filter(|s| s.end_sec >= playhead - LOOKBACK_SEC && s.start_sec <= limit)
            .take(BATCH)
            .collect();

        if pending.is_empty() {
            // Caught up to coverage (or buffered far enough); wait for the source
            // to extend or the playhead to advance.
            tokio::time::sleep(Duration::from_millis(400)).await;
            continue;
        }

        let tr = match ctx
            .runtime
            .acquire_at(
                &ctx.model_root,
                nllb_model::MODEL_ID,
                WorkClass::Realtime,
                model_cancel,
            )
            .await
        {
            Ok(lease) => lease,
            Err(RuntimeError::Unavailable) => {
                let app = ctx.app.clone();
                let models_dir = ctx.models_dir.clone();
                ctx.runtime
                    .mutate(nllb_model::MODEL_ID, async move {
                        nllb_model::ensure_model(&models_dir, move |p| {
                            let _ = app.emit(
                                "nllb:download:progress",
                                ModelDownloadEvent {
                                    artifact: p.artifact.to_string(),
                                    downloaded_bytes: p.downloaded_bytes,
                                    total_bytes: p.total_bytes,
                                },
                            );
                        })
                        .await
                        .map_err(|e| format!("model: {e}"))
                    })
                    .await
                    .map_err(|e| e.to_string())??;
                ctx.runtime
                    .acquire_at(
                        &ctx.model_root,
                        nllb_model::MODEL_ID,
                        WorkClass::Realtime,
                        model_cancel,
                    )
                    .await
                    .map_err(|e| e.to_string())?
            }
            Err(e) => return Err(e.to_string()),
        };
        let texts: Vec<String> = pending.iter().map(|s| s.text.clone()).collect();
        let src_code = src_nllb.to_string();
        let tgt_code = tgt_nllb.to_string();
        let translated = tr
            .run_blocking_at(WorkClass::Realtime, model_cancel, move |tr| {
                tr.translator()
                    .ok_or("wrong model backend".to_string())?
                    .translate_batch(&texts, &src_code, &tgt_code)
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| format!("translate task: {e}"))?
            .map_err(|e| format!("translate: {e}"))?;
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }

        let rows: Vec<TranslationRow> = pending
            .iter()
            .zip(&translated)
            .map(|(s, text)| TranslationRow {
                segment_id: s.id,
                target_lang: ctx.target_lang.clone(),
                backend: backend.clone(),
                text: text.clone(),
            })
            .collect();
        trepo.insert_batch(&rows).await.map_err(|e| e.to_string())?;

        // Emit the freshly-translated cues.
        let cues: Vec<CueDto> = pending
            .iter()
            .zip(&translated)
            .map(|(s, text)| {
                emitted.insert(s.id);
                CueDto {
                    start: s.start_sec,
                    end: s.end_sec,
                    text: text.clone(),
                }
            })
            .collect();
        emit_cues(ctx, &ctx.target_lang, cues);
    }
}

fn emit_cached(
    ctx: &StartCtx,
    segments: &[SourceSegment],
    cached: &HashMap<i64, String>,
    emitted: &mut std::collections::HashSet<i64>,
) {
    let mut cues: Vec<CueDto> = Vec::new();
    for s in segments {
        if emitted.contains(&s.id) {
            continue;
        }
        if let Some(text) = cached.get(&s.id) {
            emitted.insert(s.id);
            cues.push(CueDto {
                start: s.start_sec,
                end: s.end_sec,
                text: text.clone(),
            });
        }
    }
    if !cues.is_empty() {
        emit_cues(ctx, &ctx.target_lang, cues);
    }
}

fn emit_cues(ctx: &StartCtx, language: &str, cues: Vec<CueDto>) {
    let _ = ctx.app.emit(
        "translation:cues",
        CuesEvent {
            session_id: ctx.session_id.clone(),
            media_kind: ctx.media_kind.clone(),
            media_id: ctx.media_id,
            language: language.to_string(),
            cues,
        },
    );
}

fn emit_status(ctx: &StartCtx, state: &str, error: Option<String>) {
    let _ = ctx.app.emit(
        "translation:status",
        StatusEvent {
            session_id: ctx.session_id.clone(),
            media_kind: ctx.media_kind.clone(),
            media_id: ctx.media_id,
            state: state.to_string(),
            error,
        },
    );
}
