use nova_hls_proxy::SessionRegistry;
use nova_mpv_player::MpvEngine;
use sqlx::SqlitePool;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub struct AppState {
    /// Pool for app.db (profiles, app_settings).
    pub pool: Arc<SqlitePool>,
    pub active_profile_id: Arc<RwLock<Option<String>>>,
    /// Per-profile SQLite pools, keyed by profile UUID.
    pub profile_pools: Arc<RwLock<HashMap<String, Arc<SqlitePool>>>>,
    /// Port the local HLS proxy bound to.
    pub hls_proxy_port: u16,
    /// Shared session registry on the proxy. `resolve_stream` calls
    /// [`SessionRegistry::register`]; `stop_playback` calls `release`.
    pub hls_sessions: SessionRegistry,
    /// Native libmpv playback engine used for reliable IPTV playback.
    pub mpv_engine: MpvEngine,
}

impl AppState {
    pub fn new(pool: SqlitePool, hls_proxy_port: u16, hls_sessions: SessionRegistry) -> Self {
        Self {
            pool: Arc::new(pool),
            active_profile_id: Arc::new(RwLock::new(None)),
            profile_pools: Arc::new(RwLock::new(HashMap::new())),
            hls_proxy_port,
            hls_sessions,
            mpv_engine: MpvEngine::new(),
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
}
