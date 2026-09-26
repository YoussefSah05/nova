use nova_hls_proxy::SessionRegistry;
use nova_mpv_player::MpvEngine;
use sqlx::SqlitePool;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, RwLock};

use crate::airplay::AirPlayEngine;
use crate::caption_session::CaptionSessionManager;
use crate::desktop_jobs::DesktopJobs;
use crate::model_runtime::ModelRuntimeManager;
use crate::resource_policy::PlaybackActivityGuard;
use crate::translation_session::TranslationSessionManager;

pub struct AppState {
    /// Pool for app.db (profiles, app_settings).
    pub pool: Arc<SqlitePool>,
    pub active_profile_id: Arc<RwLock<Option<String>>>,
    /// Per-profile SQLite pools, keyed by profile UUID.
    pub profile_pools: Arc<RwLock<HashMap<String, Arc<SqlitePool>>>>,
    /// One set of bounded persisted workers per opened profile.
    pub desktop_jobs: Mutex<HashMap<String, Arc<DesktopJobs>>>,
    /// Serializes profile DB opens: migrations and the canonical-catalog data
    /// step must run once per profile, never concurrently.
    pub profile_open: Mutex<()>,
    /// Port the local HLS proxy bound to.
    pub hls_proxy_port: u16,
    /// Shared session registry on the proxy. `resolve_stream` calls
    /// [`SessionRegistry::register`]; `stop_playback` calls `release`.
    pub hls_sessions: SessionRegistry,
    /// Native libmpv playback engine used for reliable IPTV playback.
    pub mpv_engine: Arc<MpvEngine>,
    /// AVFoundation engine used only while an AirPlay route is active. On macOS
    /// it wraps an `AVPlayer` + route picker; elsewhere it's an inert stub.
    pub airplay: Arc<AirPlayEngine>,
    /// Drives streaming caption transcription for the open player (one active
    /// session at a time).
    pub caption_sessions: Arc<CaptionSessionManager>,
    /// Drives streaming translation of the source captions (one active session
    /// at a time), parallel to `caption_sessions`.
    pub translation_sessions: Arc<TranslationSessionManager>,
    /// Owns lazily loaded AI backends and prevents unload during active use.
    pub model_runtime: Arc<ModelRuntimeManager>,
    /// Native mpv has one replaceable session; this guard tracks its lifetime.
    pub mpv_activity: Mutex<Option<PlaybackActivityGuard>>,
    /// Web/HLS and AirPlay sessions are released by their proxy token.
    pub stream_activity: Mutex<HashMap<String, PlaybackActivityGuard>>,
    /// AirPlay has one AVPlayer item, separate from local mpv and proxy tokens.
    pub airplay_activity: Mutex<Option<PlaybackActivityGuard>>,
}

impl AppState {
    pub fn new(pool: SqlitePool, hls_proxy_port: u16, hls_sessions: SessionRegistry) -> Self {
        let pool = Arc::new(pool);
        Self {
            pool: pool.clone(),
            active_profile_id: Arc::new(RwLock::new(None)),
            profile_pools: Arc::new(RwLock::new(HashMap::new())),
            desktop_jobs: Mutex::new(HashMap::new()),
            profile_open: Mutex::new(()),
            hls_proxy_port,
            hls_sessions,
            mpv_engine: MpvEngine::new(),
            airplay: AirPlayEngine::new(),
            caption_sessions: Arc::new(CaptionSessionManager::new()),
            translation_sessions: Arc::new(TranslationSessionManager::new()),
            model_runtime: Arc::new(ModelRuntimeManager::new(pool)),
            mpv_activity: Mutex::new(None),
            stream_activity: Mutex::new(HashMap::new()),
            airplay_activity: Mutex::new(None),
        }
    }

    /// Returns the pool for a profile, or `None` if it hasn't been opened yet.
    pub async fn profile_pool(&self, profile_id: &str) -> Option<Arc<SqlitePool>> {
        self.profile_pools.read().await.get(profile_id).cloned()
    }

    pub async fn insert_profile_pool(&self, profile_id: String, pool: SqlitePool) {
        self.profile_pools
            .write()
            .await
            .insert(profile_id, Arc::new(pool));
    }

    pub async fn jobs_for_profile(
        &self,
        profile_id: &str,
        pool: Arc<SqlitePool>,
        app: tauri::AppHandle,
    ) -> Arc<DesktopJobs> {
        let mut jobs = self.desktop_jobs.lock().await;
        if let Some(existing) = jobs.get(profile_id) {
            return Arc::clone(existing);
        }
        let worker = Arc::new(DesktopJobs::new(pool, app, Arc::clone(&self.model_runtime)));
        // First creation in this process: no worker for this profile has run
        // yet, so any `running` row was interrupted by a previous process.
        worker.recover_interrupted().await;
        worker.kick_all();
        jobs.insert(profile_id.to_string(), Arc::clone(&worker));
        worker
    }

    /// Existing workers only; never creates or recovers a profile's queues.
    pub async fn existing_jobs(&self, profile_id: &str) -> Option<Arc<DesktopJobs>> {
        self.desktop_jobs.lock().await.get(profile_id).cloned()
    }

    pub async fn track_stream_activity(&self, token: String) {
        self.stream_activity
            .lock()
            .await
            .insert(token, self.model_runtime.playback_activity());
    }

    pub async fn release_stream_activity(&self, token: &str) {
        self.stream_activity.lock().await.remove(token);
    }

    /// Load from the caller's profile on every search. Row counts are not a
    /// revision: refresh and re-enrichment can replace every vector at the same
    /// count, and source IDs are local to each profile database.
    pub async fn catalog_vectors<F, Fut>(
        &self,
        load: F,
    ) -> Result<Arc<Vec<(i64, Vec<f32>)>>, String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<(i64, Vec<f32>)>, String>>,
    {
        Ok(Arc::new(load().await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn catalog_vectors_do_not_reuse_another_profiles_or_replaced_rows() {
        let dir = tempfile::tempdir().unwrap();
        let pool = nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
            .await
            .unwrap();
        let state = AppState::new(pool, 0, SessionRegistry::default());
        let first = state
            .catalog_vectors(|| async { Ok(vec![(1, vec![1.0, 0.0])]) })
            .await
            .unwrap();
        let other_profile = state
            .catalog_vectors(|| async { Ok(vec![(1, vec![0.0, 1.0])]) })
            .await
            .unwrap();
        assert_ne!(*first, *other_profile);
        let refreshed = state
            .catalog_vectors(|| async { Ok(vec![(2, vec![0.0, 1.0])]) })
            .await
            .unwrap();
        assert_eq!(refreshed[0].0, 2);
    }

    #[tokio::test]
    async fn stream_tokens_hold_playback_priority_until_released() {
        let dir = tempfile::tempdir().unwrap();
        let pool = nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
            .await
            .unwrap();
        let state = AppState::new(pool, 0, SessionRegistry::default());

        state.track_stream_activity("one".into()).await;
        state.track_stream_activity("two".into()).await;
        assert_eq!(
            state
                .model_runtime
                .snapshot()
                .await
                .active_playback_sessions,
            2
        );

        state.release_stream_activity("one").await;
        assert_eq!(
            state
                .model_runtime
                .snapshot()
                .await
                .active_playback_sessions,
            1
        );
        state.release_stream_activity("two").await;
        assert_eq!(
            state
                .model_runtime
                .snapshot()
                .await
                .active_playback_sessions,
            0
        );
    }
}
