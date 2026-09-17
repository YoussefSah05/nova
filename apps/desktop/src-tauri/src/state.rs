use nova_enrich::{Embedder, FastEmbedder};
use nova_hls_proxy::SessionRegistry;
use nova_mpv_player::MpvEngine;
use sqlx::SqlitePool;
use std::path::Path;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, RwLock};

use crate::airplay::AirPlayEngine;
use crate::caption_session::CaptionSessionManager;
use crate::translation_session::TranslationSessionManager;

/// An in-memory catalog vector index, validated against the DB row count so it
/// reloads only after a (re-)enrichment changes the vectors.
pub struct CachedIndex {
    pub count: i64,
    pub vectors: Arc<Vec<(i64, Vec<f32>)>>,
}

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
    /// On-device embedder for semantic search query/segment embedding, loaded
    /// once on first use (cold ONNX init is slow). `None` until then.
    semantic_embedder: Mutex<Option<Arc<dyn Embedder>>>,
    /// Per-`source:kind` catalog vector index, cached across searches and
    /// reloaded when the persisted vector count changes.
    catalog_index: Mutex<HashMap<String, CachedIndex>>,
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
            airplay: AirPlayEngine::new(),
            caption_sessions: Arc::new(CaptionSessionManager::new()),
            translation_sessions: Arc::new(TranslationSessionManager::new()),
            semantic_embedder: Mutex::new(None),
            catalog_index: Mutex::new(HashMap::new()),
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

    /// The shared embedder, loading it (downloading nothing — the model must
    /// already be installed) on first use. The cold ONNX init runs on a blocking
    /// thread; the async lock serializes concurrent first-loads so we load once.
    pub async fn semantic_embedder(&self, model_dir: &Path) -> Result<Arc<dyn Embedder>, String> {
        let mut guard = self.semantic_embedder.lock().await;
        if let Some(e) = guard.as_ref() {
            return Ok(e.clone());
        }
        let dir = model_dir.to_path_buf();
        let embedder = tokio::task::spawn_blocking(move || FastEmbedder::load(&dir, false))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let arc: Arc<dyn Embedder> = Arc::new(embedder);
        *guard = Some(arc.clone());
        Ok(arc)
    }

    /// Return the cached catalog vector index for `source:kind`, reloading via
    /// `load` only when the live `count` differs from the cached one (so a
    /// re-enrichment is picked up without any explicit invalidation wiring).
    pub async fn catalog_vectors<F, Fut>(
        &self,
        source_id: i64,
        kind: &str,
        count: i64,
        load: F,
    ) -> Result<Arc<Vec<(i64, Vec<f32>)>>, String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<(i64, Vec<f32>)>, String>>,
    {
        let key = format!("{source_id}:{kind}");
        {
            let guard = self.catalog_index.lock().await;
            if let Some(idx) = guard.get(&key) {
                if idx.count == count {
                    return Ok(idx.vectors.clone());
                }
            }
        }
        let vectors = Arc::new(load().await?);
        let mut guard = self.catalog_index.lock().await;
        guard.insert(
            key,
            CachedIndex {
                count,
                vectors: vectors.clone(),
            },
        );
        Ok(vectors)
    }
}
