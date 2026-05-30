//! AirPlay engine — parallel AVFoundation playback for external (AirPlay) routes.
//!
//! libmpv decodes frames into a `CAOpenGLLayer` and has no AVFoundation route,
//! so the local engine can never AirPlay. True AirPlay *video* (stream goes to
//! the Apple TV; the Mac shows a "Playing on Apple TV" placeholder) is an
//! `AVPlayer.allowsExternalPlayback` feature. This module owns a second,
//! AVFoundation-based engine that takes over only while an AirPlay device is
//! selected.
//!
//! Key points:
//! - The Apple TV (not libmpv) decodes, so the stream must be AVFoundation-
//!   friendly HLS. We feed `AVPlayer` the local proxy URL produced by
//!   `resolve_stream` (`crates/hls-proxy` already remuxes hostile IPTV).
//! - `AVRoutePickerView` is the only Apple-sanctioned route picker. It is bound
//!   to our `AVPlayer` via `setPlayer:` and embedded as a native NSView overlay
//!   over the HTML AirPlay button (positioned from a CSS rect).
//! - All AVFoundation objects are touched **only on the main thread**. The
//!   engine stores raw pointers and marshals every operation through
//!   `AppHandle::run_on_main_thread`. A light poll loop emits `airplay:status`.
//!
//! Handoff is orchestrated by the frontend (which already owns the mpv token,
//! current media identity, and position): on `airplay:picker-closed` it calls
//! `airplay_engage`, watches `airplay:status.engaged`, and pauses/resumes the
//! mpv session accordingly.

use std::sync::Mutex;

use serde::Serialize;

/// Snapshot of the AirPlay engine, emitted as `airplay:status` and returned by
/// the engage/status commands. Mirrors the fields the frontend needs to drive
/// the placeholder UI and decide when to hand mpv off / back.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AirPlayStatus {
    /// A non-local route (AirPlay device) is reachable — show the button.
    pub available: bool,
    /// `AVPlayer` is currently playing on the external device.
    pub engaged: bool,
    /// An item is loaded in the AVPlayer (engage attempted / in progress).
    pub active: bool,
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    pub paused: bool,
    pub volume: f64,
    pub muted: bool,
    pub error: Option<String>,
}

/// AirPlay engine handle stored in `AppState`. Platform-neutral surface; the
/// real AVFoundation work lives in the macOS `imp` module. On other platforms
/// every method is a no-op and `available()` is always false.
#[derive(Default)]
pub struct AirPlayEngine {
    #[cfg(target_os = "macos")]
    inner: Mutex<Option<imp::AirPlayInner>>,
    #[cfg(not(target_os = "macos"))]
    _unused: Mutex<()>,
}

impl AirPlayEngine {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::default())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::AirPlayStatus;
    use std::sync::Arc;
    use std::time::Duration;

    use nova_mpv_player::VideoRect;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker};
    use objc2_app_kit::{NSColor, NSView, NSWindow};
    use objc2_av_foundation::{
        AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVPlayerTimeControlStatus, AVRouteDetector,
    };
    use objc2_av_kit::{
        AVRoutePickerView, AVRoutePickerViewButtonState, AVRoutePickerViewDelegate,
    };
    use objc2_core_media::CMTime;
    use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSURL};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use tauri::{AppHandle, Emitter};

    use super::AirPlayEngine;

    /// CMTime timescale used when seeking; 600 is the AVFoundation convention
    /// (divisible by common frame rates).
    const TIMESCALE: i32 = 600;

    /// Raw pointers to the AVFoundation objects. They are created once on the
    /// main thread in [`install`] and live for the app's lifetime (we leak the
    /// `Retained` values intentionally, mirroring the proxy handle leak in
    /// `lib.rs`). Stored as `usize` so the enclosing `Mutex<Option<…>>` is
    /// trivially `Send + Sync`; every dereference happens on the main thread.
    pub struct AirPlayInner {
        player: usize,   // *const AVPlayer
        detector: usize, // *const AVRouteDetector
        picker: usize,   // *const AVRoutePickerView
        /// The NSView the picker is a subview of — used to flip CSS rects.
        host_view: usize, // *const NSView
    }

    // SAFETY: the pointers are only dereferenced inside `run_on_main_thread`
    // closures (i.e. on the main thread). The struct itself carries no thread
    // affinity beyond that discipline.
    unsafe impl Send for AirPlayInner {}

    impl AirPlayInner {
        fn player(&self) -> &AVPlayer {
            // SAFETY: `player` was set from a leaked `Retained<AVPlayer>` in
            // `install`; valid for the app lifetime. Called on the main thread.
            unsafe { &*(self.player as *const AVPlayer) }
        }
        fn detector(&self) -> &AVRouteDetector {
            // SAFETY: see `player`.
            unsafe { &*(self.detector as *const AVRouteDetector) }
        }
        fn picker(&self) -> &AVRoutePickerView {
            // SAFETY: see `player`.
            unsafe { &*(self.picker as *const AVRoutePickerView) }
        }
        fn host_view(&self) -> &NSView {
            // SAFETY: see `player`.
            unsafe { &*(self.host_view as *const NSView) }
        }

        /// Load `url` into the player at `position_sec`, muted, and start
        /// playing. Muted so that if no external route is actually selected we
        /// don't double up audio with the still-playing mpv session; the
        /// frontend unmutes once `engaged` is confirmed.
        fn load(&self, url: &str, position_sec: f64, mtm: MainThreadMarker) {
            // SAFETY: standard AVFoundation calls on the main thread; all
            // arguments outlive the calls.
            unsafe {
                let ns = NSString::from_str(url);
                let Some(nsurl) = NSURL::URLWithString(&ns) else {
                    return;
                };
                let item = AVPlayerItem::playerItemWithURL(&nsurl, mtm);
                let player = self.player();
                player.setMuted(true);
                player.replaceCurrentItemWithPlayerItem(Some(&item));
                if position_sec > 0.0 {
                    player.seekToTime(CMTime::with_seconds(position_sec, TIMESCALE));
                }
                player.play();
            }
        }

        fn clear(&self) {
            // SAFETY: main-thread AVPlayer teardown.
            unsafe {
                let player = self.player();
                player.pause();
                player.replaceCurrentItemWithPlayerItem(None);
            }
        }

        fn toggle_pause(&self) {
            // SAFETY: main-thread reads/writes of timeControlStatus.
            unsafe {
                let player = self.player();
                if player.timeControlStatus() == AVPlayerTimeControlStatus::Paused {
                    player.play();
                } else {
                    player.pause();
                }
            }
        }

        fn seek_absolute(&self, position_sec: f64) {
            // SAFETY: main-thread seek.
            unsafe {
                self.player()
                    .seekToTime(CMTime::with_seconds(position_sec.max(0.0), TIMESCALE));
            }
        }

        fn set_volume(&self, volume: f64) {
            // SAFETY: main-thread setter; AVPlayer volume is 0.0..=1.0.
            unsafe {
                self.player()
                    .setVolume(volume.clamp(0.0, 1.0) as core::ffi::c_float);
            }
        }

        fn set_muted(&self, muted: bool) {
            // SAFETY: main-thread setter.
            unsafe { self.player().setMuted(muted) }
        }

        /// Position the picker overlay to `rect` (CSS px, window-relative), or
        /// hide it when `None`.
        fn set_picker_rect(&self, rect: Option<VideoRect>) {
            let picker = self.picker();
            match rect {
                None => picker.setHidden(true),
                Some(rect) => {
                    let host = self.host_view();
                    let parent: NSRect = host.bounds();
                    let flipped: bool = host.isFlipped();
                    let frame = css_rect_to_view_frame(rect, parent, flipped);
                    let view: &NSView = picker;
                    view.setFrame(frame);
                    picker.setHidden(false);
                }
            }
        }

        fn snapshot(&self) -> AirPlayStatus {
            // SAFETY: main-thread reads of AVPlayer/AVPlayerItem state.
            unsafe {
                let player = self.player();
                let available = self.detector().multipleRoutesDetected();
                let engaged = player.isExternalPlaybackActive();
                let muted = player.isMuted();
                let volume = player.volume() as f64;
                let item = player.currentItem();
                let active = item.is_some();
                let tcs = player.timeControlStatus();
                let paused = tcs == AVPlayerTimeControlStatus::Paused;

                let position_sec = finite_seconds(player.currentTime().seconds());
                let (duration_sec, error) = match item.as_deref() {
                    Some(item) => {
                        let dur = finite_seconds(item.duration().seconds());
                        let dur = if dur > 0.0 { Some(dur) } else { None };
                        let err = if item.status() == AVPlayerItemStatus::Failed {
                            Some("AirPlay item failed to load".to_string())
                        } else {
                            None
                        };
                        (dur, err)
                    }
                    None => (None, None),
                };

                AirPlayStatus {
                    available,
                    engaged,
                    active,
                    position_sec,
                    duration_sec,
                    paused,
                    volume,
                    muted,
                    error,
                }
            }
        }
    }

    fn finite_seconds(v: f64) -> f64 {
        if v.is_finite() && v > 0.0 {
            v
        } else {
            0.0
        }
    }

    /// Convert a CSS-pixel rect (window-relative, y-down from the top-left) into
    /// an NSView frame in the host view's coordinate space. When the host view
    /// is flipped (WKWebView is), CSS y maps directly; otherwise invert.
    fn css_rect_to_view_frame(rect: VideoRect, parent: NSRect, flipped: bool) -> NSRect {
        let origin_y = if flipped {
            rect.y
        } else {
            parent.size.height - rect.y - rect.height
        };
        NSRect::new(
            NSPoint::new(rect.x, origin_y),
            NSSize::new(rect.width, rect.height),
        )
    }

    // -- Route picker delegate ------------------------------------------------
    //
    // We can't observe "device selected" directly. `didEndPresentingRoutes`
    // fires when the user finishes interacting with the system picker; the
    // frontend treats it as "attempt to engage" and confirms via the polled
    // `engaged` flag (reverting if external playback never activated).

    struct DelegateIvars {
        app: AppHandle,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "NovaRoutePickerDelegate"]
        #[ivars = DelegateIvars]
        struct RoutePickerDelegate;

        unsafe impl NSObjectProtocol for RoutePickerDelegate {}

        unsafe impl AVRoutePickerViewDelegate for RoutePickerDelegate {
            #[unsafe(method(routePickerViewDidEndPresentingRoutes:))]
            fn did_end_presenting_routes(&self, _picker: &AVRoutePickerView) {
                let _ = self.ivars().app.emit("airplay:picker-closed", ());
            }
        }
    );

    impl RoutePickerDelegate {
        fn new(app: AppHandle) -> Retained<Self> {
            let this = Self::alloc().set_ivars(DelegateIvars { app });
            // SAFETY: standard NSObject init.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Install the route picker into the window's webview. The `with_webview`
    /// callback runs on the main thread and hands us the live `WKWebView`
    /// pointer, which is an `NSView` subclass we use as the overlay host.
    pub fn install_picker<R: tauri::Runtime>(
        window: &tauri::WebviewWindow<R>,
        engine: &Arc<AirPlayEngine>,
        app: &AppHandle,
    ) {
        let engine = engine.clone();
        let app = app.clone();
        let _ = window.with_webview(move |webview| {
            // SAFETY: Tauri runs this on the main thread and hands us a live
            // WKWebView pointer; WKWebView is an NSView subclass.
            unsafe {
                let wk_ptr = webview.inner();
                if wk_ptr.is_null() {
                    tracing::warn!("AirPlay: null webview pointer; skipping install");
                    return;
                }
                // Prefer the window's contentView as the overlay host: it is a
                // sibling *above* the WKWebView, which renders reliably on top
                // of out-of-process web content (subviews of the WKWebView
                // itself can be hidden by its remote compositing).
                let window_ptr = webview.ns_window();
                if !window_ptr.is_null() {
                    let ns_window: &NSWindow = &*window_ptr.cast::<NSWindow>();
                    if let Some(content) = ns_window.contentView() {
                        install(&engine, &content, &app);
                        return;
                    }
                }
                let host: &NSView = &*wk_ptr.cast::<NSView>();
                install(&engine, host, &app);
            }
        });
    }

    /// Create the AVFoundation objects, bind the route picker to the player,
    /// add it as an overlay subview, and start the status poll loop. Runs on
    /// the main thread (Tauri's `with_webview` callback guarantees this).
    pub fn install(engine: &Arc<AirPlayEngine>, host_view: &NSView, app: &AppHandle) {
        let Some(mtm) = MainThreadMarker::new() else {
            tracing::warn!("AirPlay install not on main thread; skipping");
            return;
        };

        // SAFETY: all calls below run on the main thread with valid args.
        unsafe {
            let player = AVPlayer::new(mtm);
            player.setAllowsExternalPlayback(true);
            player.setUsesExternalPlaybackWhileExternalScreenIsActive(true);

            let detector = AVRouteDetector::new();
            detector.setRouteDetectionEnabled(true);

            let picker = AVRoutePickerView::new(mtm);
            picker.setPlayer(Some(&player));
            picker.setPrioritizesVideoDevices(true);
            // White glyph so it reads on the dark control bar; system blue when
            // a route is active.
            let white = NSColor::whiteColor();
            picker.setRoutePickerButtonColor_forState(
                Some(&white),
                AVRoutePickerViewButtonState::Normal,
            );
            picker.setHidden(true);
            let delegate = RoutePickerDelegate::new(app.clone());
            picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

            let picker_view: &NSView = &picker;
            host_view.addSubview(picker_view);

            // Leak the Retained values: these objects live for the app's
            // lifetime. The picker is also retained by its superview and the
            // delegate by the picker, but we forget explicit handles so the
            // raw pointers we store below stay valid.
            let player_ptr = Retained::as_ptr(&player) as usize;
            let detector_ptr = Retained::as_ptr(&detector) as usize;
            let picker_ptr = Retained::as_ptr(&picker) as usize;
            std::mem::forget(player);
            std::mem::forget(detector);
            std::mem::forget(picker);
            std::mem::forget(delegate);

            if let Ok(mut guard) = engine.inner.lock() {
                *guard = Some(AirPlayInner {
                    player: player_ptr,
                    detector: detector_ptr,
                    picker: picker_ptr,
                    host_view: host_view as *const NSView as usize,
                });
            }
        }

        spawn_status_loop(engine.clone(), app.clone());
    }

    /// Emit `airplay:status` periodically. Stays quiet while nothing is
    /// reachable and nothing is loaded, so an idle app doesn't spam events.
    fn spawn_status_loop(engine: Arc<AirPlayEngine>, app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(800)).await;
                let engine = engine.clone();
                let app2 = app.clone();
                let res = app.run_on_main_thread(move || {
                    let Ok(guard) = engine.inner.lock() else {
                        return;
                    };
                    let Some(inner) = guard.as_ref() else {
                        return;
                    };
                    let status = inner.snapshot();
                    // Skip emitting when fully idle (no route, nothing loaded)
                    // to avoid a steady event drip on the home screen.
                    if !status.available && !status.active {
                        return;
                    }
                    let _ = app2.emit("airplay:status", status);
                });
                if res.is_err() {
                    // App is shutting down (main thread gone) — stop polling.
                    break;
                }
            }
        });
    }

    // -- Operations dispatched from commands ----------------------------------

    fn with_main<F>(engine: &Arc<AirPlayEngine>, app: &AppHandle, f: F)
    where
        F: FnOnce(&AirPlayInner) + Send + 'static,
    {
        let engine = engine.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(guard) = engine.inner.lock() {
                if let Some(inner) = guard.as_ref() {
                    f(inner);
                }
            }
        });
    }

    pub fn set_picker_rect(engine: &Arc<AirPlayEngine>, app: &AppHandle, rect: Option<VideoRect>) {
        with_main(engine, app, move |inner| inner.set_picker_rect(rect));
    }

    pub fn engage(engine: &Arc<AirPlayEngine>, app: &AppHandle, url: String, position_sec: f64) {
        with_main(engine, app, move |inner| {
            if let Some(mtm) = MainThreadMarker::new() {
                inner.load(&url, position_sec, mtm);
            }
        });
    }

    pub fn disengage(engine: &Arc<AirPlayEngine>, app: &AppHandle) {
        with_main(engine, app, |inner| inner.clear());
    }

    pub fn toggle_pause(engine: &Arc<AirPlayEngine>, app: &AppHandle) {
        with_main(engine, app, |inner| inner.toggle_pause());
    }

    pub fn seek_absolute(engine: &Arc<AirPlayEngine>, app: &AppHandle, position_sec: f64) {
        with_main(engine, app, move |inner| inner.seek_absolute(position_sec));
    }

    pub fn set_volume(engine: &Arc<AirPlayEngine>, app: &AppHandle, volume: f64) {
        with_main(engine, app, move |inner| inner.set_volume(volume));
    }

    pub fn set_muted(engine: &Arc<AirPlayEngine>, app: &AppHandle, muted: bool) {
        with_main(engine, app, move |inner| inner.set_muted(muted));
    }

    /// Read a status snapshot synchronously (blocks briefly on the main thread).
    /// Used by the `airplay_status` command; control paths use the poll loop.
    pub fn status_blocking(engine: &Arc<AirPlayEngine>, app: &AppHandle) -> AirPlayStatus {
        let (tx, rx) = std::sync::mpsc::channel();
        let engine = engine.clone();
        let dispatched = app.run_on_main_thread(move || {
            let status = engine
                .inner
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|inner| inner.snapshot()))
                .unwrap_or_default();
            let _ = tx.send(status);
        });
        if dispatched.is_err() {
            return AirPlayStatus::default();
        }
        rx.recv_timeout(Duration::from_millis(500))
            .unwrap_or_default()
    }

    /// Whether an AirPlay engine was installed (always true on macOS once
    /// `install` ran; the helper exists so the non-macOS stub can return false).
    pub fn is_installed(engine: &Arc<AirPlayEngine>) -> bool {
        engine.inner.lock().map(|g| g.is_some()).unwrap_or(false)
    }
}

// -- Cross-platform façade ---------------------------------------------------
//
// Commands call these regardless of platform. On macOS they forward to `imp`;
// elsewhere they are no-ops and `status_blocking` reports an unavailable engine.

use std::sync::Arc;

#[cfg(target_os = "macos")]
pub use imp::install_picker;

impl AirPlayEngine {
    pub fn set_picker_rect(
        self: &Arc<Self>,
        _app: &tauri::AppHandle,
        _rect: Option<nova_mpv_player::VideoRect>,
    ) {
        #[cfg(target_os = "macos")]
        imp::set_picker_rect(self, _app, _rect);
    }

    pub fn engage(self: &Arc<Self>, _app: &tauri::AppHandle, _url: String, _position_sec: f64) {
        #[cfg(target_os = "macos")]
        imp::engage(self, _app, _url, _position_sec);
    }

    pub fn disengage(self: &Arc<Self>, _app: &tauri::AppHandle) {
        #[cfg(target_os = "macos")]
        imp::disengage(self, _app);
    }

    pub fn toggle_pause(self: &Arc<Self>, _app: &tauri::AppHandle) {
        #[cfg(target_os = "macos")]
        imp::toggle_pause(self, _app);
    }

    pub fn seek_absolute(self: &Arc<Self>, _app: &tauri::AppHandle, _position_sec: f64) {
        #[cfg(target_os = "macos")]
        imp::seek_absolute(self, _app, _position_sec);
    }

    pub fn set_volume(self: &Arc<Self>, _app: &tauri::AppHandle, _volume: f64) {
        #[cfg(target_os = "macos")]
        imp::set_volume(self, _app, _volume);
    }

    pub fn set_muted(self: &Arc<Self>, _app: &tauri::AppHandle, _muted: bool) {
        #[cfg(target_os = "macos")]
        imp::set_muted(self, _app, _muted);
    }

    pub fn status(self: &Arc<Self>, _app: &tauri::AppHandle) -> AirPlayStatus {
        #[cfg(target_os = "macos")]
        {
            imp::status_blocking(self, _app)
        }
        #[cfg(not(target_os = "macos"))]
        {
            AirPlayStatus::default()
        }
    }

    pub fn is_installed(self: &Arc<Self>) -> bool {
        #[cfg(target_os = "macos")]
        {
            imp::is_installed(self)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}
