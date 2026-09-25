pub mod airplay;
pub mod caption_session;
pub mod commands;
pub mod ipc;
pub mod model_registry;
#[cfg(target_os = "macos")]
pub mod mpv_layer;
#[cfg(target_os = "macos")]
pub mod native_player_view;
pub mod state;
pub mod translation_session;

use std::sync::Arc;

use nova_mpv_player::{EventEmitter, MpvEngine};
use nova_storage::init_app_db;
use tauri::{AppHandle, Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    let (pool, proxy_handle) = tauri::async_runtime::block_on(async {
        let data_dir = validation_data_dir().unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("nova")
        });
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
        // Remember the window's size / position / maximized state across launches.
        .plugin(tauri_plugin_window_state::Builder::default().build())
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

                // Parallel AVFoundation engine + AirPlay route picker overlay.
                let airplay = app.state::<state::AppState>().airplay.clone();
                airplay::install_picker(&window, &airplay, app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Phase 1
            commands::list_profiles,
            commands::create_profile,
            commands::delete_profile,
            commands::select_profile,
            commands::get_profile_overview,
            // Phase 2
            commands::list_sources,
            commands::add_source,
            commands::refresh_source,
            commands::list_channels,
            commands::count_channels,
            commands::list_channel_countries,
            commands::list_channel_categories,
            commands::list_channel_regions,
            commands::list_channel_groups,
            commands::list_channels_in_group,
            commands::list_vod,
            commands::count_vod,
            commands::list_vod_categories,
            commands::list_series,
            commands::count_series,
            commands::list_series_categories,
            commands::list_series_episodes,
            commands::list_player_episodes,
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
            commands::playback::set_player_surface_active,
            commands::playback::record_watch_progress,
            commands::playback::get_resume_position,
            // Phase 6 — Transcription (VOD), streaming caption sessions
            commands::transcription::get_model_status,
            commands::models::list_internal_models,
            commands::transcription::download_whisper_model,
            commands::transcription::start_caption_session,
            commands::transcription::set_caption_playhead,
            commands::transcription::stop_caption_session,
            // Phase 7 — Translation (VOD), streaming translation sessions
            commands::translation::get_nllb_model_status,
            commands::translation::download_nllb_model,
            commands::translation::list_target_languages,
            commands::translation::start_translation_session,
            commands::translation::set_translation_playhead,
            commands::translation::stop_translation_session,
            // Phase 8 — Search (transcript companion panel + dialogue FTS)
            commands::search::get_transcript,
            commands::search::search_transcript,
            // Semantic / hybrid search (catalog discovery)
            commands::search::semantic_search_catalog,
            commands::search::semantic_search_channels,
            commands::search::semantic_search_series,
            // Phase 6.2 — per-profile caption styling
            commands::caption_style::get_caption_style,
            commands::caption_style::set_caption_style,
            // Catalog enrichment (Phase B) — embeddings de-dup + genre fill
            commands::enrich::get_enrich_model_status,
            commands::enrich::download_enrich_models,
            commands::enrich::start_enrichment,
            // Cleanup — per-profile viewing settings + player episode list
            commands::profile_settings::get_profile_settings,
            commands::profile_settings::set_profile_settings,
            // Cleanup — Settings sections
            commands::settings::update_profile,
            commands::settings::delete_source,
            commands::settings::set_source_auto_refresh,
            commands::settings::remove_whisper_model,
            commands::settings::get_transcription_cache_stats,
            commands::settings::clear_transcription_cache,
            commands::settings::get_app_info,
            // Phase 5 — AirPlay (parallel AVPlayer route)
            commands::playback::airplay_set_picker_rect,
            commands::playback::airplay_engage,
            commands::playback::airplay_disengage,
            commands::playback::airplay_toggle_pause,
            commands::playback::airplay_seek_absolute,
            commands::playback::airplay_set_volume,
            commands::playback::airplay_set_mute,
            commands::playback::airplay_status,
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

/// Debug-only isolated profile/database root for repeatable native validation.
/// Release builds always use the normal application directories.
pub(crate) fn validation_data_dir() -> Option<std::path::PathBuf> {
    #[cfg(debug_assertions)]
    {
        std::env::var_os("NOVA_VALIDATION_DIR")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}
