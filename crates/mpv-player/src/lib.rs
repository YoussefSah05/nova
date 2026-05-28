//! Minimal libmpv playback engine wrapper.
//!
//! Nova owns UI, watch history, captions, and future transcription/translation;
//! libmpv owns hostile media playback. The crate exposes:
//!
//! - [`MpvEngine`]: holds the active session and dispatches commands.
//! - [`MpvRenderContext`]: thin wrapper around `mpv_render_context`. The
//!   embedding code (macOS `CAOpenGLLayer`) constructs one *inside its GL
//!   thread* when the layer first draws, then calls [`MpvRenderContext::render`]
//!   on each subsequent draw.
//! - [`RenderLifecycleHandler`]: trait the host implements so the engine can
//!   tell it "a session opened — attach your render context" / "session is
//!   closing — detach it" *synchronously*. Detach must complete before the
//!   handle is destroyed.
//! - [`EventEmitter`]: trait the host implements to receive a wake-up whenever
//!   libmpv has new events. The host snapshots state and forwards it to the
//!   frontend.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int, c_void};
use std::ptr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum MpvError {
    #[error("libmpv is not available")]
    NotAvailable,
    #[error("invalid string passed to libmpv")]
    InvalidString,
    #[error("libmpv error: {0}")]
    Lib(String),
    #[error("no active native playback session")]
    NoSession,
    #[error("active native playback token does not match")]
    TokenMismatch,
    #[error("native mpv render lifecycle handler is not registered")]
    NoRenderHandler,
    #[error("native mpv render attach failed: {0}")]
    RenderAttach(String),
    #[error("native mpv render context failed: {0}")]
    RenderContext(String),
    #[error("native playback mutex poisoned")]
    Poisoned,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MpvOpenResult {
    pub token: String,
    pub engine: String,
    pub embedded: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MpvPlaybackPhase {
    Opening,
    Buffering,
    Playing,
    Paused,
    Ended,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MpvTrack {
    pub id: i64,
    /// "audio" | "video" | "sub"
    pub kind: String,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MpvPlayerState {
    pub token: Option<String>,
    pub engine: String,
    pub phase: MpvPlaybackPhase,
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    pub volume: f64,
    pub muted: bool,
    pub seekable: bool,
    pub speed: f64,
    pub audio_track_id: Option<i64>,
    pub sub_track_id: Option<i64>,
    pub video_track_id: Option<i64>,
    pub tracks: Vec<MpvTrack>,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct MpvOpenOptions {
    pub url: String,
    pub title: String,
    pub user_agent: Option<String>,
}

/// Target rectangle for the video surface, in CSS pixels relative to the
/// window's top-left. Used by cinema mode to shrink the video into a sub-region
/// (the host converts to its own layer coordinate space). `None` = fill.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Opaque, `Copy` newtype around a raw `*mut mpv_handle`. Crossing the API
/// boundary as a `usize` keeps the FFI declarations private to this crate
/// while still letting the embedding code create a render context against the
/// same handle the engine owns.
///
/// The handle is only valid for the lifetime of the session that issued it —
/// the host must obtain it inside [`RenderLifecycleHandler::attach`] and stop
/// using it once [`RenderLifecycleHandler::detach`] returns.
#[derive(Debug, Clone, Copy)]
pub struct MpvRawHandle(usize);

impl MpvRawHandle {
    fn from_ptr(ptr: *mut mpv_handle) -> Self {
        Self(ptr as usize)
    }

    fn as_ptr(self) -> *mut mpv_handle {
        self.0 as *mut mpv_handle
    }
}

/// Implemented by the host that owns a GL surface (on macOS, the
/// `CAOpenGLLayer` subclass). The engine calls these in lockstep with the
/// session lifecycle so the render context is always created *after* the
/// handle exists and destroyed *before* the handle goes away.
///
/// Both methods are called synchronously from whichever thread opens/closes
/// the session — implementations typically dispatch onto the GL thread
/// internally (e.g. `dispatch_sync` to the main queue on macOS) so libmpv
/// calls happen with a current GL context.
pub trait RenderLifecycleHandler: Send + Sync {
    /// Attach a render context to `handle`. Returning `Err` aborts the session
    /// open.
    fn attach(&self, handle: MpvRawHandle) -> Result<(), String>;

    /// Detach and free the render context. Must complete before the function
    /// returns — the engine destroys the handle immediately afterwards.
    fn detach(&self);

    /// Ask the host to repaint the video surface. Used after a window resize /
    /// fullscreen toggle so a paused frame follows the new bounds. No-op by
    /// default for hosts that don't need it.
    fn request_redraw(&self) {}

    /// Constrain the video surface to `rect` (CSS px, window-relative), or
    /// `None` to fill the window. Used by cinema mode. No-op by default.
    fn set_video_rect(&self, _rect: Option<VideoRect>) {}
}

/// Implemented by the host to receive a notification whenever libmpv signals
/// "new events available." Typically wired to a Tauri event emitter; the host
/// then snapshots [`MpvEngine::state`] and forwards it to the frontend.
///
/// Fired from libmpv's internal thread — implementations must not block.
pub trait EventEmitter: Send + Sync {
    fn wake(&self);
}

#[derive(Default)]
pub struct MpvEngine {
    inner: Mutex<Option<MpvSession>>,
    render_handler: Mutex<Option<Arc<dyn RenderLifecycleHandler>>>,
    event_emitter: Mutex<Option<Arc<dyn EventEmitter>>>,
}

impl MpvEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register the render lifecycle handler. Called once at startup by the
    /// host (e.g. the macOS layer installer).
    pub fn set_render_handler(
        self: &Arc<Self>,
        handler: Arc<dyn RenderLifecycleHandler>,
    ) -> Result<(), MpvError> {
        *self
            .render_handler
            .lock()
            .map_err(|_| MpvError::Poisoned)? = Some(handler);
        Ok(())
    }

    pub fn set_event_emitter(
        self: &Arc<Self>,
        emitter: Arc<dyn EventEmitter>,
    ) -> Result<(), MpvError> {
        *self.event_emitter.lock().map_err(|_| MpvError::Poisoned)? = Some(emitter);
        Ok(())
    }

    pub fn open(self: &Arc<Self>, options: MpvOpenOptions) -> Result<MpvOpenResult, MpvError> {
        let handler = self
            .render_handler
            .lock()
            .map_err(|_| MpvError::Poisoned)?
            .clone()
            .ok_or(MpvError::NoRenderHandler)?;

        let emitter = self
            .event_emitter
            .lock()
            .map_err(|_| MpvError::Poisoned)?
            .clone();

        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;

        // Reuse the live session for content switches: keep the same libmpv
        // handle and attached render context, and swap files with `loadfile
        // replace`. This avoids the destroy/re-attach churn (and stale-frame
        // bleed) that a full teardown-per-switch caused; libmpv flushes the
        // previous file's demuxer/cache and loads the new one cleanly.
        if let Some(session) = guard.as_mut() {
            let token = session.load(&options.url, &options.title)?;
            return Ok(MpvOpenResult {
                token,
                engine: "mpv".to_string(),
                embedded: true,
            });
        }

        let session = MpvSession::open(options, handler, emitter)?;
        let token = session.token.clone();
        *guard = Some(session);

        Ok(MpvOpenResult {
            token,
            engine: "mpv".to_string(),
            embedded: true,
        })
    }

    pub fn close(self: &Arc<Self>, token: &str) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        let Some(session) = guard.as_ref() else {
            return Ok(());
        };
        if session.token != token {
            return Err(MpvError::TokenMismatch);
        }
        guard.take();
        Ok(())
    }

    pub fn state(self: &Arc<Self>, token: Option<&str>) -> Result<MpvPlayerState, MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        let Some(session) = guard.as_mut() else {
            return Ok(MpvPlayerState {
                token: None,
                engine: "mpv".to_string(),
                phase: MpvPlaybackPhase::Ended,
                position_sec: 0.0,
                duration_sec: None,
                volume: 1.0,
                muted: false,
                seekable: false,
                speed: 1.0,
                audio_track_id: None,
                sub_track_id: None,
                video_track_id: None,
                tracks: Vec::new(),
                error: None,
            });
        };
        if let Some(token) = token {
            session.require_token(token)?;
        }
        session.state()
    }

    pub fn toggle_pause(self: &Arc<Self>, token: &str) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        let session = active_session(&mut guard, token)?;
        let paused = session.property_bool("pause").unwrap_or(false);
        session.set_property("pause", if paused { "no" } else { "yes" })
    }

    pub fn seek_relative(self: &Arc<Self>, token: &str, delta_sec: f64) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.command(&[
            "seek",
            &format!("{delta_sec:.3}"),
            "relative",
            "exact",
        ])
    }

    pub fn seek_absolute(self: &Arc<Self>, token: &str, position_sec: f64) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.command(&[
            "seek",
            &format!("{position_sec:.3}"),
            "absolute",
            "exact",
        ])
    }

    pub fn seek_live_edge(self: &Arc<Self>, token: &str) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.command(&["seek", "100", "absolute-percent"])
    }

    pub fn set_volume(self: &Arc<Self>, token: &str, volume: f64) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        let clamped = volume.clamp(0.0, 1.0) * 100.0;
        active_session(&mut guard, token)?.set_property("volume", &format!("{clamped:.1}"))
    }

    pub fn set_mute(self: &Arc<Self>, token: &str, muted: bool) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.set_property("mute", if muted { "yes" } else { "no" })
    }

    pub fn set_audio_track(self: &Arc<Self>, token: &str, id: &str) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.set_property("aid", id)
    }

    pub fn set_subtitle_track(self: &Arc<Self>, token: &str, id: &str) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.set_property("sid", id)
    }

    pub fn set_video_track(self: &Arc<Self>, token: &str, id: &str) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.set_property("vid", id)
    }

    pub fn set_speed(self: &Arc<Self>, token: &str, speed: f64) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        let clamped = speed.clamp(0.25, 4.0);
        active_session(&mut guard, token)?.set_property("speed", &format!("{clamped}"))
    }

    pub fn frame_step(self: &Arc<Self>, token: &str, forward: bool) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        let cmd = if forward { "frame-step" } else { "frame-back-step" };
        active_session(&mut guard, token)?.command(&[cmd])
    }

    /// Ask the render host to repaint (e.g. after a window resize/fullscreen
    /// toggle). No-op if no handler is registered.
    pub fn request_redraw(self: &Arc<Self>) -> Result<(), MpvError> {
        if let Some(handler) = self.render_handler.lock().map_err(|_| MpvError::Poisoned)?.as_ref() {
            handler.request_redraw();
        }
        Ok(())
    }

    /// Constrain the video surface to `rect` (CSS px, window-relative), or
    /// `None` to fill the window. No-op if no handler is registered.
    pub fn set_video_rect(self: &Arc<Self>, rect: Option<VideoRect>) -> Result<(), MpvError> {
        if let Some(handler) = self.render_handler.lock().map_err(|_| MpvError::Poisoned)?.as_ref() {
            handler.set_video_rect(rect);
        }
        Ok(())
    }
}

fn active_session<'a>(
    guard: &'a mut Option<MpvSession>,
    token: &str,
) -> Result<&'a mut MpvSession, MpvError> {
    let session = guard.as_mut().ok_or(MpvError::NoSession)?;
    session.require_token(token)?;
    Ok(session)
}

// -- Session -----------------------------------------------------------------

struct MpvSession {
    handle: *mut mpv_handle,
    token: String,
    last_phase: MpvPlaybackPhase,
    last_error: Option<String>,
    opened_at: Instant,
    handler: Arc<dyn RenderLifecycleHandler>,
    /// Boxed so the address stays stable; libmpv stores this raw pointer for
    /// the lifetime of the handle as the wakeup-callback userdata.
    wakeup_ctx: Option<Box<WakeupCtx>>,
}

// SAFETY: the raw `*mut mpv_handle` is only touched while holding the engine
// mutex (single-threaded access from Rust's perspective). libmpv's own
// internals are thread-safe.
unsafe impl Send for MpvSession {}

impl MpvSession {
    fn open(
        options: MpvOpenOptions,
        handler: Arc<dyn RenderLifecycleHandler>,
        emitter: Option<Arc<dyn EventEmitter>>,
    ) -> Result<Self, MpvError> {
        // SAFETY: `mpv_create` returns either a valid handle or null; we check
        // for null below. No other unsafe state involved.
        let handle = unsafe { mpv_create() };
        if handle.is_null() {
            return Err(MpvError::NotAvailable);
        }

        let mut session = Self {
            handle,
            token: Uuid::new_v4().to_string(),
            last_phase: MpvPlaybackPhase::Opening,
            last_error: None,
            opened_at: Instant::now(),
            handler,
            wakeup_ctx: None,
        };

        // Phase 5 architecture: we always render via the host's render context
        // (CAOpenGLLayer on macOS). libmpv runs with `vo=libmpv` and never
        // creates its own window. Render is driven by the layer's draw cycle.
        session.set_option("vo", "libmpv")?;
        session.set_option("force-window", "no")?;
        session.set_option("keep-open", "yes")?;
        session.set_option("idle", "yes")?;
        session.set_option("terminal", "no")?;
        session.set_option("input-default-bindings", "no")?;
        session.set_option("osc", "no")?;
        session.set_option("osd-level", "0")?;
        session.set_option("hwdec", "auto-safe")?;
        session.set_option("profile", "fast")?;
        session.set_option("cache", "yes")?;
        session.set_option("demuxer-readahead-secs", "20")?;
        session.set_option("demuxer-max-bytes", "150MiB")?;
        session.set_option("hls-bitrate", "max")?;
        if let Some(agent) = options.user_agent.as_deref() {
            session.set_option("user-agent", agent)?;
        }

        // SAFETY: `handle` is the just-created libmpv handle from
        // `mpv_create`, owned by this `MpvSession` for the duration of the
        // call. Init failure surfaces via the return code we check.
        let rc = unsafe { mpv_initialize(handle) };
        session.check(rc)?;

        // Hand the layer a chance to set up its render context against the
        // initialized handle. The contract requires this to be synchronous
        // and to complete before `loadfile` (so the first frame can paint).
        session
            .handler
            .attach(MpvRawHandle::from_ptr(handle))
            .map_err(MpvError::RenderAttach)?;

        // Observe the properties whose changes drive the frontend state.
        // Each observe causes future changes to emit MPV_EVENT_PROPERTY_CHANGE,
        // which wakes the host via the wakeup callback below.
        for prop in [
            "time-pos",
            "duration",
            "pause",
            "paused-for-cache",
            "idle-active",
            "volume",
            "mute",
            "seekable",
            "speed",
            "aid",
            "sid",
            "vid",
            "track-list",
        ] {
            session.observe_property(prop)?;
        }

        // Wire the wakeup callback if the host registered an event emitter.
        if let Some(emitter) = emitter {
            let ctx = Box::new(WakeupCtx { emitter });
            let ctx_ptr: *mut WakeupCtx = Box::into_raw(ctx);
            // SAFETY: `ctx_ptr` outlives the handle — we store it back in
            // `wakeup_ctx` as a `Box` that is dropped *after* we call
            // `mpv_terminate_destroy` (see `Drop`). libmpv guarantees no more
            // callbacks after `mpv_terminate_destroy` returns.
            unsafe {
                mpv_set_wakeup_callback(
                    handle,
                    Some(wakeup_trampoline),
                    ctx_ptr.cast::<c_void>(),
                );
                session.wakeup_ctx = Some(Box::from_raw(ctx_ptr));
            }
        }

        let _ = session.set_property("force-media-title", &options.title);
        session.command(&["loadfile", &options.url, "replace"])?;
        Ok(session)
    }

    /// Swap the playing file on the live session without tearing down the
    /// libmpv handle or render context. Mints a fresh token so the frontend
    /// drops any `mpv:state` still draining from the previous file, and resets
    /// the per-file phase tracking so the new load reports `Opening` first.
    fn load(&mut self, url: &str, title: &str) -> Result<String, MpvError> {
        let token = Uuid::new_v4().to_string();
        self.token = token.clone();
        self.last_phase = MpvPlaybackPhase::Opening;
        self.last_error = None;
        self.opened_at = Instant::now();
        let _ = self.set_property("force-media-title", title);
        self.command(&["loadfile", url, "replace"])?;
        Ok(token)
    }

    fn require_token(&self, token: &str) -> Result<(), MpvError> {
        if self.token == token {
            Ok(())
        } else {
            Err(MpvError::TokenMismatch)
        }
    }

    fn state(&mut self) -> Result<MpvPlayerState, MpvError> {
        self.drain_events();

        let position = self.property_f64("time-pos").unwrap_or(0.0).max(0.0);
        let duration = self
            .property_f64("duration")
            .filter(|v| v.is_finite() && *v > 0.0);
        let volume = self
            .property_f64("volume")
            .unwrap_or(100.0)
            .clamp(0.0, 100.0)
            / 100.0;
        let muted = self.property_bool("mute").unwrap_or(false);
        let paused = self.property_bool("pause").unwrap_or(false);
        let paused_for_cache = self.property_bool("paused-for-cache").unwrap_or(false);
        let idle = self.property_bool("idle-active").unwrap_or(false);
        let seekable = self.property_bool("seekable").unwrap_or(duration.is_some());
        let speed = self.property_f64("speed").unwrap_or(1.0);
        let audio_track_id = self.property_track_id("aid");
        let sub_track_id = self.property_track_id("sid");
        let video_track_id = self.property_track_id("vid");
        let tracks = self.track_list();

        let phase = if self.last_error.is_some() {
            MpvPlaybackPhase::Error
        } else if matches!(self.last_phase, MpvPlaybackPhase::Ended) {
            MpvPlaybackPhase::Ended
        } else if idle && position <= 0.0 && self.opened_at.elapsed().as_secs_f32() < 2.0 {
            MpvPlaybackPhase::Opening
        } else if paused {
            MpvPlaybackPhase::Paused
        } else if paused_for_cache {
            MpvPlaybackPhase::Buffering
        } else {
            MpvPlaybackPhase::Playing
        };
        self.last_phase = phase.clone();

        Ok(MpvPlayerState {
            token: Some(self.token.clone()),
            engine: "mpv".to_string(),
            phase,
            position_sec: position,
            duration_sec: duration,
            volume,
            muted,
            seekable,
            speed,
            audio_track_id,
            sub_track_id,
            video_track_id,
            tracks,
            error: self.last_error.clone(),
        })
    }

    /// Read a track-selection property (`aid`/`sid`/`vid`). libmpv returns the
    /// selected track number, or `"no"`/`"auto"` when nothing is selected.
    fn property_track_id(&self, name: &str) -> Option<i64> {
        self.property_string(name)
            .and_then(|value| value.parse::<i64>().ok())
    }

    /// Build the current track list from libmpv's `track-list` sub-properties.
    /// Reading scalar sub-properties avoids parsing the JSON blob and matches
    /// the existing string-property helpers.
    fn track_list(&self) -> Vec<MpvTrack> {
        let count = self
            .property_string("track-list/count")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        let mut tracks = Vec::with_capacity(count);
        for i in 0..count {
            let Some(id) = self
                .property_string(&format!("track-list/{i}/id"))
                .and_then(|v| v.parse::<i64>().ok())
            else {
                continue;
            };
            let kind = self
                .property_string(&format!("track-list/{i}/type"))
                .unwrap_or_else(|| "unknown".to_string());
            let title = self.property_string(&format!("track-list/{i}/title"));
            let lang = self.property_string(&format!("track-list/{i}/lang"));
            let selected = self
                .property_bool(&format!("track-list/{i}/selected"))
                .unwrap_or(false);
            tracks.push(MpvTrack {
                id,
                kind,
                title,
                lang,
                selected,
            });
        }
        tracks
    }

    fn drain_events(&mut self) {
        loop {
            // SAFETY: `self.handle` is valid for the lifetime of `self`; a
            // zero timeout makes `mpv_wait_event` non-blocking. The returned
            // pointer is owned by libmpv and valid until the next call.
            let event = unsafe { mpv_wait_event(self.handle, 0.0) };
            if event.is_null() {
                break;
            }
            // SAFETY: see above — libmpv-owned, lives until our next call.
            let event = unsafe { &*event };
            match event.event_id {
                MPV_EVENT_NONE => break,
                MPV_EVENT_FILE_LOADED | MPV_EVENT_PLAYBACK_RESTART => {
                    self.last_phase = MpvPlaybackPhase::Playing;
                    self.last_error = None;
                }
                MPV_EVENT_END_FILE if !event.data.is_null() => {
                    // SAFETY: for an END_FILE event libmpv documents `data`
                    // as a pointer to `mpv_event_end_file`.
                    let end = unsafe { &*(event.data as *const mpv_event_end_file) };
                    if end.reason == MPV_END_FILE_REASON_ERROR {
                        self.last_phase = MpvPlaybackPhase::Error;
                        self.last_error = Some(mpv_error_to_string(end.error));
                    } else if end.reason == MPV_END_FILE_REASON_EOF {
                        self.last_phase = MpvPlaybackPhase::Ended;
                    }
                }
                MPV_EVENT_SHUTDOWN => {
                    self.last_phase = MpvPlaybackPhase::Ended;
                    break;
                }
                _ => {}
            }
        }
    }

    fn observe_property(&self, name: &str) -> Result<(), MpvError> {
        let cname = CString::new(name).map_err(|_| MpvError::InvalidString)?;
        // SAFETY: `self.handle` is valid; `cname` outlives the call.
        let rc = unsafe { mpv_observe_property(self.handle, 0, cname.as_ptr(), MPV_FORMAT_NONE) };
        self.check(rc)
    }

    fn set_option(&self, name: &str, value: &str) -> Result<(), MpvError> {
        let name = CString::new(name).map_err(|_| MpvError::InvalidString)?;
        let value = CString::new(value).map_err(|_| MpvError::InvalidString)?;
        // SAFETY: `self.handle` is valid; both CStrings outlive the call.
        let rc = unsafe { mpv_set_option_string(self.handle, name.as_ptr(), value.as_ptr()) };
        self.check(rc)
    }

    fn set_property(&self, name: &str, value: &str) -> Result<(), MpvError> {
        let name = CString::new(name).map_err(|_| MpvError::InvalidString)?;
        let value = CString::new(value).map_err(|_| MpvError::InvalidString)?;
        // SAFETY: `self.handle` is valid; both CStrings outlive the call.
        let rc = unsafe { mpv_set_property_string(self.handle, name.as_ptr(), value.as_ptr()) };
        self.check(rc)
    }

    fn command(&self, args: &[&str]) -> Result<(), MpvError> {
        let cstrings = args
            .iter()
            .map(|arg| CString::new(*arg).map_err(|_| MpvError::InvalidString))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ptrs = cstrings.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();
        ptrs.push(ptr::null());
        // SAFETY: `self.handle` is valid; `ptrs` is a null-terminated array
        // of pointers into `cstrings`, which outlives the call.
        let rc = unsafe { mpv_command(self.handle, ptrs.as_ptr()) };
        self.check(rc)
    }

    fn property_string(&self, name: &str) -> Option<String> {
        let name = CString::new(name).ok()?;
        // SAFETY: `self.handle` is valid; `name` outlives the call. The
        // returned pointer (if non-null) is libmpv-allocated and must be
        // freed with `mpv_free`, which we do below.
        let raw = unsafe { mpv_get_property_string(self.handle, name.as_ptr()) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is a valid, NUL-terminated string owned by libmpv.
        let value = unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `raw` was returned by libmpv; freeing returns the storage.
        unsafe { mpv_free(raw.cast::<c_void>()) };
        Some(value)
    }

    fn property_f64(&self, name: &str) -> Option<f64> {
        self.property_string(name)
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
    }

    fn property_bool(&self, name: &str) -> Option<bool> {
        self.property_string(name)
            .and_then(|value| match value.as_str() {
                "yes" | "true" => Some(true),
                "no" | "false" => Some(false),
                _ => None,
            })
    }

    fn check(&self, rc: c_int) -> Result<(), MpvError> {
        if rc >= 0 {
            Ok(())
        } else {
            Err(MpvError::Lib(mpv_error_to_string(rc)))
        }
    }
}

impl Drop for MpvSession {
    fn drop(&mut self) {
        tracing::debug!(token = %self.token, "MpvSession::drop begin");

        // Unregister the wakeup callback FIRST. After this returns libmpv
        // guarantees no further callback invocations, so it's safe to free
        // the wakeup userdata Box later in this Drop.
        if !self.handle.is_null() {
            // SAFETY: `self.handle` is non-null.
            unsafe {
                mpv_set_wakeup_callback(self.handle, None, ptr::null_mut());
            }
        }
        tracing::debug!("MpvSession::drop wakeup callback cleared");

        // Detach the render lifecycle handler. The host implementation must
        // synchronously release any `mpv_render_context` it holds before
        // returning — otherwise the upcoming `mpv_terminate_destroy` would
        // race with the still-live render context.
        self.handler.detach();
        tracing::debug!("MpvSession::drop handler detached");

        if !self.handle.is_null() {
            // SAFETY: `self.handle` is non-null. `mpv_terminate_destroy`
            // blocks until libmpv's internal threads exit.
            unsafe {
                mpv_terminate_destroy(self.handle);
            }
            self.handle = ptr::null_mut();
        }
        tracing::debug!("MpvSession::drop libmpv handle destroyed");

        // Now safe to drop the wakeup userdata Box — no callback can still
        // be in flight after `mpv_terminate_destroy` returns.
        self.wakeup_ctx.take();
        tracing::debug!(token = %self.token, "MpvSession::drop complete");
    }
}

// -- Wakeup callback ---------------------------------------------------------

struct WakeupCtx {
    emitter: Arc<dyn EventEmitter>,
}

unsafe extern "C" fn wakeup_trampoline(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    // SAFETY: `ctx` was set in `MpvSession::open` to a `Box::into_raw` of a
    // `WakeupCtx`, and that Box is held alive in `MpvSession::wakeup_ctx`
    // until after `mpv_terminate_destroy` returns. libmpv guarantees no
    // wakeups happen after that point, so dereferencing here is sound.
    let cb = unsafe { &*(ctx as *const WakeupCtx) };
    cb.emitter.wake();
}

// -- Render context ----------------------------------------------------------

/// Wrapper around `mpv_render_context`. The embedding code (macOS
/// `CAOpenGLLayer`) constructs one inside `copyCGLContextForPixelFormat:`
/// (i.e. inside its GL thread, with the CGL context current) and calls
/// [`render`] on each `drawInCGLContext:` invocation.
///
/// The render context is freed on Drop, which calls `mpv_render_context_free`.
/// That call is safe to make from any thread but must happen *before* the
/// corresponding `mpv_handle` is destroyed.
///
/// [`render`]: MpvRenderContext::render
pub struct MpvRenderContext {
    ctx: *mut mpv_render_context,
}

// SAFETY: libmpv documents `mpv_render_context_*` as thread-safe. The host is
// responsible for making the right GL context current before calling `render`.
unsafe impl Send for MpvRenderContext {}
unsafe impl Sync for MpvRenderContext {}

impl MpvRenderContext {
    /// Create the render context against `handle`. `get_proc_address` is the
    /// usual GL-loader callback; libmpv uses it to resolve GL entry points.
    ///
    /// Must be called with a GL context current on the calling thread.
    pub fn new(
        handle: MpvRawHandle,
        get_proc_address_ctx: *mut c_void,
        get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    ) -> Result<Self, MpvError> {
        let mut init_params = mpv_opengl_init_params {
            get_proc_address: Some(get_proc_address),
            get_proc_address_ctx,
        };
        let api_type = CString::new(MPV_RENDER_API_TYPE_OPENGL)
            .map_err(|_| MpvError::RenderContext("invalid render API type".to_string()))?;
        let mut params = [
            mpv_render_param {
                param_type: MPV_RENDER_PARAM_API_TYPE,
                data: api_type.as_ptr().cast::<c_void>().cast_mut(),
            },
            mpv_render_param {
                param_type: MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: (&mut init_params as *mut mpv_opengl_init_params).cast::<c_void>(),
            },
            mpv_render_param {
                param_type: MPV_RENDER_PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        let mut ctx: *mut mpv_render_context = ptr::null_mut();
        // SAFETY: `handle.as_ptr()` is a live mpv handle (invariant on
        // `MpvRawHandle`'s API). `params` is fully initialized above and
        // outlives the call; `init_params` likewise.
        let rc = unsafe {
            mpv_render_context_create(&mut ctx, handle.as_ptr(), params.as_mut_ptr())
        };
        if rc < 0 {
            return Err(MpvError::RenderContext(mpv_error_to_string(rc)));
        }
        if ctx.is_null() {
            return Err(MpvError::RenderContext(
                "libmpv returned a null render context".to_string(),
            ));
        }
        Ok(Self { ctx })
    }

    /// Register a callback libmpv invokes when a new frame is ready to render.
    /// The host should signal its GL surface to redraw (e.g. `setNeedsDisplay`
    /// on the layer).
    ///
    /// `ctx` is opaque to libmpv — passed back to `callback` on each
    /// invocation.
    ///
    /// # Safety
    ///
    /// `ctx` must remain valid (live, properly aligned, and unchanged in type)
    /// until either this render context is dropped or this method is called
    /// again with `None`.
    pub unsafe fn set_update_callback(
        &self,
        callback: Option<unsafe extern "C" fn(*mut c_void)>,
        ctx: *mut c_void,
    ) {
        // SAFETY: `self.ctx` is a valid render context owned by `self`. The
        // caller's invariant ensures `ctx` outlives the callback.
        unsafe { mpv_render_context_set_update_callback(self.ctx, callback, ctx) };
    }

    /// Render one frame into the bound framebuffer. Must be called with a GL
    /// context current on the calling thread.
    pub fn render(&self, fbo: i32, width: i32, height: i32, flip_y: bool) {
        // SAFETY: `self.ctx` is valid; all params are fully populated below
        // and outlive the call.
        unsafe {
            let update = mpv_render_context_update(self.ctx);
            if update & MPV_RENDER_UPDATE_FRAME == 0 {
                // No new frame — libmpv may still want a redraw of the last
                // frame (e.g. for window-size changes), so render anyway.
            }

            let mut fbo = mpv_opengl_fbo {
                fbo,
                w: width,
                h: height,
                internal_format: 0,
            };
            let mut flip_y_c: c_int = if flip_y { 1 } else { 0 };
            let mut params = [
                mpv_render_param {
                    param_type: MPV_RENDER_PARAM_OPENGL_FBO,
                    data: (&mut fbo as *mut mpv_opengl_fbo).cast::<c_void>(),
                },
                mpv_render_param {
                    param_type: MPV_RENDER_PARAM_FLIP_Y,
                    data: (&mut flip_y_c as *mut c_int).cast::<c_void>(),
                },
                mpv_render_param {
                    param_type: MPV_RENDER_PARAM_INVALID,
                    data: ptr::null_mut(),
                },
            ];
            let _ = mpv_render_context_render(self.ctx, params.as_mut_ptr());
        }
    }
}

impl Drop for MpvRenderContext {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            // SAFETY: `self.ctx` is valid until dropped; libmpv's
            // `mpv_render_context_free` is the documented teardown call.
            unsafe {
                mpv_render_context_set_update_callback(self.ctx, None, ptr::null_mut());
                mpv_render_context_free(self.ctx);
            }
            self.ctx = ptr::null_mut();
        }
    }
}

// -- Error helper ------------------------------------------------------------

fn mpv_error_to_string(error: c_int) -> String {
    // SAFETY: `mpv_error_string` returns a static C string or null.
    let raw = unsafe { mpv_error_string(error) };
    if raw.is_null() {
        return format!("unknown libmpv error {error}");
    }
    // SAFETY: `raw` is a libmpv-owned, NUL-terminated static string.
    unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned()
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct StubHandler {
        attached: AtomicBool,
    }

    impl RenderLifecycleHandler for StubHandler {
        fn attach(&self, _handle: MpvRawHandle) -> Result<(), String> {
            self.attached.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn detach(&self) {
            self.attached.store(false, Ordering::SeqCst);
        }
    }

    #[test]
    fn open_without_handler_returns_no_render_handler() {
        let engine = MpvEngine::new();
        let result = engine.open(MpvOpenOptions {
            url: "http://example.invalid/x".to_string(),
            title: "x".to_string(),
            user_agent: None,
        });
        assert!(matches!(result, Err(MpvError::NoRenderHandler)));
    }

    #[test]
    fn engine_accepts_handler_registration() {
        let engine = MpvEngine::new();
        let handler = Arc::new(StubHandler {
            attached: AtomicBool::new(false),
        });
        engine.set_render_handler(handler.clone()).unwrap();
        // We don't actually open here — `mpv_create` requires libmpv to be
        // linked. The registration itself shouldn't error.
        assert!(!handler.attached.load(Ordering::SeqCst));
    }
}

// -- FFI ---------------------------------------------------------------------

#[allow(non_camel_case_types)]
enum mpv_handle {}

#[repr(C)]
struct mpv_event {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct mpv_event_end_file {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}

#[allow(non_camel_case_types)]
enum mpv_render_context {}

#[repr(C)]
struct mpv_render_param {
    param_type: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct mpv_opengl_init_params {
    get_proc_address: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void>,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct mpv_opengl_fbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

const MPV_EVENT_NONE: c_int = 0;
const MPV_EVENT_SHUTDOWN: c_int = 1;
const MPV_EVENT_END_FILE: c_int = 7;
const MPV_EVENT_FILE_LOADED: c_int = 8;
const MPV_EVENT_PLAYBACK_RESTART: c_int = 21;

const MPV_END_FILE_REASON_EOF: c_int = 0;
const MPV_END_FILE_REASON_ERROR: c_int = 4;

const MPV_FORMAT_NONE: c_int = 0;

const MPV_RENDER_API_TYPE_OPENGL: &str = "opengl";
const MPV_RENDER_PARAM_INVALID: c_int = 0;
const MPV_RENDER_PARAM_API_TYPE: c_int = 1;
const MPV_RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const MPV_RENDER_PARAM_OPENGL_FBO: c_int = 3;
const MPV_RENDER_PARAM_FLIP_Y: c_int = 4;
const MPV_RENDER_UPDATE_FRAME: u64 = 1;

unsafe extern "C" {
    fn mpv_create() -> *mut mpv_handle;
    fn mpv_initialize(ctx: *mut mpv_handle) -> c_int;
    fn mpv_terminate_destroy(ctx: *mut mpv_handle);
    fn mpv_set_option_string(
        ctx: *mut mpv_handle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int;
    fn mpv_set_property_string(
        ctx: *mut mpv_handle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int;
    fn mpv_get_property_string(ctx: *mut mpv_handle, name: *const c_char) -> *mut c_char;
    fn mpv_command(ctx: *mut mpv_handle, args: *const *const c_char) -> c_int;
    fn mpv_wait_event(ctx: *mut mpv_handle, timeout: c_double) -> *mut mpv_event;
    fn mpv_error_string(error: c_int) -> *const c_char;
    fn mpv_free(data: *mut c_void);
    fn mpv_observe_property(
        ctx: *mut mpv_handle,
        reply_userdata: u64,
        name: *const c_char,
        format: c_int,
    ) -> c_int;
    fn mpv_set_wakeup_callback(
        ctx: *mut mpv_handle,
        callback: Option<unsafe extern "C" fn(*mut c_void)>,
        ctx_data: *mut c_void,
    );
    fn mpv_render_context_create(
        res: *mut *mut mpv_render_context,
        mpv: *mut mpv_handle,
        params: *mut mpv_render_param,
    ) -> c_int;
    fn mpv_render_context_set_update_callback(
        ctx: *mut mpv_render_context,
        callback: Option<unsafe extern "C" fn(*mut c_void)>,
        callback_ctx: *mut c_void,
    );
    fn mpv_render_context_update(ctx: *mut mpv_render_context) -> u64;
    fn mpv_render_context_render(
        ctx: *mut mpv_render_context,
        params: *mut mpv_render_param,
    ) -> c_int;
    fn mpv_render_context_free(ctx: *mut mpv_render_context);
}
