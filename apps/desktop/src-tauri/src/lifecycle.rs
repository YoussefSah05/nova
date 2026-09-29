//! macOS app lifecycle (M29): playback never outlives the page that owns it,
//! and sleep/wake is handled in Rust.
//!
//! Every playback resource is released by a frontend call (`mpv_close`,
//! `stop_playback`, `airplay_disengage`, `stop_caption_session`). When the
//! webview reloads or the app quits, no frontend is left to make it: native
//! video would keep playing behind a fresh page and the playback guards would
//! keep background work suspended.
//!
//! Each playback resource records the page epoch it was opened for
//! (`AppState::page_epoch`, bumped when the webview starts loading a page).
//! [`release_for_page`] releases what older pages opened (writing the native
//! session's progress first) and leaves the new page's; an open still in
//! flight when the page changed undoes itself (`commands::playback`).
//! [`release_for_exit`] runs when the app quits.
//!
//! Sleep: the native session pauses (which writes its progress). Wake: once
//! the network had a moment, a live channel that was playing reconnects at
//! the live edge through the player's recovery path (`playback:recover`,
//! `reason: wake`); anything else stays paused for the user.

use std::time::Duration;

use tauri::AppHandle;

use crate::state::AppState;

/// A power transition reported by the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    WillSleep,
    DidWake,
}

/// How long a release waits for the native session's final progress write.
const FINISH_TIMEOUT: Duration = Duration::from_secs(2);

/// Write the native session's progress, bounded: releasing must never hang.
async fn finish_native(state: &AppState, token: Option<&str>) {
    if tokio::time::timeout(FINISH_TIMEOUT, state.playback.finish(token))
        .await
        .is_err()
    {
        tracing::warn!("playback progress not written before release");
    }
}

/// A page (the page load of `epoch`) replaced the one that opened playback:
/// release every playback resource opened for an older page — the native
/// session after its final progress write, AirPlay, proxy sessions and their
/// remuxes, caption transcription — and its guard. Anything the new page
/// already opened stays. `app` is `None` only in tests (no AirPlay route).
pub async fn release_for_page(state: &AppState, app: Option<&AppHandle>, epoch: u64) {
    let mut released = false;

    let stale_native = state
        .mpv_activity
        .lock()
        .await
        .as_ref()
        .filter(|owned| owned.epoch < epoch)
        .map(|owned| owned.token.clone());
    if let Some(token) = stale_native {
        finish_native(state, Some(&token)).await;
        // A mismatch means a newer session replaced it: nothing to close.
        let _ = state.mpv_engine.close(&token);
        let mut activity = state.mpv_activity.lock().await;
        if activity.as_ref().is_some_and(|owned| owned.token == token) {
            activity.take();
        }
        released = true;
    }

    let stale_streams: Vec<String> = {
        let mut streams = state.stream_activity.lock().await;
        let stale: Vec<String> = streams
            .iter()
            .filter(|(_, owned)| owned.epoch < epoch)
            .map(|(token, _)| token.clone())
            .collect();
        for token in &stale {
            streams.remove(token);
        }
        stale
    };
    for token in &stale_streams {
        state.hls_sessions.release(token).await;
    }
    released |= !stale_streams.is_empty();

    let stale_airplay = {
        let mut airplay = state.airplay_activity.lock().await;
        if airplay.as_ref().is_some_and(|owned| owned.epoch < epoch) {
            airplay.take()
        } else {
            None
        }
    };
    if let Some(owned) = stale_airplay {
        if let Some(app) = app {
            state.airplay.disengage(app);
        }
        state.hls_sessions.release(&owned.token).await;
        released = true;
    }

    // Captions only run for a player: stop them with the old page's player.
    if released {
        state.caption_sessions.stop().await;
    }
}

/// The app is quitting: write the native session's progress and stop what
/// would outlive the process's own teardown (ffmpeg remuxes, transcription),
/// dropping every guard. Runs on the main thread (`RunEvent::Exit`), so it
/// never waits on the native engine or AirPlay: the process exit ends them.
pub async fn release_for_exit(state: &AppState) {
    finish_native(state, None).await;
    state.mpv_activity.lock().await.take();
    state.airplay_activity.lock().await.take();
    state.stream_activity.lock().await.clear();
    state.hls_sessions.release_all().await;
    state.caption_sessions.stop().await;
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
    use tauri::Manager;

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
        // Delivered on the posting (main) thread. Queued straight onto the
        // playback coordinator (never blocks), so a sleep and the wake after
        // it are handled in order even if the runtime was frozen between.
        let block = RcBlock::new(move |_note: NonNull<NSNotification>| {
            tracing::info!(?event, "power transition");
            let playback = &app.state::<AppState>().playback;
            match event {
                PowerEvent::WillSleep => playback.notify_sleep(),
                PowerEvent::DidWake => playback.notify_wake(),
            }
        });
        // SAFETY: no sender filter and no queue (delivered synchronously on
        // the posting thread); the block only reads a Send handle.
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

    async fn state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let pool = nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
            .await
            .unwrap();
        let state = AppState::new(pool, 0, SessionRegistry::default());
        state.playback.start(std::sync::Arc::new(|_| {}));
        (dir, state)
    }

    async fn active(state: &AppState) -> usize {
        state
            .model_runtime
            .snapshot()
            .await
            .active_playback_sessions
    }

    #[tokio::test]
    async fn a_reload_releases_the_old_pages_playback_and_keeps_the_new_ones() {
        let (_dir, state) = state().await;
        let old = state.page_epoch();
        state.track_stream_activity("old-web".into(), old).await;
        *state.mpv_activity.lock().await = Some(state.own_playback("old-native".into(), old));
        *state.airplay_activity.lock().await = Some(state.own_playback("old-air".into(), old));
        assert_eq!(active(&state).await, 3);

        // The webview reloads; the new page opens a stream before the
        // release gets to run.
        let page = state.next_page();
        state.track_stream_activity("new-web".into(), page).await;
        release_for_page(&state, None, page).await;
        assert_eq!(
            active(&state).await,
            1,
            "only the new page's stream is held"
        );
        assert!(state.stream_activity.lock().await.contains_key("new-web"));
        assert!(state.mpv_activity.lock().await.is_none());
        assert!(state.airplay_activity.lock().await.is_none());

        // Idempotent (the first page load of a fresh app runs it too).
        release_for_page(&state, None, page).await;
        assert_eq!(active(&state).await, 1);
    }

    #[tokio::test]
    async fn quitting_releases_everything() {
        let (_dir, state) = state().await;
        let page = state.page_epoch();
        state.track_stream_activity("web".into(), page).await;
        *state.mpv_activity.lock().await = Some(state.own_playback("native".into(), page));
        *state.airplay_activity.lock().await = Some(state.own_playback("air".into(), page));
        release_for_exit(&state).await;
        assert_eq!(active(&state).await, 0, "background work resumes");
    }
}
