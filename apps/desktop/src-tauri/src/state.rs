use sqlx::SqlitePool;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub struct AppState {
    /// Pool for app.db (profiles, app_settings).
    pub pool: Arc<SqlitePool>,
    pub active_profile_id: Arc<RwLock<Option<String>>>,
    /// Per-profile SQLite pools, keyed by profile UUID.
    pub profile_pools: Arc<RwLock<HashMap<String, Arc<SqlitePool>>>>,
}

impl AppState {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool: Arc::new(pool),
            active_profile_id: Arc::new(RwLock::new(None)),
            profile_pools: Arc::new(RwLock::new(HashMap::new())),
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
