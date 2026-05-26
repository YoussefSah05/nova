#![cfg_attr(target_os = "macos", allow(deprecated))]

#[cfg(target_os = "macos")]
use nova_mpv_player::{MpvEngine, MpvRenderSurface, MpvRenderSurfaceRegistration};

#[cfg(target_os = "macos")]
use objc2::{rc::Retained, MainThreadMarker};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSOpenGLContext, NSOpenGLView, NSView, NSWindow,
    NSWindowOrderingMode,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{ns_string, NSNumber, NSObjectNSKeyValueCoding};
#[cfg(target_os = "macos")]
use objc2_web_kit::WKWebView;
#[cfg(target_os = "macos")]
use std::{
    ffi::{c_char, c_int, c_void, CString},
    ptr,
    sync::Arc,
};
#[cfg(target_os = "macos")]
use tauri::{webview::PlatformWebview, Runtime, WebviewWindow};

#[cfg(target_os = "macos")]
pub fn install_mpv_render_view<R: Runtime>(
    window: &WebviewWindow<R>,
    engine: MpvEngine,
) -> tauri::Result<()> {
    window.with_webview(move |webview| {
        if let Err(error) = install_macos_player_view(&webview, &engine) {
            tracing::warn!(%error, "failed to install native mpv render view");
        }
    })
}

#[cfg(target_os = "macos")]
fn install_macos_player_view(webview: &PlatformWebview, engine: &MpvEngine) -> Result<(), String> {
    let mtm = MainThreadMarker::new()
        .ok_or("Tauri did not execute native-view installation on the main thread")?;

    // SAFETY: `with_webview` runs on the main thread and Tauri provides live AppKit/WebKit
    // pointers for the main webview. The created OpenGL view is retained by AppKit after
    // `addSubview`, and libmpv receives only stable callbacks into this owned surface.
    unsafe {
        let wk_ptr = webview.inner();
        let window_ptr = webview.ns_window();
        if wk_ptr.is_null() || window_ptr.is_null() {
            return Err("Tauri returned a null native webview/window pointer".to_string());
        }

        let wk_webview: &WKWebView = &*wk_ptr.cast::<WKWebView>();
        let webview_view: &NSView = &*wk_ptr.cast::<NSView>();
        let ns_window: &NSWindow = &*window_ptr.cast::<NSWindow>();
        let superview = webview_view
            .superview()
            .ok_or_else(|| "WKWebView is not attached to an NSView hierarchy".to_string())?;

        let mut player_frame = superview.bounds();
        if player_frame.size.width <= 0.0 || player_frame.size.height <= 0.0 {
            player_frame = webview_view.frame();
        }

        let pixel_format = NSOpenGLView::defaultPixelFormat(mtm);
        let player_view = NSOpenGLView::initWithFrame_pixelFormat(
            mtm.alloc::<NSOpenGLView>(),
            player_frame,
            Some(&pixel_format),
        )
        .ok_or_else(|| "failed to create native OpenGL player view".to_string())?;
        let player_context = player_view
            .openGLContext()
            .ok_or_else(|| "native OpenGL player view did not create a context".to_string())?;
        player_view.setWantsLayer(true);
        player_view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        superview.addSubview_positioned_relativeTo(
            &player_view,
            NSWindowOrderingMode::Below,
            Some(webview_view),
        );
        player_context.setView(Some(&player_view), mtm);

        let no = NSNumber::numberWithBool(false);
        wk_webview.setValue_forKey(Some(&no), ns_string!("drawsBackground"));
        let clear = NSColor::clearColor();
        ns_window.setOpaque(false);
        ns_window.setBackgroundColor(Some(&clear));

        let surface = Arc::new(MacosRenderSurface::new(player_view, player_context)?);
        let callbacks = MpvRenderSurface {
            callback_ctx: Arc::as_ptr(&surface) as usize,
            make_current: macos_render_make_current,
            flush: macos_render_flush,
            size: macos_render_size,
            get_proc_address_ctx: Arc::as_ptr(&surface) as *mut c_void as usize,
            get_proc_address: macos_get_proc_address,
        };
        engine
            .set_render_surface(MpvRenderSurfaceRegistration::new(callbacks, surface))
            .map_err(|e| format!("failed to register mpv render surface: {e}"))?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
struct MacosRenderSurface {
    view: Retained<NSOpenGLView>,
    context: Retained<NSOpenGLContext>,
    opengl: *mut c_void,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacosRenderSurface {}
#[cfg(target_os = "macos")]
unsafe impl Sync for MacosRenderSurface {}

#[cfg(target_os = "macos")]
impl MacosRenderSurface {
    fn new(
        view: Retained<NSOpenGLView>,
        context: Retained<NSOpenGLContext>,
    ) -> Result<Self, String> {
        let path = CString::new("/System/Library/Frameworks/OpenGL.framework/OpenGL")
            .map_err(|_| "failed to create OpenGL framework path".to_string())?;
        let opengl = unsafe { dlopen(path.as_ptr(), RTLD_LAZY) };
        if opengl.is_null() {
            return Err("failed to load OpenGL framework for libmpv render API".to_string());
        }

        Ok(Self {
            view,
            context,
            opengl,
        })
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn macos_render_make_current(ctx: usize) -> bool {
    let Some(surface) = (ctx as *const MacosRenderSurface).as_ref() else {
        return false;
    };
    surface.context.makeCurrentContext();
    true
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn macos_render_flush(ctx: usize) {
    let Some(surface) = (ctx as *const MacosRenderSurface).as_ref() else {
        return;
    };
    surface.context.flushBuffer();
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn macos_render_size(ctx: usize, width: *mut c_int, height: *mut c_int) -> bool {
    let Some(surface) = (ctx as *const MacosRenderSurface).as_ref() else {
        return false;
    };
    if width.is_null() || height.is_null() {
        return false;
    }

    let view: &NSView = &*Retained::as_ptr(&surface.view).cast::<NSView>();
    let bounds = view.bounds();
    let scale = view
        .window()
        .as_deref()
        .map(NSWindow::backingScaleFactor)
        .unwrap_or(1.0);
    *width = (bounds.size.width * scale).round().max(1.0) as c_int;
    *height = (bounds.size.height * scale).round().max(1.0) as c_int;
    true
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn macos_get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    let Some(surface) = (ctx as *const MacosRenderSurface).as_ref() else {
        return ptr::null_mut();
    };
    if name.is_null() || surface.opengl.is_null() {
        return ptr::null_mut();
    }
    dlsym(surface.opengl, name)
}

#[cfg(target_os = "macos")]
const RTLD_LAZY: c_int = 0x1;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
