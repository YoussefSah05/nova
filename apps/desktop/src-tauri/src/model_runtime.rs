//! Lazy, lease-owned execution of registered local models. Registry observations
//! identify a model; the runtime owns loaded backends and their filesystem grants.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use nova_core::models::{ModelAvailability, ModelId, ModelLibraryId};
use nova_enrich::{Embedder, FastEmbedder};
use nova_storage::{ModelRepository, RegisteredModel};
use nova_transcribe::{default_model, model_paths};
use sqlx::SqlitePool;
use tokio::sync::{Mutex, Notify, Semaphore};

use crate::model_registry;
use crate::resource_policy::{
    AdmissionController, PlaybackActivityGuard, ResourceBudget, WorkClass,
};

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
    Whisper(PathBuf),
}

trait BackendAdapter: Send + Sync {
    fn load(&self, path: &Path) -> Result<Backend, RuntimeError>;
    /// A resource-policy hook, not an admission decision. None means unknown.
    fn estimated_resident_bytes(&self) -> Option<u64> {
        None
    }
    /// Whether a loaded backend keeps heavyweight weights in this process and
    /// therefore counts against the resident-model budget.
    fn resident(&self) -> bool {
        true
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
    /// Whisper weights live in the per-window subprocess, not in Nova.
    fn resident(&self) -> bool {
        false
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
    /// Declared last so the budget is released only after the backend drops.
    residency: Option<ResidencyToken>,
}

/// Counts resident heavyweight backends, including loads in progress.
struct Residency {
    budget: usize,
    count: std::sync::Mutex<usize>,
    /// Signals lease release or residency release: either may make room.
    released: Notify,
}

/// One reserved resident position, owned by the loaded backend.
struct ResidencyToken(Arc<Residency>);

impl Drop for ResidencyToken {
    fn drop(&mut self) {
        let mut count = lock_count(&self.0.count);
        *count = count.saturating_sub(1);
        drop(count);
        self.0.released.notify_waiters();
    }
}

impl Residency {
    fn new(budget: usize) -> Arc<Self> {
        Arc::new(Self {
            budget: budget.max(1),
            count: std::sync::Mutex::new(0),
            released: Notify::new(),
        })
    }

    fn try_reserve(self: &Arc<Self>) -> Option<ResidencyToken> {
        let mut count = lock_count(&self.count);
        if *count >= self.budget {
            return None;
        }
        *count += 1;
        Some(ResidencyToken(Arc::clone(self)))
    }

    fn force_reserve(self: &Arc<Self>) -> ResidencyToken {
        *lock_count(&self.count) += 1;
        ResidencyToken(Arc::clone(self))
    }

    fn resident(&self) -> usize {
        *lock_count(&self.count)
    }
}

fn lock_count(mutex: &std::sync::Mutex<usize>) -> std::sync::MutexGuard<'_, usize> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Default)]
struct Slot {
    loaded: Option<Arc<Loaded>>,
}

struct Entry {
    state: Arc<Mutex<Slot>>,
    leases: AtomicUsize,
    operations: Arc<Semaphore>,
    resources: Arc<AdmissionController>,
    residency: Arc<Residency>,
    /// Counts against the resident budget and may be evicted when idle.
    resident: bool,
    /// Logical clock of the last lease, for least-recently-used eviction.
    last_used: AtomicU64,
}

impl Entry {
    fn new(resources: Arc<AdmissionController>, residency: Arc<Residency>, resident: bool) -> Self {
        Self {
            state: Arc::new(Mutex::new(Slot::default())),
            leases: AtomicUsize::new(0),
            operations: Arc::new(Semaphore::new(1)),
            resources,
            residency,
            resident,
            last_used: AtomicU64::new(0),
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
        // An unleased model may now be evicted for a waiting load.
        self.residency.released.notify_waiters();
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
        self.embed_documents_at(texts, WorkClass::Background, cancellation, on_progress)
            .await
    }

    pub async fn embed_documents_at(
        &self,
        texts: &[String],
        class: WorkClass,
        cancellation: &RuntimeCancellation,
        on_progress: impl Fn(usize),
    ) -> Result<Vec<Vec<f32>>, RuntimeError> {
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(256) {
            let count = batch.len();
            let batch = batch.to_vec();
            let result = self
                .clone()
                .run_blocking_at(class, cancellation, move |lease| {
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
        self.run_blocking_at(WorkClass::Background, cancellation, work)
            .await
    }

    pub async fn run_blocking_at<T: Send + 'static>(
        self,
        class: WorkClass,
        cancellation: &RuntimeCancellation,
        work: impl FnOnce(&ModelLease) -> T + Send + 'static,
    ) -> Result<T, RuntimeError> {
        let admission = self
            .entry
            .resources
            .admit(class, cancellation)
            .await
            .map_err(|_| RuntimeError::Cancelled)?;
        let permit = tokio::select! {
            _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
            permit = self.entry.operations.clone().acquire_owned() => permit.map_err(|_| RuntimeError::WorkerFailed)?,
        };
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let worker = tokio::task::spawn_blocking(move || {
            let _admission = admission;
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
        self.run_async_at(WorkClass::Background, cancellation, work)
            .await
    }

    pub async fn run_async_at<T, F>(
        self,
        class: WorkClass,
        cancellation: &RuntimeCancellation,
        work: impl FnOnce(ModelLease) -> F + Send + 'static,
    ) -> Result<T, RuntimeError>
    where
        T: Send + 'static,
        F: std::future::Future<Output = T> + Send + 'static,
    {
        let admission = self
            .entry
            .resources
            .admit(class, cancellation)
            .await
            .map_err(|_| RuntimeError::Cancelled)?;
        let permit = tokio::select! {
            _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
            permit = self.entry.operations.clone().acquire_owned() => permit.map_err(|_| RuntimeError::WorkerFailed)?,
        };
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let worker = tokio::spawn(async move {
            let _admission = admission;
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
    pub active_playback_sessions: usize,
    pub queued_work: usize,
    pub active_work_class: Option<WorkClass>,
    pub resident_models: usize,
    pub resident_model_budget: usize,
    pub queued_jobs: usize,
    pub active_jobs: usize,
    pub job_slots: usize,
    pub suspended_work: usize,
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
    resources: Arc<AdmissionController>,
    residency: Arc<Residency>,
    clock: AtomicU64,
}

impl ModelRuntimeManager {
    pub fn new(registry_pool: Arc<SqlitePool>) -> Self {
        Self::with_budget(registry_pool, ResourceBudget::DEFAULT)
    }

    pub fn with_budget(registry_pool: Arc<SqlitePool>, budget: ResourceBudget) -> Self {
        let mut adapters: HashMap<ModelId, Arc<dyn BackendAdapter>> = HashMap::new();
        for (id, adapter) in [
            (
                default_model().id,
                Arc::new(WhisperAdapter) as Arc<dyn BackendAdapter>,
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
            resources: Arc::new(AdmissionController::new(budget)),
            residency: Residency::new(budget.resident_models),
            clock: AtomicU64::new(0),
        }
    }

    fn entry_for(&self, entries: &mut HashMap<ModelId, Arc<Entry>>, id: &ModelId) -> Arc<Entry> {
        let resident = self.adapters.get(id).is_none_or(|a| a.resident());
        entries
            .entry(id.clone())
            .or_insert_with(|| {
                Arc::new(Entry::new(
                    Arc::clone(&self.resources),
                    Arc::clone(&self.residency),
                    resident,
                ))
            })
            .clone()
    }

    /// Reserve a resident position before a cold load. Evicts the least
    /// recently used idle, unleased model first; never touches a leased one.
    /// Realtime/interactive work may exceed the budget when nothing is
    /// evictable; lower classes wait cancellably for a lease to end. Callers
    /// must not hold another lease while acquiring (none do).
    async fn reserve_residency(
        &self,
        id: &ModelId,
        class: WorkClass,
        cancellation: &RuntimeCancellation,
    ) -> Result<ResidencyToken, RuntimeError> {
        loop {
            let notified = self.residency.released.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            if let Some(token) = self.residency.try_reserve() {
                return Ok(token);
            }
            if self.evict_idle(id).await {
                continue;
            }
            if class.may_exceed_residency() {
                return Ok(self.residency.force_reserve());
            }
            tokio::select! {
                _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
                _ = notified => {}
            }
        }
    }

    async fn evict_idle(&self, except: &ModelId) -> bool {
        let mut candidates: Vec<Arc<Entry>> = self
            .entries
            .lock()
            .await
            .iter()
            .filter(|(id, e)| *id != except && e.resident && e.leases.load(Ordering::Acquire) == 0)
            .map(|(_, e)| Arc::clone(e))
            .collect();
        candidates.sort_by_key(|e| e.last_used.load(Ordering::Acquire));
        for entry in candidates {
            // Same exclusion as `unload`: the lifecycle lock plus a zero lease
            // count proves no lease can be created or is outstanding.
            let Ok(mut slot) = entry.state.clone().try_lock_owned() else {
                continue;
            };
            if entry.leases.load(Ordering::Acquire) > 0 {
                continue;
            }
            let Some(old) = slot.loaded.take() else {
                continue;
            };
            let _ = tokio::task::spawn_blocking(move || {
                drop(old);
                drop(slot);
            })
            .await;
            return true;
        }
        false
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
        self.acquire_at(app_data_dir, model_id, WorkClass::Background, cancellation)
            .await
    }

    pub async fn acquire_at(
        &self,
        app_data_dir: &Path,
        model_id: &str,
        class: WorkClass,
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
        self.acquire_resolved_at(id, resolved, class, cancellation)
            .await
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

    #[cfg(test)]
    async fn acquire_resolved(
        &self,
        id: ModelId,
        resolved: Resolved,
        cancellation: &RuntimeCancellation,
    ) -> Result<ModelLease, RuntimeError> {
        self.acquire_resolved_at(id, resolved, WorkClass::Background, cancellation)
            .await
    }

    async fn acquire_resolved_at(
        &self,
        id: ModelId,
        resolved: Resolved,
        class: WorkClass,
        cancellation: &RuntimeCancellation,
    ) -> Result<ModelLease, RuntimeError> {
        let adapter = self
            .adapters
            .get(&id)
            .ok_or(RuntimeError::InvalidModel)?
            .clone();
        let entry = {
            let mut entries = self.entries.lock().await;
            self.entry_for(&mut entries, &id)
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
            // Release a replaced generation before reserving residency so it
            // cannot count against its own successor.
            if let Some(old) = slot.loaded.take() {
                let _ = tokio::task::spawn_blocking(move || drop(old)).await;
            }
            let reservation = if entry.resident {
                Some(self.reserve_residency(&id, class, cancellation).await?)
            } else {
                None
            };
            // The native loader owns the mutex guard. Dropping/aborting the
            // requesting future cannot allow another loader or unload to race
            // the native call. A cancelled initiating caller gets no lease;
            // another waiter may reuse the completed shared load.
            let admission = entry
                .resources
                .admit(class, cancellation)
                .await
                .map_err(|_| RuntimeError::Cancelled)?;
            let worker = tokio::task::spawn_blocking(move || {
                let _admission = admission;
                // A failed load drops the reservation with its error.
                let mut loaded = load_resolved(resolved, adapter)?;
                loaded.residency = reservation;
                slot.loaded = Some(Arc::new(loaded));
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
        entry.last_used.store(
            self.clock.fetch_add(1, Ordering::AcqRel) + 1,
            Ordering::Release,
        );
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
        let entry = {
            let mut entries = self.entries.lock().await;
            self.entry_for(&mut entries, &id)
        };
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
        let admission = self.resources.snapshot();
        snapshot.active_playback_sessions = admission.playback_sessions;
        snapshot.queued_work = admission.queued_work;
        snapshot.active_work_class = admission.active_work_class;
        snapshot.resident_models = self.residency.resident();
        snapshot.resident_model_budget = self.residency.budget;
        snapshot.queued_jobs = admission.queued_jobs;
        snapshot.active_jobs = admission.active_jobs;
        snapshot.job_slots = admission.budget.job_slots;
        snapshot.suspended_work = admission.suspended_work;
        snapshot
    }

    /// Playback holds this guard for the player session. Background model
    /// inference remains queued until every active player session releases it.
    pub fn playback_activity(&self) -> PlaybackActivityGuard {
        self.resources.playback_started()
    }

    /// Share the deterministic admission lane with persisted desktop jobs.
    pub fn resource_admission(&self) -> Arc<AdmissionController> {
        Arc::clone(&self.resources)
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
        residency: None,
    })
}
