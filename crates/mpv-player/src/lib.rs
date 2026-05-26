//! Minimal libmpv playback engine wrapper.
//!
//! This is intentionally small: Nova owns UI, watch history, captions, and
//! future transcription/translation; libmpv owns hostile media playback.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int, c_void};
use std::ptr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

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
    #[error("native mpv embed surface is not available")]
    EmbedSurfaceUnavailable,
    #[error("native mpv render surface is not available")]
    RenderSurfaceUnavailable,
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
    pub embedded_required: bool,
    pub render_surface_required: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct MpvRenderSurface {
    pub callback_ctx: usize,
    pub make_current: unsafe extern "C" fn(usize) -> bool,
    pub flush: unsafe extern "C" fn(usize),
    pub size: unsafe extern "C" fn(usize, *mut c_int, *mut c_int) -> bool,
    pub get_proc_address_ctx: usize,
    pub get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
}

#[derive(Clone)]
pub struct MpvRenderSurfaceRegistration {
    surface: MpvRenderSurface,
    _keepalive: Arc<dyn std::any::Any + Send + Sync>,
}

impl MpvRenderSurfaceRegistration {
    pub fn new<T>(surface: MpvRenderSurface, keepalive: Arc<T>) -> Self
    where
        T: std::any::Any + Send + Sync + 'static,
    {
        Self {
            surface,
            _keepalive: keepalive,
        }
    }
}

#[derive(Clone, Default)]
pub struct MpvEngine {
    inner: Arc<Mutex<Option<MpvSession>>>,
    embed_wid: Arc<Mutex<Option<u64>>>,
    render_surface: Arc<Mutex<Option<MpvRenderSurfaceRegistration>>>,
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

    pub fn set_render_surface(
        &self,
        registration: MpvRenderSurfaceRegistration,
    ) -> Result<(), MpvError> {
        let mut guard = self.render_surface.lock().map_err(|_| MpvError::Poisoned)?;
        *guard = Some(registration);
        Ok(())
    }

    pub fn open(&self, options: MpvOpenOptions) -> Result<MpvOpenResult, MpvError> {
        let render_surface = self
            .render_surface
            .lock()
            .map_err(|_| MpvError::Poisoned)?
            .clone();
        let render_surface = render_surface.map(|registration| registration.surface);
        if options.render_surface_required && render_surface.is_none() {
            return Err(MpvError::RenderSurfaceUnavailable);
        }

        let embed_wid = *self.embed_wid.lock().map_err(|_| MpvError::Poisoned)?;
        let window_target = resolve_window_target(
            render_surface,
            options.embed_wid,
            embed_wid,
            options.embedded_required,
        )?;

        let mut guard = self.inner.lock().map_err(|_| MpvError::Poisoned)?;
        if let Some(session) = guard.take() {
            drop(session);
        }

        let (render_surface, embed_wid) = match window_target {
            MpvWindowTarget::RenderApi(surface) => (Some(surface), None),
            MpvWindowTarget::Embedded(wid) => (None, Some(wid)),
            MpvWindowTarget::ExternalWindow => (None, None),
        };
        let options = MpvOpenOptions {
            embed_wid,
            render_surface_required: options.render_surface_required,
            ..options
        };
        let session = MpvSession::open(options, render_surface)?;
        let token = session.token.clone();
        let embedded = session.is_embedded();
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

#[derive(Debug, Clone, Copy)]
enum MpvWindowTarget {
    RenderApi(MpvRenderSurface),
    Embedded(u64),
    ExternalWindow,
}

fn resolve_window_target(
    render_surface: Option<MpvRenderSurface>,
    requested_wid: Option<u64>,
    registered_wid: Option<u64>,
    embedded_required: bool,
) -> Result<MpvWindowTarget, MpvError> {
    if let Some(surface) = render_surface {
        return Ok(MpvWindowTarget::RenderApi(surface));
    }

    if let Some(wid) = requested_wid.or(registered_wid) {
        return Ok(MpvWindowTarget::Embedded(wid));
    }

    if embedded_required {
        return Err(MpvError::EmbedSurfaceUnavailable);
    }

    Ok(MpvWindowTarget::ExternalWindow)
}

struct MpvSession {
    handle: *mut mpv_handle,
    token: String,
    last_phase: MpvPlaybackPhase,
    last_error: Option<String>,
    opened_at: Instant,
    embed_wid: Option<u64>,
    render: Option<MpvRenderSession>,
}

unsafe impl Send for MpvSession {}

impl MpvSession {
    fn open(
        options: MpvOpenOptions,
        render_surface: Option<MpvRenderSurface>,
    ) -> Result<Self, MpvError> {
        if options.render_surface_required && render_surface.is_none() {
            return Err(MpvError::RenderSurfaceUnavailable);
        }
        if options.embedded_required && options.embed_wid.is_none() && render_surface.is_none() {
            return Err(MpvError::EmbedSurfaceUnavailable);
        }

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
            render: None,
        };

        if render_surface.is_some() {
            session.set_option("vo", "libmpv")?;
            session.set_option("force-window", "no")?;
        } else if let Some(wid) = options.embed_wid {
            // `wid` is mpv's native-window embedding path. Forcing `vo=libmpv`
            // here switches to render-context embedding, but Nova does not own
            // a render context for this session.
            session.set_option("wid", &wid.to_string())?;
            session.set_option("force-window", "no")?;
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
        let mut session = session;
        if let Some(surface) = render_surface {
            session.render = Some(MpvRenderSession::new(handle, surface)?);
        }
        let _ = session.set_property("force-media-title", &options.title);
        session.command(&["loadfile", &options.url, "replace"])?;
        Ok(session)
    }

    fn is_embedded(&self) -> bool {
        self.render.is_some() || self.embed_wid.is_some()
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
        if let Some(render) = self.render.take() {
            drop(render);
        }
        if !self.handle.is_null() {
            unsafe { mpv_terminate_destroy(self.handle) };
            self.handle = ptr::null_mut();
        }
    }
}

struct MpvRenderSession {
    ctx: *mut mpv_render_context,
    stop: Arc<AtomicBool>,
    wake: Arc<(Mutex<bool>, Condvar)>,
    wake_raw: *const (Mutex<bool>, Condvar),
    thread: Option<JoinHandle<()>>,
}

unsafe impl Send for MpvRenderSession {}

impl MpvRenderSession {
    fn new(handle: *mut mpv_handle, surface: MpvRenderSurface) -> Result<Self, MpvError> {
        if unsafe { !(surface.make_current)(surface.callback_ctx) } {
            return Err(MpvError::RenderContext(
                "OpenGL context could not be made current".to_string(),
            ));
        }

        let mut init_params = mpv_opengl_init_params {
            get_proc_address: Some(surface.get_proc_address),
            get_proc_address_ctx: surface.get_proc_address_ctx as *mut c_void,
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
        let mut ctx = ptr::null_mut();
        let rc = unsafe { mpv_render_context_create(&mut ctx, handle, params.as_mut_ptr()) };
        if rc < 0 {
            return Err(MpvError::RenderContext(mpv_error_to_string(rc)));
        }
        if ctx.is_null() {
            return Err(MpvError::RenderContext(
                "libmpv returned a null render context".to_string(),
            ));
        }

        let stop = Arc::new(AtomicBool::new(false));
        let wake = Arc::new((Mutex::new(false), Condvar::new()));
        let wake_raw = Arc::into_raw(wake.clone());
        unsafe {
            mpv_render_context_set_update_callback(
                ctx,
                Some(render_update_callback),
                wake_raw.cast::<c_void>().cast_mut(),
            );
        }

        let thread_stop = stop.clone();
        let thread_wake = wake.clone();
        let thread_ctx = RenderContextPtr(ctx);
        let thread = thread::spawn(move || {
            render_loop(thread_ctx, surface, thread_stop, thread_wake);
        });

        Ok(Self {
            ctx,
            stop,
            wake,
            wake_raw,
            thread: Some(thread),
        })
    }
}

impl Drop for MpvRenderSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        {
            let (lock, cvar) = &*self.wake;
            if let Ok(mut pending) = lock.lock() {
                *pending = true;
                cvar.notify_one();
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if !self.ctx.is_null() {
            unsafe {
                mpv_render_context_set_update_callback(self.ctx, None, ptr::null_mut());
                mpv_render_context_free(self.ctx);
            }
            self.ctx = ptr::null_mut();
        }
        if !self.wake_raw.is_null() {
            unsafe {
                drop(Arc::from_raw(self.wake_raw));
            }
            self.wake_raw = ptr::null();
        }
    }
}

struct RenderContextPtr(*mut mpv_render_context);

unsafe impl Send for RenderContextPtr {}

unsafe extern "C" fn render_update_callback(callback_ctx: *mut c_void) {
    if callback_ctx.is_null() {
        return;
    }
    let wake = unsafe { &*(callback_ctx.cast::<(Mutex<bool>, Condvar)>()) };
    let (lock, cvar) = wake;
    if let Ok(mut pending) = lock.lock() {
        *pending = true;
        cvar.notify_one();
    }
}

fn render_loop(
    ctx: RenderContextPtr,
    surface: MpvRenderSurface,
    stop: Arc<AtomicBool>,
    wake: Arc<(Mutex<bool>, Condvar)>,
) {
    while !stop.load(Ordering::SeqCst) {
        let (lock, cvar) = &*wake;
        if let Ok(mut pending) = lock.lock() {
            if !*pending {
                let (guard, _) = cvar
                    .wait_timeout(pending, Duration::from_millis(16))
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                pending = guard;
            }
            *pending = false;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        unsafe {
            render_frame(ctx.0, surface);
        }
    }
}

unsafe fn render_frame(ctx: *mut mpv_render_context, surface: MpvRenderSurface) {
    if ctx.is_null() || !(surface.make_current)(surface.callback_ctx) {
        return;
    }

    let update = mpv_render_context_update(ctx);
    if update & MPV_RENDER_UPDATE_FRAME == 0 {
        return;
    }

    let mut width = 0;
    let mut height = 0;
    if !(surface.size)(surface.callback_ctx, &mut width, &mut height) {
        return;
    }
    if width <= 0 || height <= 0 {
        return;
    }

    let mut fbo = mpv_opengl_fbo {
        fbo: 0,
        w: width,
        h: height,
        internal_format: 0,
    };
    let mut flip_y: c_int = 1;
    let mut params = [
        mpv_render_param {
            param_type: MPV_RENDER_PARAM_OPENGL_FBO,
            data: (&mut fbo as *mut mpv_opengl_fbo).cast::<c_void>(),
        },
        mpv_render_param {
            param_type: MPV_RENDER_PARAM_FLIP_Y,
            data: (&mut flip_y as *mut c_int).cast::<c_void>(),
        },
        mpv_render_param {
            param_type: MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        },
    ];
    let _ = mpv_render_context_render(ctx, params.as_mut_ptr());
    (surface.flush)(surface.callback_ctx);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_required_rejects_missing_surface() {
        let result = resolve_window_target(None, None, None, true);

        assert!(matches!(result, Err(MpvError::EmbedSurfaceUnavailable)));
    }

    #[test]
    fn embedded_required_uses_registered_surface() {
        let result = resolve_window_target(None, None, Some(42), true);

        assert!(matches!(result.unwrap(), MpvWindowTarget::Embedded(42)));
    }

    #[test]
    fn render_api_wins_over_window_ids() {
        let result = resolve_window_target(Some(test_surface()), Some(7), Some(42), true);

        assert!(matches!(result.unwrap(), MpvWindowTarget::RenderApi(_)));
    }

    #[test]
    fn explicit_surface_wins_over_registered_surface() {
        let result = resolve_window_target(None, Some(7), Some(42), true);

        assert!(matches!(result.unwrap(), MpvWindowTarget::Embedded(7)));
    }

    #[test]
    fn external_window_allowed_only_when_embedding_not_required() {
        let result = resolve_window_target(None, None, None, false);

        assert!(matches!(result.unwrap(), MpvWindowTarget::ExternalWindow));
    }

    fn test_surface() -> MpvRenderSurface {
        MpvRenderSurface {
            callback_ctx: 0,
            make_current: test_make_current,
            flush: test_flush,
            size: test_size,
            get_proc_address_ctx: 0,
            get_proc_address: test_get_proc_address,
        }
    }

    unsafe extern "C" fn test_make_current(_: usize) -> bool {
        true
    }

    unsafe extern "C" fn test_flush(_: usize) {}

    unsafe extern "C" fn test_size(_: usize, width: *mut c_int, height: *mut c_int) -> bool {
        if !width.is_null() {
            unsafe { *width = 1 };
        }
        if !height.is_null() {
            unsafe { *height = 1 };
        }
        true
    }

    unsafe extern "C" fn test_get_proc_address(_: *mut c_void, _: *const c_char) -> *mut c_void {
        ptr::null_mut()
    }
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
