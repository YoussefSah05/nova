//! Install Nova's mpv-backed CAOpenGLLayer into the Tauri window.
//!
//! The old implementation added an `NSOpenGLView` as a *sibling* of the
//! WKWebView (under the same superview). That fought Tauri's layout for
//! z-order, live resize, fullscreen, and PiP transitions — see the four
//! "fix(player)" commits leading up to this rewrite.
//!
//! The new layout is layer-based: we attach a [`MpvLayer`] (CAOpenGLLayer
//! subclass) as a sublayer of the WKWebView's own backing layer. AppKit's
//! Core Animation compositor handles every transition for us — bounds
//! changes propagate via `setNeedsDisplayOnBoundsChange`, fullscreen and PiP
//! reparent the host layer transparently, and z-order is "below the webview
//! contents" by default.

#![cfg(target_os = "macos")]

use std::sync::Arc;

use nova_mpv_player::MpvEngine;

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSColor, NSView, NSWindow};
use objc2_foundation::{ns_string, NSNumber, NSObjectNSKeyValueCoding};
use objc2_quartz_core::{CALayer, CAAutoresizingMask};
use objc2_web_kit::WKWebView;
use tauri::{webview::PlatformWebview, Runtime, WebviewWindow};

use crate::mpv_layer::{MpvLayer, MpvLayerHandler};

pub fn install_mpv_render_view<R: Runtime>(
    window: &WebviewWindow<R>,
    engine: Arc<MpvEngine>,
) -> tauri::Result<()> {
    window.with_webview(move |webview| {
        if let Err(error) = install_macos_player_layer(&webview, &engine) {
            tracing::warn!(%error, "failed to install native mpv layer");
        }
    })
}

fn install_macos_player_layer(
    webview: &PlatformWebview,
    engine: &Arc<MpvEngine>,
) -> Result<(), String> {
    let _ = MainThreadMarker::new()
        .ok_or("Tauri did not run native view installation on the main thread")?;

    // SAFETY: `with_webview` runs on the main thread and Tauri hands us live
    // AppKit/WebKit pointers for the main webview. The MpvLayer is retained
    // by the handler we register with the engine; the engine owns the
    // handler for the rest of the app's lifetime.
    unsafe {
        let wk_ptr = webview.inner();
        let window_ptr = webview.ns_window();
        if wk_ptr.is_null() || window_ptr.is_null() {
            return Err("Tauri returned null native webview/window pointer".to_string());
        }

        let wk_webview: &WKWebView = &*wk_ptr.cast::<WKWebView>();
        let webview_view: &NSView = &*wk_ptr.cast::<NSView>();
        let ns_window: &NSWindow = &*window_ptr.cast::<NSWindow>();

        // Make the WKWebView see-through so our video layer underneath shows.
        // `drawsBackground` is a private but stable WKWebView KVC key.
        let no = NSNumber::numberWithBool(false);
        wk_webview.setValue_forKey(Some(&no), ns_string!("drawsBackground"));
        ns_window.setOpaque(false);
        let clear = NSColor::clearColor();
        ns_window.setBackgroundColor(Some(&clear));

        // Force the WKWebView to be layer-backed so it has a `layer` we can
        // host sublayers in.
        webview_view.setWantsLayer(true);
        let host_layer: Retained<CALayer> = webview_view
            .layer()
            .ok_or_else(|| "WKWebView has no backing layer".to_string())?;

        let mpv_layer = MpvLayer::new();

        // Track the host layer's full bounds.
        let host_bounds = host_layer.bounds();
        mpv_layer.setFrame(host_bounds);
        mpv_layer.setAutoresizingMask(
            CAAutoresizingMask::LayerWidthSizable | CAAutoresizingMask::LayerHeightSizable,
        );

        // High-DPI: pick up the window's actual backing scale so we render
        // at native pixel resolution.
        let scale: f64 = ns_window.backingScaleFactor();
        mpv_layer.setContentsScale(scale);

        // Insert as the very first sublayer so the webview content sits on
        // top.
        host_layer.insertSublayer_atIndex(&mpv_layer, 0);

        // Register the layer as the engine's render lifecycle handler.
        let handler = MpvLayerHandler::new(mpv_layer);
        engine
            .set_render_handler(handler)
            .map_err(|e| format!("failed to register mpv render handler: {e}"))?;
    }

    Ok(())
}
