pub mod commands;
pub mod ipc;
#[cfg(target_os = "macos")]
pub mod mpv_layer;
#[cfg(target_os = "macos")]
pub mod native_player_view;
pub mod state;

use std::sync::Arc;

use nova_mpv_player::{EventEmitter, MpvEngine};
use nova_storage::init_app_db;
use tauri::{AppHandle, Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    let (pool, proxy_handle) = tauri::async_runtime::block_on(async {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("nova");
        std::fs::create_dir_all(&data_dir).expect("failed to create Nova data dir");
        let db_path = data_dir.join("app.db");
        let pool = init_app_db(db_path.to_str().expect("non-utf8 path"))
            .await
            .expect("failed to initialise Nova database");

        // Local HLS reverse proxy. Phase 5 ships a passthrough; later phases
        // rewrite manifests through this port to inject captions.
        let proxy = nova_hls_proxy::start()
            .await
            .expect("failed to start HLS proxy");
        (pool, proxy)
    });

    let proxy_port = proxy_handle.port;
    let proxy_sessions = proxy_handle.sessions().clone();
    // Keep the proxy alive for the lifetime of the app. We deliberately leak
    // the handle — Tauri's process is the proxy's lifecycle owner; cleanup
    // happens when the process exits.
    std::mem::forget(proxy_handle);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state::AppState::new(pool, proxy_port, proxy_sessions))
        .setup(|app| {
            let engine = app.state::<state::AppState>().mpv_engine.clone();

            // Wire the wakeup callback → "mpv:state" Tauri event. The
            // frontend `PlayerModal` listens for this instead of polling.
            let emitter = AppEventEmitter::new(app.handle().clone(), engine.clone());
            if let Err(e) = engine.set_event_emitter(Arc::new(emitter)) {
                tracing::warn!(error = %e, "failed to register mpv event emitter");
            }

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                native_player_view::install_mpv_render_view(&window, engine)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Phase 1
            commands::list_profiles,
            commands::create_profile,
            commands::delete_profile,
            commands::select_profile,
            // Phase 2
            commands::list_sources,
            commands::add_source,
            commands::list_channels,
            commands::count_channels,
            commands::list_channel_countries,
            commands::list_channel_categories,
            commands::list_vod,
            commands::count_vod,
            commands::list_vod_categories,
            commands::list_series,
            commands::count_series,
            commands::list_series_categories,
            commands::list_series_episodes,
            // Phase 3
            commands::set_channel_favorite,
            commands::set_vod_favorite,
            commands::set_series_favorite,
            commands::set_channel_in_watchlist,
            commands::set_vod_in_watchlist,
            commands::set_series_in_watchlist,
            commands::list_continue_watching,
            // Phase 4 — EPG
            commands::epg::epg_now_and_next,
            commands::epg::epg_range,
            commands::epg::add_epg_url,
            commands::epg::epg_refresh,
            // Phase 5 — Playback
            commands::playback::get_proxy_port,
            commands::playback::resolve_stream,
            commands::playback::stop_playback,
            commands::playback::mpv_open_stream,
            commands::playback::mpv_close,
            commands::playback::mpv_player_state,
            commands::playback::mpv_toggle_pause,
            commands::playback::mpv_seek_relative,
            commands::playback::mpv_seek_absolute,
            commands::playback::mpv_seek_live_edge,
            commands::playback::mpv_set_volume,
            commands::playback::mpv_set_mute,
            commands::playback::mpv_set_audio_track,
            commands::playback::mpv_set_subtitle_track,
            commands::playback::mpv_set_video_track,
            commands::playback::mpv_set_speed,
            commands::playback::mpv_frame_step,
            commands::playback::mpv_request_redraw,
            commands::playback::mpv_set_video_rect,
            commands::playback::record_watch_progress,
            commands::playback::get_resume_position,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Nova");
}

/// libmpv wakeup callback adapter. Fires on libmpv's internal thread, so we
/// schedule the actual state snapshot + event emission onto the Tauri async
/// runtime to avoid blocking the wakeup thread.
struct AppEventEmitter {
    app: AppHandle,
    engine: Arc<MpvEngine>,
}

impl AppEventEmitter {
    fn new(app: AppHandle, engine: Arc<MpvEngine>) -> Self {
        Self { app, engine }
    }
}

impl EventEmitter for AppEventEmitter {
    fn wake(&self) {
        let app = self.app.clone();
        let engine = self.engine.clone();
        // The wakeup callback must return immediately. Snapshot + emit on a
        // background tokio task. We accept that a flurry of wakeups produces
        // a flurry of events — Tauri's IPC channel coalesces sensibly and
        // the frontend's React state setters are idempotent.
        tauri::async_runtime::spawn(async move {
            match engine.state(None) {
                Ok(state) => {
                    if let Err(e) = app.emit("mpv:state", state) {
                        tracing::debug!(error = %e, "failed to emit mpv:state");
                    }
                }
                Err(e) => {
                    tracing::debug!(error = %e, "failed to snapshot mpv state");
                }
            }
        });
    }
}
