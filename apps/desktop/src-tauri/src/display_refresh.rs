//! Let the interface render at the display's full refresh rate.
//!
//! WebKit paces rendering updates (requestAnimationFrame, animations, newly
//! painted content) near 60 fps by default, even on 120 Hz ProMotion
//! displays. That pacing is WebKit's `PreferPageRenderingUpdatesNear60FPSEnabled`
//! feature, reachable only through WebKit's internal feature list
//! (`+[WKPreferences _features]`, `-[WKPreferences _setEnabled:forFeature:]`).
//! Both are looked up at runtime: when a WebKit release renames or removes
//! them, nothing happens and Nova keeps the default 60 fps pacing.

#![cfg(target_os = "macos")]

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{msg_send, sel};
use objc2_foundation::{NSArray, NSString};
use tauri::{Runtime, WebviewWindow};

const NEAR_60_FPS: &str = "PreferPageRenderingUpdatesNear60FPSEnabled";

/// Apply the preference to `window`'s webview. Runs on the main thread.
pub fn apply<R: Runtime>(window: &WebviewWindow<R>, high_refresh_rate: bool) {
    let result = window.with_webview(move |webview| {
        let wk = webview.inner();
        if wk.is_null() {
            return;
        }
        // SAFETY: `with_webview` runs on the main thread with a live
        // WKWebView; every selector is checked before it is sent.
        let applied = unsafe { set_near_60fps(&*wk.cast::<AnyObject>(), !high_refresh_rate) };
        match applied {
            Ok(()) => tracing::info!(high_refresh_rate, "display refresh preference applied"),
            Err(why) => tracing::warn!(
                why,
                "display refresh preference unavailable; keeping 60 fps"
            ),
        }
    });
    if let Err(error) = result {
        tracing::warn!(%error, "could not reach the webview to set its refresh rate");
    }
}

unsafe fn responds(object: &AnyObject, selector: objc2::runtime::Sel) -> bool {
    let yes: Bool = msg_send![object, respondsToSelector: selector];
    yes.as_bool()
}

unsafe fn set_near_60fps(webview: &AnyObject, near_60: bool) -> Result<(), &'static str> {
    let class = AnyClass::get(c"WKPreferences").ok_or("WKPreferences missing")?;
    let class_object: &AnyObject = &*(class as *const AnyClass).cast::<AnyObject>();
    if !responds(class_object, sel!(_features)) {
        return Err("feature list unavailable");
    }
    let configuration: Option<Retained<AnyObject>> = msg_send![webview, configuration];
    let configuration = configuration.ok_or("no configuration")?;
    let preferences: Option<Retained<AnyObject>> = msg_send![&*configuration, preferences];
    let preferences = preferences.ok_or("no preferences")?;
    if !responds(&preferences, sel!(_setEnabled:forFeature:)) {
        return Err("feature toggle unavailable");
    }
    let features: Option<Retained<NSArray<AnyObject>>> = msg_send![class, _features];
    let features = features.ok_or("empty feature list")?;
    let wanted = NSString::from_str(NEAR_60_FPS);
    for feature in features.iter() {
        if !responds(&feature, sel!(key)) {
            continue;
        }
        let key: Option<Retained<NSString>> = msg_send![&*feature, key];
        if key.is_some_and(|key| key.isEqualToString(&wanted)) {
            let _: () =
                msg_send![&*preferences, _setEnabled: Bool::new(near_60), forFeature: &*feature];
            return Ok(());
        }
    }
    Err("feature not found in this WebKit")
}
