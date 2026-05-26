//! macOS `CAOpenGLLayer` subclass that hosts libmpv's render output.
//!
//! Architecture:
//!
//! 1. The layer is created at app start and installed as a sublayer of the
//!    WKWebView's own layer tree (see `native_player_view.rs`). Because it
//!    lives inside the same CA layer tree as the webview, AppKit handles
//!    z-order, live resize, native fullscreen, and PiP transitions for free.
//! 2. The layer also serves as the engine's [`RenderLifecycleHandler`].
//!    When [`nova_mpv_player::MpvEngine::open`] succeeds, it synchronously
//!    calls `attach` — which creates an [`MpvRenderContext`] against the
//!    just-initialized handle and registers an update callback. When the
//!    session closes, the engine calls `detach` and the render context is
//!    dropped (which calls `mpv_render_context_free`) before the handle is
//!    destroyed.
//! 3. libmpv's update callback fires on its own thread. We dispatch a
//!    `setNeedsDisplay` to the main queue; Core Animation invokes our
//!    `drawInCGLContext:` later, at which point the CGL context is current
//!    and we call [`MpvRenderContext::render`].

#![cfg(target_os = "macos")]
// OpenGL is deprecated on macOS but libmpv's render API supports OpenGL and
// D3D11 only — there's no Metal hook. Until libmpv adds a Metal output target,
// we silence the deprecation noise locally.
#![allow(deprecated)]

use std::ffi::{c_char, c_int, c_void, CString};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};

use nova_mpv_player::{MpvRawHandle, MpvRenderContext, RenderLifecycleHandler};

use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_foundation::NSRect;
use objc2_quartz_core::{CAOpenGLLayer, CALayer};

// -- libdispatch FFI ---------------------------------------------------------
//
// `dispatch_get_main_queue` is a C macro that expands to `(&_dispatch_main_q)`,
// so there's no symbol to link against. We declare the underlying global and
// take its address.

unsafe extern "C" {
    static _dispatch_main_q: c_void;

    fn dispatch_async_f(
        queue: *mut c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
    fn dispatch_sync_f(
        queue: *mut c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
}

fn dispatch_main_queue() -> *mut c_void {
    // SAFETY: `_dispatch_main_q` is a process-global static defined by
    // libdispatch (always linked on macOS via Foundation).
    unsafe { (&_dispatch_main_q as *const c_void) as *mut c_void }
}

// -- dlopen FFI --------------------------------------------------------------

const RTLD_LAZY: c_int = 0x1;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

// -- GL FFI (loaded via dlsym from OpenGL.framework) -------------------------

/// `glGetIntegerv(GL_FRAMEBUFFER_BINDING, &out)` — needed to find out which
/// FBO CAOpenGLLayer bound for us before invoking `drawInCGLContext:`. That
/// FBO is what we must hand to libmpv, NOT the default (0).
const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;

type GlGetIntegerv = unsafe extern "C" fn(pname: u32, params: *mut c_int);

fn current_framebuffer(opengl_lib: *mut c_void) -> i32 {
    if opengl_lib.is_null() {
        return 0;
    }
    let name = match std::ffi::CString::new("glGetIntegerv") {
        Ok(n) => n,
        Err(_) => return 0,
    };
    // SAFETY: opengl_lib is a live dlopen handle for OpenGL.framework, name
    // is a valid NUL-terminated string. `glGetIntegerv` is exported by the
    // framework.
    let sym = unsafe { dlsym(opengl_lib, name.as_ptr()) };
    if sym.is_null() {
        return 0;
    }
    // SAFETY: transmuting a non-null dlsym result to the documented GL
    // function signature.
    let f: GlGetIntegerv = unsafe { std::mem::transmute(sym) };
    let mut fbo: c_int = 0;
    // SAFETY: `&mut fbo` is a valid out-pointer for a single GLint.
    unsafe { f(GL_FRAMEBUFFER_BINDING, &mut fbo) };
    fbo
}

// -- Layer ivars -------------------------------------------------------------

pub struct MpvLayerIvars {
    inner: Mutex<MpvLayerInner>,
    /// Handle to OpenGL.framework, opened lazily on first `attach`. Held as
    /// an atomic so the `get_proc_address` callback can read it without
    /// taking the mutex.
    opengl_lib: AtomicPtr<c_void>,
}

struct MpvLayerInner {
    /// Handle awaiting attach to a render context. Created on the next
    /// `drawInCGLContext:` call (which is the only place CGL is current).
    pending_handle: Option<MpvRawHandle>,
    render_ctx: Option<MpvRenderContext>,
    /// `Some` after a failed lazy attach; surfaced to logs only.
    attach_error: Option<String>,
}

impl MpvLayerIvars {
    fn new() -> Self {
        Self {
            inner: Mutex::new(MpvLayerInner {
                pending_handle: None,
                render_ctx: None,
                attach_error: None,
            }),
            opengl_lib: AtomicPtr::new(ptr::null_mut()),
        }
    }
}

// -- CAOpenGLLayer subclass --------------------------------------------------

define_class!(
    #[unsafe(super(CAOpenGLLayer, CALayer))]
    #[name = "NovaMpvLayer"]
    #[ivars = MpvLayerIvars]
    pub struct MpvLayer;

    impl MpvLayer {
        /// CoreAnimation polls this before each frame; returning true causes
        /// `drawInCGLContext:` to be invoked. We say "yes" whenever the
        /// render context exists — libmpv itself decides internally whether
        /// it has fresh pixels.
        #[unsafe(method(canDrawInCGLContext:pixelFormat:forLayerTime:displayTime:))]
        fn can_draw_in_cgl_context(
            &self,
            _cgl_context: *mut c_void,
            _pixel_format: *mut c_void,
            _layer_time: f64,
            _display_time: *const c_void,
        ) -> Bool {
            let inner = match self.ivars().inner.lock() {
                Ok(g) => g,
                Err(_) => return Bool::NO,
            };
            // Draw if we already have a context OR a handle waiting for
            // attach (we need to be invoked at least once to do the lazy
            // creation).
            Bool::new(inner.render_ctx.is_some() || inner.pending_handle.is_some())
        }

        /// CoreAnimation has made `cgl_context` current; ask libmpv to paint
        /// the current frame into the default framebuffer (fbo=0).
        #[unsafe(method(drawInCGLContext:pixelFormat:forLayerTime:displayTime:))]
        fn draw_in_cgl_context(
            &self,
            cgl_context: *mut c_void,
            pixel_format: *mut c_void,
            layer_time: f64,
            display_time: *const c_void,
        ) {
            self.draw_mpv_frame();
            // Always call super so CAOpenGLLayer flushes the GL context to
            // its backing IOSurface.
            // SAFETY: signature matches the documented CAOpenGLLayer method.
            unsafe {
                let _: () = msg_send![
                    super(self),
                    drawInCGLContext: cgl_context,
                    pixelFormat: pixel_format,
                    forLayerTime: layer_time,
                    displayTime: display_time
                ];
            }
        }
    }
);

impl MpvLayer {
    pub fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(MpvLayerIvars::new());
        // SAFETY: `init` is the standard NSObject initializer; the alloc came
        // from objc2 which gives us a valid uninitialized object.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        // `setAsynchronous=NO`: CA only redraws when we call setNeedsDisplay,
        // which we do from libmpv's update callback. Saves CPU vs. polling.
        // `setNeedsDisplayOnBoundsChange=YES`: redraw automatically on layout.
        this.setAsynchronous(false);
        this.setNeedsDisplayOnBoundsChange(true);
        this
    }

    fn draw_mpv_frame(&self) {
        // Pin the layer to its superlayer's bounds. CALayer has no built-in
        // autoresize for sublayers without a `layoutManager`, so the frame
        // we set at install time (often 0×0 before the window has laid out)
        // never grows. Sync each draw — cheap, and CA's no-op when bounds
        // match.
        let parent_bounds_opt = self.superlayer().map(|p| p.bounds());
        if let Some(parent_bounds) = parent_bounds_opt {
            let my_frame = self.frame();
            if (parent_bounds.size.width - my_frame.size.width).abs() > 0.5
                || (parent_bounds.size.height - my_frame.size.height).abs() > 0.5
                || (parent_bounds.origin.x - my_frame.origin.x).abs() > 0.5
                || (parent_bounds.origin.y - my_frame.origin.y).abs() > 0.5
            {
                self.setFrame(parent_bounds);
            }
        }

        // CGL is current on this thread because CoreAnimation called us
        // inside `drawInCGLContext:`. That's a hard requirement for both
        // `mpv_render_context_create` and `mpv_render_context_render`.
        let mut inner = match self.ivars().inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };

        // Lazy attach: turn the pending handle into a render context now
        // that GL is current. On failure we drop the handle and stash the
        // error so we don't keep retrying every frame.
        if inner.render_ctx.is_none() {
            if let Some(handle) = inner.pending_handle.take() {
                let ctx_ptr = (&self.ivars().opengl_lib as *const AtomicPtr<c_void>)
                    as *mut c_void;
                match MpvRenderContext::new(handle, ctx_ptr, get_proc_address_trampoline) {
                    Ok(ctx) => {
                        let layer_ptr: *const MpvLayer = self;
                        // SAFETY: `layer_ptr` is `self`; the layer is held
                        // alive by the MpvLayerHandler (and through it, the
                        // engine) for the duration of the session.
                        unsafe {
                            ctx.set_update_callback(
                                Some(redraw_trampoline),
                                layer_ptr as *mut c_void,
                            );
                        }
                        inner.render_ctx = Some(ctx);
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        tracing::error!(error = %msg, "mpv render context create failed");
                        inner.attach_error = Some(msg);
                        return;
                    }
                }
            }
        }

        let Some(render) = inner.render_ctx.as_ref() else {
            return;
        };

        let bounds: NSRect = self.bounds();
        let scale: f64 = self.contentsScale();
        let width = (bounds.size.width * scale).round().max(1.0) as i32;
        let height = (bounds.size.height * scale).round().max(1.0) as i32;

        // CAOpenGLLayer binds its OWN framebuffer object before invoking
        // `drawInCGLContext:`. Passing fbo=0 would render into the default
        // (window) framebuffer which CA doesn't composite from — that's
        // exactly the "audio works, screen is black" symptom. Query the
        // currently bound FBO and hand it to libmpv.
        let fbo = current_framebuffer(self.ivars().opengl_lib.load(Ordering::SeqCst));

        // CAOpenGLLayer's framebuffer is bottom-up; libmpv expects flip_y=true.
        render.render(fbo, width, height, true);
    }
}

// SAFETY: `MpvLayer` is a Retained<...> objc object; AppKit holds it on the
// main thread once added to a layer tree. We only access the ivars (a Mutex)
// from background threads in attach/detach — no Objective-C method calls
// from non-main threads except `setNeedsDisplay` via dispatch_async.
unsafe impl Send for MpvLayer {}
unsafe impl Sync for MpvLayer {}

// -- RenderLifecycleHandler --------------------------------------------------

/// Bridges [`MpvEngine`] → [`MpvLayer`]. The engine calls `attach`/`detach`
/// synchronously from whichever thread opens/closes the session.
pub struct MpvLayerHandler {
    layer: Retained<MpvLayer>,
}

impl MpvLayerHandler {
    pub fn new(layer: Retained<MpvLayer>) -> Arc<Self> {
        Arc::new(Self { layer })
    }
}

// SAFETY: `Retained<MpvLayer>` is a refcount handle; cloning/dropping it is
// thread-safe. The only AppKit interaction we trigger from non-main threads
// is `setNeedsDisplay`, which we explicitly dispatch to the main queue via
// `dispatch_async_f`. Direct ivar (Mutex) access is fine from any thread.
unsafe impl Send for MpvLayerHandler {}
unsafe impl Sync for MpvLayerHandler {}

impl RenderLifecycleHandler for MpvLayerHandler {
    fn attach(&self, handle: MpvRawHandle) -> Result<(), String> {
        let ivars = self.layer.ivars();

        // Open OpenGL.framework once. `get_proc_address` reads this without
        // taking the mutex; it has to be live before the (deferred) render
        // context creation in `drawInCGLContext:`.
        if ivars.opengl_lib.load(Ordering::SeqCst).is_null() {
            let path = CString::new("/System/Library/Frameworks/OpenGL.framework/OpenGL")
                .map_err(|_| "failed to build OpenGL framework path".to_string())?;
            // SAFETY: `path` outlives the call.
            let opengl = unsafe { dlopen(path.as_ptr(), RTLD_LAZY) };
            if opengl.is_null() {
                return Err("dlopen(OpenGL.framework) failed".to_string());
            }
            let _ = ivars.opengl_lib.compare_exchange(
                ptr::null_mut(),
                opengl,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }

        // Stash the handle for the next `drawInCGLContext:` to consume.
        // We can't create the render context here — `mpv_render_context_create`
        // requires the OpenGL context to be current on the calling thread,
        // and CAOpenGLLayer only makes its CGL context current inside its
        // draw callbacks.
        {
            let mut inner = ivars
                .inner
                .lock()
                .map_err(|_| "layer mutex poisoned".to_string())?;
            inner.pending_handle = Some(handle);
            inner.attach_error = None;
        }

        // Kick CoreAnimation so it calls our draw method and lazy-creates
        // the render context.
        request_redraw_main(&self.layer);

        Ok(())
    }

    fn detach(&self) {
        let ivars = self.layer.ivars();

        // Take the render context out under the mutex, then RELEASE the
        // mutex before doing anything expensive. Holding it across an FFI
        // call into libmpv would deadlock the main thread, which is
        // currently blocked in `canDrawInCGLContext` waiting on the same
        // mutex (per the macOS crash trace).
        let render_ctx = {
            let mut inner = match ivars.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            inner.pending_handle = None;
            inner.render_ctx.take()
        };

        let Some(render_ctx) = render_ctx else {
            // Nothing was rendering — no FFI needed.
            return;
        };

        // `mpv_render_context_free` needs the GL context current to free its
        // OpenGL resources cleanly. CAOpenGLLayer only makes CGL current on
        // its own thread inside the draw callbacks, so we dispatch the drop
        // to the main thread, synchronously — this guarantees the free
        // completes before the engine destroys the underlying mpv handle.
        let boxed = Box::into_raw(Box::new(Some(render_ctx)));
        // SAFETY: `boxed` is freshly leaked; main will reconstitute the Box
        // and drop it, ensuring the MpvRenderContext (and the libmpv free
        // call) runs exactly once on the main thread.
        unsafe {
            dispatch_sync_f(
                dispatch_main_queue(),
                boxed as *mut c_void,
                drop_render_ctx_on_main,
            );
        }
    }
}

unsafe extern "C" fn drop_render_ctx_on_main(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    tracing::debug!("drop_render_ctx_on_main: freeing mpv render context on main thread");
    // SAFETY: ctx was set in `detach` via `Box::into_raw` of
    // `Box<Option<MpvRenderContext>>`. Reconstructing the Box here transfers
    // ownership; dropping it calls `mpv_render_context_free`.
    let boxed: Box<Option<MpvRenderContext>> = unsafe { Box::from_raw(ctx as *mut _) };
    drop(boxed);
    tracing::debug!("drop_render_ctx_on_main: free complete");
}

// -- Trampolines -------------------------------------------------------------

unsafe extern "C" fn get_proc_address_trampoline(
    ctx: *mut c_void,
    name: *const c_char,
) -> *mut c_void {
    if ctx.is_null() || name.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `ctx` is a `*const AtomicPtr<c_void>` we passed in from
    // `attach`. The AtomicPtr lives inside the layer's ivars, which outlive
    // the render context.
    let atomic = unsafe { &*(ctx as *const AtomicPtr<c_void>) };
    let handle = atomic.load(Ordering::SeqCst);
    if handle.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `handle` is a live dlopen handle to OpenGL.framework.
    unsafe { dlsym(handle, name) }
}

unsafe extern "C" fn redraw_trampoline(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    // Fired from libmpv's thread. We can't touch AppKit here — bounce to
    // the main queue.
    let queue = dispatch_main_queue();
    unsafe { dispatch_async_f(queue, ctx, set_needs_display_on_main) };
}

unsafe extern "C" fn set_needs_display_on_main(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    // SAFETY: `ctx` is the `*const MpvLayer` we passed to
    // `mpv_render_context_set_update_callback`. The layer is retained by
    // `MpvLayerHandler` for app lifetime; the handler is held by `MpvEngine`.
    let layer: &MpvLayer = unsafe { &*(ctx as *const MpvLayer) };
    // `setNeedsDisplay` is a `CALayer` method, safe to call on main (which
    // we are on after the dispatch).
    layer.setNeedsDisplay();
}

fn request_redraw_main(layer: &MpvLayer) {
    let layer_ptr: *const MpvLayer = layer;
    let queue = dispatch_main_queue();
    // SAFETY: see `set_needs_display_on_main`. Caller holds a Retained to the
    // layer, so it outlives the dispatched work.
    unsafe {
        dispatch_async_f(queue, layer_ptr as *mut c_void, set_needs_display_on_main);
    }
}
