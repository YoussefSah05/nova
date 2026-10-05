use nova_hls_proxy::SessionRegistry;
use nova_mpv_player::MpvEngine;
use sqlx::SqlitePool;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock};

use crate::airplay::AirPlayEngine;
use crate::caption_session::CaptionSessionManager;
use crate::desktop_jobs::DesktopJobs;
use crate::model_runtime::ModelRuntimeManager;
use crate::playback_coordinator::PlaybackCoordinator;
use crate::resource_policy::PlaybackActivityGuard;

/// A playback resource held for the page that opened it: its session token,
/// the page epoch it was opened in, and the guard that keeps background work
/// suspended while it plays.
pub struct OwnedPlayback {
    pub token: String,
    pub epoch: u64,
    _guard: PlaybackActivityGuard,
}

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
    pub profile_open: Arc<Mutex<()>>,
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
    /// Owns lazily loaded AI backends and prevents unload during active use.
    pub model_runtime: Arc<ModelRuntimeManager>,
    /// Playback policy of the native session: resume, progress, completion,
    /// next episode (Rust-owned; React renders).
    pub playback: Arc<PlaybackCoordinator>,
    /// Native mpv has one replaceable session; this tracks its lifetime.
    pub mpv_activity: Mutex<Option<OwnedPlayback>>,
    /// Web/HLS sessions, by proxy token.
    pub stream_activity: Mutex<HashMap<String, OwnedPlayback>>,
    /// AirPlay has one AVPlayer item (token: its proxy session).
    pub airplay_activity: Mutex<Option<OwnedPlayback>>,
    /// Bumped whenever the webview starts loading a page: playback opened for
    /// an older page has no owner left and is released (see `lifecycle`).
    page_epoch: AtomicU64,
}

impl AppState {
    pub fn new(pool: SqlitePool, hls_proxy_port: u16, hls_sessions: SessionRegistry) -> Self {
        let pool = Arc::new(pool);
        let mpv_engine = MpvEngine::new();
        Self {
            pool: pool.clone(),
            active_profile_id: Arc::new(RwLock::new(None)),
            profile_pools: Arc::new(RwLock::new(HashMap::new())),
            desktop_jobs: Mutex::new(HashMap::new()),
            profile_open: Arc::new(Mutex::new(())),
            hls_proxy_port,
            hls_sessions,
            playback: PlaybackCoordinator::new(mpv_engine.clone()),
            mpv_engine,
            airplay: AirPlayEngine::new(),
            caption_sessions: Arc::new(CaptionSessionManager::new()),
            model_runtime: Arc::new(ModelRuntimeManager::new(pool)),
            mpv_activity: Mutex::new(None),
            stream_activity: Mutex::new(HashMap::new()),
            airplay_activity: Mutex::new(None),
            page_epoch: AtomicU64::new(0),
        }
    }

    /// The current page's epoch; capture it before a playback open awaits.
    pub fn page_epoch(&self) -> u64 {
        self.page_epoch.load(Ordering::SeqCst)
    }

    /// A new page started loading; returns its epoch.
    pub fn next_page(&self) -> u64 {
        self.page_epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Hold a playback resource for the page of `epoch`.
    pub fn own_playback(&self, token: String, epoch: u64) -> OwnedPlayback {
        OwnedPlayback {
            token,
            epoch,
            _guard: self.model_runtime.playback_activity(),
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
        let worker = Arc::new(DesktopJobs::new(
            profile_id,
            pool,
            app,
            Arc::clone(&self.model_runtime),
        ));
        // First creation in this process: no worker for this profile has run
        // yet, so any `running` row was interrupted by a previous process.
        worker.recover_interrupted().await;
        worker.kick_all();
        // Automatic playlist refreshes, only while this is the active profile.
        let scheduler = Arc::downgrade(&worker);
        let active = Arc::clone(&self.active_profile_id);
        let owner = profile_id.to_string();
        tauri::async_runtime::spawn(async move {
            loop {
                let Some(worker) = scheduler.upgrade() else {
                    return;
                };
                if active.read().await.as_deref() == Some(owner.as_str()) {
                    worker.schedule_due_refreshes().await;
                }
                drop(worker);
                tokio::time::sleep(crate::desktop_jobs::SCHEDULE_INTERVAL).await;
            }
        });
        jobs.insert(profile_id.to_string(), Arc::clone(&worker));
        worker
    }

    /// Existing workers only; never creates or recovers a profile's queues.
    pub async fn existing_jobs(&self, profile_id: &str) -> Option<Arc<DesktopJobs>> {
        self.desktop_jobs.lock().await.get(profile_id).cloned()
    }

    pub async fn track_stream_activity(&self, token: String, epoch: u64) {
        let owned = self.own_playback(token.clone(), epoch);
        self.stream_activity.lock().await.insert(token, owned);
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

        state.track_stream_activity("one".into(), 0).await;
        state.track_stream_activity("two".into(), 0).await;
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
