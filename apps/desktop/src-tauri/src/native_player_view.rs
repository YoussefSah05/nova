#[cfg(target_os = "macos")]
use nova_mpv_player::MpvEngine;

#[cfg(target_os = "macos")]
use objc2::{rc::Retained, MainThreadMarker};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSView, NSWindow, NSWindowOrderingMode};
#[cfg(target_os = "macos")]
use objc2_foundation::{ns_string, NSNumber, NSObjectNSKeyValueCoding};
#[cfg(target_os = "macos")]
use objc2_web_kit::WKWebView;
#[cfg(target_os = "macos")]
use tauri::{webview::PlatformWebview, Runtime, WebviewWindow};

#[cfg(target_os = "macos")]
pub fn install_mpv_embed_view<R: Runtime>(
    window: &WebviewWindow<R>,
    engine: MpvEngine,
) -> tauri::Result<()> {
    window.with_webview(move |webview| {
        if let Err(error) = install_macos_player_view(&webview, &engine) {
            tracing::warn!(%error, "failed to install native mpv embed view");
        }
    })
}

#[cfg(target_os = "macos")]
fn install_macos_player_view(webview: &PlatformWebview, engine: &MpvEngine) -> Result<(), String> {
    let mtm = MainThreadMarker::new()
        .ok_or("Tauri did not execute native-view installation on the main thread")?;

    // SAFETY: `with_webview` runs on the main thread and Tauri provides live AppKit/WebKit
    // pointers for the main webview. The created NSView is retained by AppKit after
    // `addSubview`, and libmpv only receives the stable NSView pointer as its `wid` target.
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

        let player_view = NSView::initWithFrame(mtm.alloc::<NSView>(), webview_view.frame());
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

        let no = NSNumber::numberWithBool(false);
        wk_webview.setValue_forKey(Some(&no), ns_string!("drawsBackground"));
        let clear = NSColor::clearColor();
        ns_window.setOpaque(false);
        ns_window.setBackgroundColor(Some(&clear));

        engine
            .set_embed_wid(Retained::as_ptr(&player_view) as u64)
            .map_err(|e| format!("failed to register mpv embed view: {e}"))?;
    }

    Ok(())
}
