//! Minimal libmpv playback engine wrapper.
//!
//! This is intentionally small: Nova owns UI, watch history, captions, and
//! future transcription/translation; libmpv owns hostile media playback.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int, c_void};
use std::ptr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
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

#[derive(Debug, Clone, Serialize)]
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
pub struct MpvPlayerState {
    pub token: Option<String>,
    pub engine: String,
    pub phase: MpvPlaybackPhase,
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    pub volume: f64,
    pub muted: bool,
    pub seekable: bool,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct MpvOpenOptions {
    pub url: String,
    pub title: String,
    pub user_agent: Option<String>,
    pub embed_wid: Option<u64>,
}

#[derive(Clone, Default)]
pub struct MpvEngine {
    inner: Arc<Mutex<Option<MpvSession>>>,
    embed_wid: Arc<Mutex<Option<u64>>>,
}

impl MpvEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_embed_wid(&self, wid: u64) -> Result<(), MpvError> {
        let mut guard = self.embed_wid.lock().map_err(|_| MpvError::Poisoned)?;
        *guard = Some(wid);
        Ok(())
    }

    pub fn open(&self, options: MpvOpenOptions) -> Result<MpvOpenResult, MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        if let Some(session) = guard.take() {
            drop(session);
        }

        let embed_wid = *self.embed_wid.lock().map_err(|_| MpvError::Poisoned)?;
        let options = MpvOpenOptions {
            embed_wid: options.embed_wid.or(embed_wid),
            ..options
        };
        let session = MpvSession::open(options)?;
        let token = session.token.clone();
        let embedded = session.embed_wid.is_some();
        *guard = Some(session);

        Ok(MpvOpenResult {
            token,
            engine: "mpv".to_string(),
            embedded,
        })
    }

    pub fn close(&self, token: &str) -> Result<(), MpvError> {
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

    pub fn state(&self, token: Option<&str>) -> Result<MpvPlayerState, MpvError> {
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
                error: None,
            });
        };
        if let Some(token) = token {
            session.require_token(token)?;
        }
        session.state()
    }

    pub fn toggle_pause(&self, token: &str) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        let session = active_session(&mut guard, token)?;
        let paused = session.property_bool("pause").unwrap_or(false);
        session.set_property("pause", if paused { "no" } else { "yes" })
    }

    pub fn seek_relative(&self, token: &str, delta_sec: f64) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.command(&[
            "seek",
            &format!("{delta_sec:.3}"),
            "relative",
            "exact",
        ])
    }

    pub fn seek_absolute(&self, token: &str, position_sec: f64) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.command(&[
            "seek",
            &format!("{position_sec:.3}"),
            "absolute",
            "exact",
        ])
    }

    pub fn seek_live_edge(&self, token: &str) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.command(&["seek", "100", "absolute-percent"])
    }

    pub fn set_volume(&self, token: &str, volume: f64) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        let clamped = volume.clamp(0.0, 1.0) * 100.0;
        active_session(&mut guard, token)?.set_property("volume", &format!("{clamped:.1}"))
    }

    pub fn set_mute(&self, token: &str, muted: bool) -> Result<(), MpvError> {
        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        active_session(&mut guard, token)?.set_property("mute", if muted { "yes" } else { "no" })
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

struct MpvSession {
    handle: *mut mpv_handle,
    token: String,
    last_phase: MpvPlaybackPhase,
    last_error: Option<String>,
    opened_at: Instant,
    embed_wid: Option<u64>,
}

unsafe impl Send for MpvSession {}

impl MpvSession {
    fn open(options: MpvOpenOptions) -> Result<Self, MpvError> {
        let handle = unsafe { mpv_create() };
        if handle.is_null() {
            return Err(MpvError::NotAvailable);
        }

        let session = Self {
            handle,
            token: Uuid::new_v4().to_string(),
            last_phase: MpvPlaybackPhase::Opening,
            last_error: None,
            opened_at: Instant::now(),
            embed_wid: options.embed_wid,
        };

        if let Some(wid) = options.embed_wid {
            // `wid` is mpv's native-window embedding path. Forcing `vo=libmpv`
            // here switches to render-context embedding, but Nova does not own
            // a render context for this session.
            session.set_option("wid", &wid.to_string())?;
        } else {
            session.set_option("force-window", "yes")?;
        }
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

        let rc = unsafe { mpv_initialize(handle) };
        session.check(rc)?;
        let _ = session.set_property("force-media-title", &options.title);
        session.command(&["loadfile", &options.url, "replace"])?;
        Ok(session)
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
            error: self.last_error.clone(),
        })
    }

    fn drain_events(&mut self) {
        loop {
            let event = unsafe { mpv_wait_event(self.handle, 0.0) };
            if event.is_null() {
                break;
            }
            let event = unsafe { &*event };
            match event.event_id {
                MPV_EVENT_NONE => break,
                MPV_EVENT_FILE_LOADED | MPV_EVENT_PLAYBACK_RESTART => {
                    self.last_phase = MpvPlaybackPhase::Playing;
                    self.last_error = None;
                }
                MPV_EVENT_END_FILE if !event.data.is_null() => {
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

    fn set_option(&self, name: &str, value: &str) -> Result<(), MpvError> {
        let name = CString::new(name).map_err(|_| MpvError::InvalidString)?;
        let value = CString::new(value).map_err(|_| MpvError::InvalidString)?;
        let rc = unsafe { mpv_set_option_string(self.handle, name.as_ptr(), value.as_ptr()) };
        self.check(rc)
    }

    fn set_property(&self, name: &str, value: &str) -> Result<(), MpvError> {
        let name = CString::new(name).map_err(|_| MpvError::InvalidString)?;
        let value = CString::new(value).map_err(|_| MpvError::InvalidString)?;
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
        let rc = unsafe { mpv_command(self.handle, ptrs.as_ptr()) };
        self.check(rc)
    }

    fn property_string(&self, name: &str) -> Option<String> {
        let name = CString::new(name).ok()?;
        let raw = unsafe { mpv_get_property_string(self.handle, name.as_ptr()) };
        if raw.is_null() {
            return None;
        }
        let value = unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
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
        if !self.handle.is_null() {
            unsafe { mpv_terminate_destroy(self.handle) };
            self.handle = ptr::null_mut();
        }
    }
}

fn mpv_error_to_string(error: c_int) -> String {
    let raw = unsafe { mpv_error_string(error) };
    if raw.is_null() {
        return format!("unknown libmpv error {error}");
    }
    unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned()
}

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

const MPV_EVENT_NONE: c_int = 0;
const MPV_EVENT_SHUTDOWN: c_int = 1;
const MPV_EVENT_END_FILE: c_int = 7;
const MPV_EVENT_FILE_LOADED: c_int = 8;
const MPV_EVENT_PLAYBACK_RESTART: c_int = 21;

const MPV_END_FILE_REASON_EOF: c_int = 0;
const MPV_END_FILE_REASON_ERROR: c_int = 4;

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
}
