//! macOS app lifecycle (M29): playback never outlives the page that owns it,
//! and sleep/wake is handled in Rust.
//!
//! Every playback resource is released by a frontend call (`mpv_close`,
//! `stop_playback`, `airplay_disengage`, `stop_caption_session`). When the
//! webview reloads or the app quits, no frontend is left to make it: native
//! video would keep playing behind a fresh page and the playback guards would
//! keep background work suspended. [`release_all_playback`] releases all of
//! it (writing the native session's progress first) when the webview starts
//! loading a page and when the app exits.
//!
//! Sleep: the native session pauses (which writes its progress). Wake: a live
//! channel that was playing reconnects at the live edge through the player's
//! recovery path (`playback:recover`, `reason: wake`); anything else stays
//! paused for the user.

use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// How long a release waits for the native session's final progress write.
const FINISH_TIMEOUT: Duration = Duration::from_secs(2);

/// A power transition reported by the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    WillSleep,
    DidWake,
}

/// Release every playback resource and guard: the native session (after its
/// final progress write), AirPlay, proxy sessions and remux processes,
/// caption transcription. `app` is `None` only in tests (no AirPlay route).
pub async fn release_all_playback(state: &AppState, app: Option<&AppHandle>) {
    // Bounded: quitting must never hang on a progress write.
    if tokio::time::timeout(FINISH_TIMEOUT, state.playback.finish(None))
        .await
        .is_err()
    {
        tracing::warn!("playback progress not written before release");
    }
    if let Err(e) = state.mpv_engine.close_active() {
        tracing::warn!(error = %e, "could not close the native player");
    }
    state.mpv_activity.lock().await.take();
    if let Some(app) = app {
        state.airplay.disengage(app);
    }
    state.airplay_activity.lock().await.take();
    state.hls_sessions.release_all().await;
    state.stream_activity.lock().await.clear();
    state.caption_sessions.stop().await;
}

/// React to a power transition.
pub async fn on_power(app: &AppHandle, event: PowerEvent) {
    let state = app.state::<AppState>();
    tracing::info!(?event, "power transition");
    match event {
        PowerEvent::WillSleep => state.playback.sleep().await,
        PowerEvent::DidWake => state.playback.wake().await,
    }
}

/// Observe the workspace's sleep and wake notifications for the app's
/// lifetime. Call once, from the main thread (setup).
#[cfg(target_os = "macos")]
pub fn observe_power(app: &AppHandle) {
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::NSNotification;

    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: AppKit's notification-name constants are immutable statics.
    let names = unsafe {
        [
            (NSWorkspaceWillSleepNotification, PowerEvent::WillSleep),
            (NSWorkspaceDidWakeNotification, PowerEvent::DidWake),
        ]
    };
    for (name, event) in names {
        let app = app.clone();
        // Delivered on the posting (main) thread: hand off at once.
        let block = RcBlock::new(move |_note: NonNull<NSNotification>| {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { on_power(&app, event).await });
        });
        // SAFETY: no sender filter and no queue (delivered synchronously on
        // the posting thread); the block only clones a Send handle and spawns.
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        // Observed for the whole process: the center holds the block, and
        // the returned token is never removed.
        std::mem::forget(observer);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn observe_power(_app: &AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_hls_proxy::SessionRegistry;

    #[tokio::test]
    async fn a_reload_releases_every_playback_guard() {
        let dir = tempfile::tempdir().unwrap();
        let pool = nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
            .await
            .unwrap();
        let state = AppState::new(pool, 0, SessionRegistry::default());
        state.playback.start(std::sync::Arc::new(|_| {}));
        state.track_stream_activity("web".into()).await;
        state.track_stream_activity("airplay-proxy".into()).await;
        *state.mpv_activity.lock().await = Some(state.model_runtime.playback_activity());
        *state.airplay_activity.lock().await = Some(state.model_runtime.playback_activity());
        let active = || async {
            state
                .model_runtime
                .snapshot()
                .await
                .active_playback_sessions
        };
        assert_eq!(active().await, 4);

        release_all_playback(&state, None).await;
        assert_eq!(active().await, 0, "background work resumes");
        // Idempotent: the first page load of a fresh app runs it too.
        release_all_playback(&state, None).await;
        assert_eq!(active().await, 0);
    }
}
