//! Lazy, lease-owned execution of registered local models. Registry observations
//! identify a model; the runtime owns loaded backends and their filesystem grants.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use nova_core::models::{ModelAvailability, ModelId, ModelLibraryId};
use nova_enrich::{Embedder, FastEmbedder};
use nova_storage::{ModelRepository, RegisteredModel};
use nova_transcribe::{default_model, model_paths};
use nova_translate::NllbTranslator;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, Notify, Semaphore};

use crate::model_registry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    InvalidModel,
    Unavailable,
    LibraryOffline,
    Busy,
    Cancelled,
    LoadFailed,
    RegistryFailed,
    WorkerFailed,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidModel => "unknown model",
            Self::Unavailable => "model is unavailable",
            Self::LibraryOffline => "model library is offline",
            Self::Busy => "model is loading, unloading or in use",
            Self::Cancelled => "model request cancelled",
            Self::LoadFailed => "model backend could not load",
            Self::RegistryFailed => "model registry could not be read",
            Self::WorkerFailed => "model worker failed",
        };
        f.write_str(message)
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Clone, Default)]
pub struct RuntimeCancellation {
    inner: Arc<CancellationInner>,
}

#[derive(Default)]
struct CancellationInner {
    cancelled: AtomicBool,
    changed: Notify,
}

impl RuntimeCancellation {
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.changed.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.inner.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

enum Backend {
    Embedding(Box<dyn Embedder>),
    Translation(NllbTranslator),
    Whisper(PathBuf),
}

trait BackendAdapter: Send + Sync {
    fn load(&self, path: &Path) -> Result<Backend, RuntimeError>;
    /// A resource-policy hook, not an admission decision. None means unknown.
    fn estimated_resident_bytes(&self) -> Option<u64> {
        None
    }
}

struct EmbeddingAdapter;
impl BackendAdapter for EmbeddingAdapter {
    fn load(&self, path: &Path) -> Result<Backend, RuntimeError> {
        FastEmbedder::load_local(path)
            .map(|model| Backend::Embedding(Box::new(model)))
            .map_err(|_| RuntimeError::LoadFailed)
    }
}

struct TranslationAdapter;
impl BackendAdapter for TranslationAdapter {
    fn load(&self, path: &Path) -> Result<Backend, RuntimeError> {
        NllbTranslator::load(path)
            .map(Backend::Translation)
            .map_err(|_| RuntimeError::LoadFailed)
    }
}

struct WhisperAdapter;
impl BackendAdapter for WhisperAdapter {
    fn load(&self, path: &Path) -> Result<Backend, RuntimeError> {
        let model = default_model();
        let paths = model_paths(path, &model);
        if !paths.ggml.is_file() {
            return Err(RuntimeError::Unavailable);
        }
        Ok(Backend::Whisper(paths.ggml))
    }
}

struct Loaded {
    backend: Backend,
    library_id: ModelLibraryId,
    path: PathBuf,
    estimated_bytes: Option<u64>,
    bookmark_data: Option<Vec<u8>>,
    #[cfg(target_os = "macos")]
    _external_grant: Option<crate::mac_model_library::ScopedModelFolder>,
}

#[derive(Default)]
struct Slot {
    loaded: Option<Arc<Loaded>>,
}

struct Entry {
    state: Arc<Mutex<Slot>>,
    leases: AtomicUsize,
    operations: Arc<Semaphore>,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(Slot::default())),
            leases: AtomicUsize::new(0),
            operations: Arc::new(Semaphore::new(1)),
        }
    }
}

/// The lease is the only way consumers access a backend. Dropping it releases
/// one use, including on cancellation, task failure or panic unwind.
pub struct ModelLease {
    entry: Arc<Entry>,
    loaded: Arc<Loaded>,
}

impl Clone for ModelLease {
    fn clone(&self) -> Self {
        // Existing leases guarantee `loaded` cannot be evicted while this
        // increment happens, even though the async entry mutex is not held.
        self.entry.leases.fetch_add(1, Ordering::AcqRel);
        Self {
            entry: Arc::clone(&self.entry),
            loaded: Arc::clone(&self.loaded),
        }
    }
}

impl Drop for ModelLease {
    fn drop(&mut self) {
        // Drop is synchronous and can run on an async worker. Use a separate
        // atomic lease counter instead of blocking on Tokio's mutex here.
        self.entry.release();
    }
}

impl Entry {
    fn release(&self) {
        self.leases.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ModelLease {
    /// Yield admission between document batches so a catalog sweep cannot hold
    /// the shared embedder ahead of an interactive query for the whole import.
    pub async fn embed_documents(
        &self,
        texts: &[String],
        cancellation: &RuntimeCancellation,
        on_progress: impl Fn(usize),
    ) -> Result<Vec<Vec<f32>>, RuntimeError> {
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(256) {
            let count = batch.len();
            let batch = batch.to_vec();
            let result = self
                .clone()
                .run_blocking(cancellation, move |lease| {
                    lease
                        .embedder()
                        .ok_or(RuntimeError::InvalidModel)?
                        .embed(&batch)
                        .map_err(|_| RuntimeError::WorkerFailed)
                })
                .await??;
            if result.len() != count {
                return Err(RuntimeError::WorkerFailed);
            }
            vectors.extend(result);
            on_progress(vectors.len());
        }
        Ok(vectors)
    }

    /// Native calls are admitted one at a time per model. The worker owns both
    /// lease and permit even if its caller is cancelled or its task is aborted.
    /// Running native calls finish cooperatively; their abandoned result is
    /// discarded, and the model cannot be evicted while they finish.
    pub async fn run_blocking<T: Send + 'static>(
        self,
        cancellation: &RuntimeCancellation,
        work: impl FnOnce(&ModelLease) -> T + Send + 'static,
    ) -> Result<T, RuntimeError> {
        let permit = tokio::select! {
            _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
            permit = self.entry.operations.clone().acquire_owned() => permit.map_err(|_| RuntimeError::WorkerFailed)?,
        };
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work(&self)
        });
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(RuntimeError::Cancelled),
            result = worker => result.map_err(|_| RuntimeError::WorkerFailed),
        }
    }

    /// Subprocess adapters also retain their lease until the child-owning
    /// future finishes. Cancelling the caller cannot detach unleased work.
    pub async fn run_async<T, F>(
        self,
        cancellation: &RuntimeCancellation,
        work: impl FnOnce(ModelLease) -> F + Send + 'static,
    ) -> Result<T, RuntimeError>
    where
        T: Send + 'static,
        F: std::future::Future<Output = T> + Send + 'static,
    {
        let permit = tokio::select! {
            _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
            permit = self.entry.operations.clone().acquire_owned() => permit.map_err(|_| RuntimeError::WorkerFailed)?,
        };
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let worker = tokio::spawn(async move {
            let _permit = permit;
            work(self).await
        });
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(RuntimeError::Cancelled),
            result = worker => result.map_err(|_| RuntimeError::WorkerFailed),
        }
    }

    pub fn embedder(&self) -> Option<&dyn Embedder> {
        match &self.loaded.backend {
            Backend::Embedding(model) => Some(model.as_ref()),
            _ => None,
        }
    }

    pub fn translator(&self) -> Option<&NllbTranslator> {
        match &self.loaded.backend {
            Backend::Translation(model) => Some(model),
            _ => None,
        }
    }

    pub fn whisper_path(&self) -> Option<&Path> {
        match &self.loaded.backend {
            Backend::Whisper(path) => Some(path),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub loaded_models: usize,
    pub active_leases: usize,
    pub estimated_resident_bytes: u64,
    pub unknown_resident_models: usize,
    pub busy_models: usize,
    pub transitioning_models: usize,
}

struct Resolved {
    model: RegisteredModel,
    app_data_dir: PathBuf,
    external_bookmark: Option<Vec<u8>>,
}

pub struct ModelRuntimeManager {
    registry_pool: Arc<SqlitePool>,
    entries: Mutex<HashMap<ModelId, Arc<Entry>>>,
    adapters: HashMap<ModelId, Arc<dyn BackendAdapter>>,
}

impl ModelRuntimeManager {
    pub fn new(registry_pool: Arc<SqlitePool>) -> Self {
        let mut adapters: HashMap<ModelId, Arc<dyn BackendAdapter>> = HashMap::new();
        for (id, adapter) in [
            (
                default_model().id,
                Arc::new(WhisperAdapter) as Arc<dyn BackendAdapter>,
            ),
            (
                nova_translate::model::MODEL_ID,
                Arc::new(TranslationAdapter),
            ),
            (nova_enrich::MODEL_ID, Arc::new(EmbeddingAdapter)),
        ] {
            if let Ok(id) = ModelId::new(id) {
                adapters.insert(id, adapter);
            }
        }
        Self {
            registry_pool,
            entries: Mutex::new(HashMap::new()),
            adapters,
        }
    }
}

impl ModelRuntimeManager {
    /// Live availability observation only: no backend initialization/download.
    pub async fn is_available(&self, root: &Path, model_id: &str) -> Result<bool, RuntimeError> {
        let id = ModelId::new(model_id).map_err(|_| RuntimeError::InvalidModel)?;
        if !self.adapters.contains_key(&id) {
            return Err(RuntimeError::InvalidModel);
        }
        match self.resolve(root, &id).await {
            Ok(_) => Ok(true),
            Err(RuntimeError::Unavailable | RuntimeError::LibraryOffline) => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub async fn acquire(
        &self,
        app_data_dir: &Path,
        model_id: &str,
        cancellation: &RuntimeCancellation,
    ) -> Result<ModelLease, RuntimeError> {
        let id = ModelId::new(model_id).map_err(|_| RuntimeError::InvalidModel)?;
        if !self.adapters.contains_key(&id) {
            return Err(RuntimeError::InvalidModel);
        }
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let resolved = tokio::select! {
            _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
            resolved = self.resolve(app_data_dir, &id) => resolved?,
        };
        self.acquire_resolved(id, resolved, cancellation).await
    }

    async fn resolve(&self, app_data_dir: &Path, id: &ModelId) -> Result<Resolved, RuntimeError> {
        let pool = &*self.registry_pool;
        model_registry::refresh_internal(app_data_dir.to_path_buf(), pool)
            .await
            .map_err(|_| RuntimeError::RegistryFailed)?;
        let repo = ModelRepository::new(pool);
        let internal = repo
            .list_internal()
            .await
            .map_err(|_| RuntimeError::RegistryFailed)?;
        if let Some(model) = internal
            .into_iter()
            .find(|m| m.descriptor.id == *id && usable(m.availability))
        {
            return Ok(Resolved {
                model,
                app_data_dir: app_data_dir.to_path_buf(),
                external_bookmark: None,
            });
        }
        model_registry::refresh_external(pool)
            .await
            .map_err(|_| RuntimeError::RegistryFailed)?;
        let libraries = repo
            .list_external_libraries()
            .await
            .map_err(|_| RuntimeError::RegistryFailed)?;
        let external = repo
            .list_external_models()
            .await
            .map_err(|_| RuntimeError::RegistryFailed)?;
        let mut offline = false;
        for candidate in external {
            offline |= candidate.model.descriptor.id == *id
                && candidate.model.availability == ModelAvailability::Unavailable;
            if candidate.model.descriptor.id != *id || !usable(candidate.model.availability) {
                continue;
            }
            if let Some(library) = libraries
                .iter()
                .find(|l| l.library.id == candidate.model.library.id && l.online)
            {
                return Ok(Resolved {
                    model: candidate.model,
                    app_data_dir: app_data_dir.to_path_buf(),
                    external_bookmark: Some(library.bookmark_data.clone()),
                });
            }
        }
        Err(if offline {
            RuntimeError::LibraryOffline
        } else {
            RuntimeError::Unavailable
        })
    }

    async fn acquire_resolved(
        &self,
        id: ModelId,
        resolved: Resolved,
        cancellation: &RuntimeCancellation,
    ) -> Result<ModelLease, RuntimeError> {
        let adapter = self
            .adapters
            .get(&id)
            .ok_or(RuntimeError::InvalidModel)?
            .clone();
        let entry = {
            let mut entries = self.entries.lock().await;
            entries
                .entry(id)
                .or_insert_with(|| Arc::new(Entry::default()))
                .clone()
        };
        let mut slot = tokio::select! {
            _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
            guard = entry.state.clone().lock_owned() => guard,
        };
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let location = &resolved.model.location;
        let expected_path = location
            .resolve(&resolved.app_data_dir, &resolved.model.library)
            .ok_or(RuntimeError::RegistryFailed)?;
        let same = slot.loaded.as_ref().is_some_and(|loaded| {
            loaded.library_id == location.library_id
                && loaded.bookmark_data == resolved.external_bookmark
                && (resolved.external_bookmark.is_some() || loaded.path == expected_path)
        });
        if !same {
            if entry.leases.load(Ordering::Acquire) > 0 {
                return Err(RuntimeError::Busy);
            }
            // The native loader owns the mutex guard. Dropping/aborting the
            // requesting future cannot allow another loader or unload to race
            // the native call. A cancelled initiating caller gets no lease;
            // another waiter may reuse the completed shared load.
            let worker = tokio::task::spawn_blocking(move || {
                slot.loaded = None;
                slot.loaded = Some(Arc::new(load_resolved(resolved, adapter)?));
                Ok::<_, RuntimeError>(slot)
            });
            slot = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
                result = worker => result.map_err(|_| RuntimeError::LoadFailed)??,
            };
        }
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let loaded = slot
            .loaded
            .as_ref()
            .ok_or(RuntimeError::LoadFailed)?
            .clone();
        entry.leases.fetch_add(1, Ordering::AcqRel);
        drop(slot);
        Ok(ModelLease { entry, loaded })
    }

    pub async fn unload(&self, model_id: &str) -> Result<bool, RuntimeError> {
        let id = ModelId::new(model_id).map_err(|_| RuntimeError::InvalidModel)?;
        if !self.adapters.contains_key(&id) {
            return Err(RuntimeError::InvalidModel);
        }
        let entry = { self.entries.lock().await.get(&id).cloned() };
        let Some(entry) = entry else {
            return Ok(false);
        };
        let mut slot = entry
            .state
            .clone()
            .try_lock_owned()
            .map_err(|_| RuntimeError::Busy)?;
        if entry.leases.load(Ordering::Acquire) > 0 {
            return Err(RuntimeError::Busy);
        }
        tokio::task::spawn_blocking(move || {
            let had_model = slot.loaded.take().is_some();
            drop(slot);
            had_model
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)
    }

    /// Install/removal is exclusive with loads and leases. The mutation task
    /// owns the guard if its IPC/session caller goes away midway through I/O.
    pub async fn mutate<T, F>(&self, model_id: &str, work: F) -> Result<T, RuntimeError>
    where
        T: Send + 'static,
        F: std::future::Future<Output = T> + Send + 'static,
    {
        let id = ModelId::new(model_id).map_err(|_| RuntimeError::InvalidModel)?;
        if !self.adapters.contains_key(&id) {
            return Err(RuntimeError::InvalidModel);
        }
        let entry = self.entries.lock().await.entry(id).or_default().clone();
        let mut slot = entry
            .state
            .clone()
            .try_lock_owned()
            .map_err(|_| RuntimeError::Busy)?;
        if entry.leases.load(Ordering::Acquire) > 0 {
            return Err(RuntimeError::Busy);
        }
        let slot = tokio::task::spawn_blocking(move || {
            slot.loaded = None;
            slot
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?;
        tokio::spawn(async move {
            let _slot = slot;
            work.await
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)
    }

    pub async fn snapshot(&self) -> RuntimeSnapshot {
        let entries: Vec<_> = self.entries.lock().await.values().cloned().collect();
        let mut snapshot = RuntimeSnapshot::default();
        for entry in entries {
            snapshot.active_leases += entry.leases.load(Ordering::Acquire);
            snapshot.busy_models += usize::from(entry.operations.available_permits() == 0);
            let Ok(slot) = entry.state.try_lock() else {
                snapshot.transitioning_models += 1;
                continue;
            };
            if let Some(loaded) = &slot.loaded {
                snapshot.loaded_models += 1;
                snapshot.estimated_resident_bytes += loaded.estimated_bytes.unwrap_or(0);
                snapshot.unknown_resident_models += usize::from(loaded.estimated_bytes.is_none());
            }
        }
        snapshot
    }
}

#[cfg(test)]
mod tests;

fn usable(availability: ModelAvailability) -> bool {
    matches!(availability, ModelAvailability::Available)
}

fn load_resolved(
    resolved: Resolved,
    adapter: Arc<dyn BackendAdapter>,
) -> Result<Loaded, RuntimeError> {
    let library_id = resolved.model.library.id.clone();
    let bookmark_data = resolved.external_bookmark.clone();
    #[cfg(target_os = "macos")]
    let grant = if let Some(bookmark) = &resolved.external_bookmark {
        Some(
            crate::mac_model_library::open_bookmarked_folder(bookmark)
                .ok_or(RuntimeError::LibraryOffline)?
                .0,
        )
    } else {
        None
    };
    #[cfg(not(target_os = "macos"))]
    if resolved.external_bookmark.is_some() {
        return Err(RuntimeError::Unavailable);
    }
    #[cfg(target_os = "macos")]
    let root = grant
        .as_ref()
        .map(|g| g.path())
        .unwrap_or(&resolved.app_data_dir);
    #[cfg(not(target_os = "macos"))]
    let root = &resolved.app_data_dir;
    if resolved.external_bookmark.is_some() {
        let (online, observed) =
            model_registry::observe_external_at(&resolved.model.library, Some(root))
                .map_err(|_| RuntimeError::RegistryFailed)?;
        if !online
            || !observed
                .iter()
                .any(|m| m.descriptor.id == resolved.model.descriptor.id && usable(m.availability))
        {
            return Err(RuntimeError::Unavailable);
        }
    }
    let path = resolved
        .model
        .location
        .resolve(root, &resolved.model.library)
        .ok_or(RuntimeError::RegistryFailed)?;
    if !path.is_dir()
        || path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(RuntimeError::Unavailable);
    }
    let backend = adapter.load(&path)?;
    Ok(Loaded {
        backend,
        library_id,
        path,
        estimated_bytes: adapter.estimated_resident_bytes(),
        bookmark_data,
        #[cfg(target_os = "macos")]
        _external_grant: grant,
    })
}
