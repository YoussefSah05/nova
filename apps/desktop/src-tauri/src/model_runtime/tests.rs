use super::*;
use crate::resource_policy::{ResourceBudget, WorkClass};
use nova_enrich::EnrichError;
use std::sync::Mutex as SyncMutex;
use std::time::Duration;
use tokio::task::JoinSet;

const DEADLINE: Duration = Duration::from_secs(5);

struct FakeModel(Arc<AtomicUsize>);
impl Drop for FakeModel {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Embedder for FakeModel {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EnrichError> {
        Ok(vec![vec![1.0]; texts.len()])
    }
    fn embed_query(&self, _: &str) -> Result<Vec<f32>, EnrichError> {
        Ok(vec![1.0])
    }
}

#[derive(Default)]
struct FakeAdapter {
    loads: AtomicUsize,
    drops: Arc<AtomicUsize>,
    fail: AtomicBool,
    gate: SyncMutex<Option<std::sync::mpsc::Receiver<()>>>,
    started: Notify,
    /// Models a subprocess backend (Whisper) that keeps no weights resident.
    transient: bool,
}
impl BackendAdapter for FakeAdapter {
    fn resident(&self) -> bool {
        !self.transient
    }

    fn load(&self, _: &Path) -> Result<Backend, RuntimeError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        let gate = self.gate.lock().unwrap().take();
        self.started.notify_one();
        if let Some(gate) = gate {
            gate.recv_timeout(DEADLINE).unwrap();
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(RuntimeError::LoadFailed);
        }
        Ok(Backend::Embedding(Box::new(FakeModel(self.drops.clone()))))
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    runtime: Arc<ModelRuntimeManager>,
    adapter: Arc<FakeAdapter>,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models/embeddings")).unwrap();
        let adapter = Arc::new(FakeAdapter::default());
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let mut runtime = ModelRuntimeManager::new(Arc::new(pool));
        runtime.adapters.insert(Self::id(), adapter.clone());
        Self {
            dir,
            runtime: Arc::new(runtime),
            adapter,
        }
    }
    fn id() -> ModelId {
        ModelId::new(nova_enrich::MODEL_ID).unwrap()
    }
    fn resolved(&self) -> Resolved {
        let (_, models) = model_registry::discover_internal(self.dir.path()).unwrap();
        Resolved {
            model: models
                .into_iter()
                .find(|m| m.descriptor.id == Self::id())
                .unwrap(),
            app_data_dir: self.dir.path().into(),
            external_bookmark: None,
        }
    }
    async fn acquire(&self) -> ModelLease {
        self.runtime
            .acquire_resolved(Self::id(), self.resolved(), &RuntimeCancellation::default())
            .await
            .unwrap()
    }
    fn gate_load(&self) -> std::sync::mpsc::Sender<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.adapter.gate.lock().unwrap() = Some(rx);
        tx
    }
}

async fn no_leases(runtime: &ModelRuntimeManager) {
    tokio::time::timeout(DEADLINE, async {
        while runtime.snapshot().await.active_leases != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_use_loads_once_and_unload_waits_for_every_clone() {
    let fixture = Fixture::new();
    let mut tasks = JoinSet::new();
    for _ in 0..16 {
        let runtime = fixture.runtime.clone();
        let resolved = fixture.resolved();
        tasks.spawn(async move {
            runtime
                .acquire_resolved(Fixture::id(), resolved, &RuntimeCancellation::default())
                .await
                .unwrap()
        });
    }
    let mut leases = vec![];
    while let Some(result) = tokio::time::timeout(DEADLINE, tasks.join_next())
        .await
        .unwrap()
    {
        leases.push(result.unwrap());
    }
    assert_eq!(fixture.adapter.loads.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runtime.snapshot().await.active_leases, 16);
    let last = leases[0].clone();
    drop(leases);
    assert_eq!(
        fixture.runtime.unload(nova_enrich::MODEL_ID).await,
        Err(RuntimeError::Busy)
    );
    assert_eq!(fixture.adapter.drops.load(Ordering::SeqCst), 0);
    drop(last);
    assert!(fixture.runtime.unload(nova_enrich::MODEL_ID).await.unwrap());
    assert_eq!(fixture.adapter.drops.load(Ordering::SeqCst), 1);
    let again = fixture.acquire().await;
    assert_eq!(fixture.adapter.loads.load(Ordering::SeqCst), 2);
    drop(again);
}

#[tokio::test]
async fn cancelled_or_aborted_load_owner_cannot_start_an_overlapping_native_load() {
    for abort in [false, true] {
        let fixture = Fixture::new();
        let release = fixture.gate_load();
        let runtime = fixture.runtime.clone();
        let resolved = fixture.resolved();
        let cancel = RuntimeCancellation::default();
        let token = cancel.clone();
        let first = tokio::spawn(async move {
            runtime
                .acquire_resolved(Fixture::id(), resolved, &token)
                .await
        });
        tokio::time::timeout(DEADLINE, fixture.adapter.started.notified())
            .await
            .unwrap();
        if abort {
            first.abort();
            assert!(matches!(first.await, Err(e) if e.is_cancelled()));
        } else {
            cancel.cancel();
            assert!(matches!(first.await.unwrap(), Err(RuntimeError::Cancelled)));
        }
        assert_eq!(
            fixture.runtime.unload(nova_enrich::MODEL_ID).await,
            Err(RuntimeError::Busy)
        );
        assert_eq!(fixture.runtime.snapshot().await.transitioning_models, 1);
        let runtime = fixture.runtime.clone();
        let resolved = fixture.resolved();
        let second = tokio::spawn(async move {
            runtime
                .acquire_resolved(Fixture::id(), resolved, &RuntimeCancellation::default())
                .await
        });
        release.send(()).unwrap();
        let lease = tokio::time::timeout(DEADLINE, second)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(fixture.adapter.loads.load(Ordering::SeqCst), 1);
        drop(lease);
        assert!(fixture.runtime.unload(nova_enrich::MODEL_ID).await.unwrap());
    }
}

#[tokio::test]
async fn cancelled_waiter_gets_no_lease_and_does_not_cancel_shared_load() {
    let fixture = Fixture::new();
    let release = fixture.gate_load();
    let runtime = fixture.runtime.clone();
    let resolved = fixture.resolved();
    let first = tokio::spawn(async move {
        runtime
            .acquire_resolved(Fixture::id(), resolved, &RuntimeCancellation::default())
            .await
    });
    tokio::time::timeout(DEADLINE, fixture.adapter.started.notified())
        .await
        .unwrap();
    let cancel = RuntimeCancellation::default();
    cancel.cancel();
    let result = fixture
        .runtime
        .acquire_resolved(Fixture::id(), fixture.resolved(), &cancel)
        .await;
    assert!(matches!(result, Err(RuntimeError::Cancelled)));
    release.send(()).unwrap();
    drop(first.await.unwrap().unwrap());
    assert_eq!(fixture.runtime.snapshot().await.active_leases, 0);
    assert_eq!(fixture.adapter.loads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn load_failure_releases_lock_and_is_retryable() {
    let fixture = Fixture::new();
    fixture.adapter.fail.store(true, Ordering::SeqCst);
    let result = fixture
        .runtime
        .acquire_resolved(
            Fixture::id(),
            fixture.resolved(),
            &RuntimeCancellation::default(),
        )
        .await;
    assert!(matches!(result, Err(RuntimeError::LoadFailed)));
    assert_eq!(fixture.runtime.snapshot().await.loaded_models, 0);
    fixture.adapter.fail.store(false, Ordering::SeqCst);
    drop(fixture.acquire().await);
    assert_eq!(fixture.adapter.loads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cancelled_native_work_retains_lease_and_admission_until_it_finishes() {
    let fixture = Fixture::new();
    let lease = fixture.acquire().await;
    let queued = lease.clone();
    let (release, finish) = std::sync::mpsc::channel();
    let (started, waiting) = tokio::sync::oneshot::channel();
    let cancel = RuntimeCancellation::default();
    let token = cancel.clone();
    let first = tokio::spawn(async move {
        lease
            .run_blocking(&token, move |_| {
                started.send(()).unwrap();
                finish.recv_timeout(DEADLINE).unwrap();
            })
            .await
    });
    tokio::time::timeout(DEADLINE, waiting)
        .await
        .unwrap()
        .unwrap();
    cancel.cancel();
    assert!(matches!(first.await.unwrap(), Err(RuntimeError::Cancelled)));
    assert_eq!(
        fixture.runtime.unload(nova_enrich::MODEL_ID).await,
        Err(RuntimeError::Busy)
    );
    assert_eq!(fixture.runtime.snapshot().await.busy_models, 1);
    // A cancelled queued operation never enters its native closure.
    let result = queued
        .run_blocking(&cancel, |_| panic!("cancelled queued inference ran"))
        .await;
    assert!(matches!(result, Err(RuntimeError::Cancelled)));
    release.send(()).unwrap();
    no_leases(&fixture.runtime).await;
    assert!(fixture.runtime.unload(nova_enrich::MODEL_ID).await.unwrap());
    assert_eq!(fixture.adapter.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn aborted_async_caller_does_not_release_child_operation_lease() {
    let fixture = Fixture::new();
    let lease = fixture.acquire().await;
    let (release, finish) = tokio::sync::oneshot::channel();
    let (started, waiting) = tokio::sync::oneshot::channel();
    let first = tokio::spawn(async move {
        lease
            .run_async(&RuntimeCancellation::default(), move |lease| async move {
                started.send(()).unwrap();
                finish.await.unwrap();
                assert!(lease.embedder().is_some());
            })
            .await
    });
    waiting.await.unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert_eq!(
        fixture.runtime.unload(nova_enrich::MODEL_ID).await,
        Err(RuntimeError::Busy)
    );
    release.send(()).unwrap();
    no_leases(&fixture.runtime).await;
    assert!(fixture.runtime.unload(nova_enrich::MODEL_ID).await.unwrap());
}

#[tokio::test]
async fn replacement_location_does_not_change_identity_or_evict_active_generation() {
    let fixture = Fixture::new();
    let old = fixture.acquire().await;
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(other.path().join("models/embeddings")).unwrap();
    let mut replacement = fixture.resolved();
    replacement.app_data_dir = other.path().into();
    assert!(matches!(
        fixture
            .runtime
            .acquire_resolved(Fixture::id(), replacement, &RuntimeCancellation::default())
            .await,
        Err(RuntimeError::Busy)
    ));
    drop(old);
    let mut replacement = fixture.resolved();
    replacement.app_data_dir = other.path().into();
    let new = fixture
        .runtime
        .acquire_resolved(Fixture::id(), replacement, &RuntimeCancellation::default())
        .await
        .unwrap();
    assert_eq!(new.loaded.path, other.path().join("models/embeddings"));
    assert_eq!(fixture.adapter.loads.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.adapter.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn model_mutation_is_exclusive_and_retained_after_caller_abort() {
    let fixture = Fixture::new();
    let lease = fixture.acquire().await;
    assert_eq!(
        fixture
            .runtime
            .mutate(nova_enrich::MODEL_ID, async {})
            .await,
        Err(RuntimeError::Busy)
    );
    drop(lease);
    let (release, finish) = tokio::sync::oneshot::channel();
    let (started, waiting) = tokio::sync::oneshot::channel();
    let runtime = fixture.runtime.clone();
    let mutation = tokio::spawn(async move {
        runtime
            .mutate(nova_enrich::MODEL_ID, async {
                started.send(()).unwrap();
                finish.await.unwrap();
            })
            .await
    });
    waiting.await.unwrap();
    mutation.abort();
    assert!(mutation.await.unwrap_err().is_cancelled());
    assert_eq!(
        fixture.runtime.unload(nova_enrich::MODEL_ID).await,
        Err(RuntimeError::Busy)
    );
    release.send(()).unwrap();
    let lease = tokio::time::timeout(DEADLINE, fixture.acquire())
        .await
        .unwrap();
    assert_eq!(fixture.adapter.loads.load(Ordering::SeqCst), 2);
    drop(lease);
}

#[tokio::test]
async fn worker_panic_releases_lease_and_admission() {
    let fixture = Fixture::new();
    let lease = fixture.acquire().await;
    let result = lease
        .run_blocking(&RuntimeCancellation::default(), |_| {
            panic!("synthetic native failure")
        })
        .await;
    assert!(matches!(result, Err(RuntimeError::WorkerFailed)));
    no_leases(&fixture.runtime).await;
    let lease = fixture.acquire().await;
    assert_eq!(
        lease
            .run_blocking(&RuntimeCancellation::default(), |_| 7)
            .await
            .unwrap(),
        7
    );
    assert!(fixture.runtime.unload(nova_enrich::MODEL_ID).await.unwrap());
}

#[tokio::test]
async fn document_batches_release_admission_and_observe_cancellation_between_batches() {
    let fixture = Fixture::new();
    let lease = fixture.acquire().await;
    let texts = vec!["fixture".to_string(); 513];
    let cancel = RuntimeCancellation::default();
    let result = lease
        .embed_documents(&texts, &cancel, |processed| {
            assert_eq!(processed, 256);
            assert_eq!(lease.entry.operations.available_permits(), 1);
            cancel.cancel();
        })
        .await;
    assert!(matches!(result, Err(RuntimeError::Cancelled)));
    let snapshot = fixture.runtime.snapshot().await;
    assert_eq!(snapshot.loaded_models, 1);
    assert_eq!(snapshot.unknown_resident_models, 1);
    assert_eq!(snapshot.estimated_resident_bytes, 0);
    assert_eq!(snapshot.active_leases, 1);
    assert_eq!(snapshot.busy_models, 0);
    drop(lease);
    fixture.runtime.unload(nova_enrich::MODEL_ID).await.unwrap();
}

#[tokio::test]
async fn playback_prioritizes_interactive_runtime_work_and_pauses_background_work() {
    let fixture = Fixture::new();
    let background = fixture.acquire().await;
    let interactive = fixture.acquire().await;
    let playback = fixture.runtime.playback_activity();
    let waiting_lease = background.clone();
    let waiting = tokio::spawn(async move {
        waiting_lease
            .run_blocking_at(
                WorkClass::Background,
                &RuntimeCancellation::default(),
                |_| (),
            )
            .await
    });

    tokio::time::timeout(DEADLINE, async {
        while fixture.runtime.snapshot().await.queued_work != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.runtime.snapshot().await.active_playback_sessions, 1);

    interactive
        .run_blocking_at(
            WorkClass::Interactive,
            &RuntimeCancellation::default(),
            |_| (),
        )
        .await
        .unwrap();
    assert!(!waiting.is_finished());
    drop(playback);
    tokio::time::timeout(DEADLINE, waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn playback_defers_cold_model_initialization_until_playback_ends() {
    let fixture = Fixture::new();
    let playback = fixture.runtime.playback_activity();
    let runtime = fixture.runtime.clone();
    let resolved = fixture.resolved();
    let waiting = tokio::spawn(async move {
        runtime
            .acquire_resolved_at(
                Fixture::id(),
                resolved,
                WorkClass::Background,
                &RuntimeCancellation::default(),
            )
            .await
    });
    tokio::time::timeout(DEADLINE, async {
        while fixture.runtime.snapshot().await.queued_work != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.adapter.loads.load(Ordering::SeqCst), 0);
    drop(playback);
    let lease = tokio::time::timeout(DEADLINE, waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(fixture.adapter.loads.load(Ordering::SeqCst), 1);
    drop(lease);
}

#[tokio::test]
async fn per_model_admission_stays_exclusive_after_cancelled_caller_returns() {
    let fixture = Fixture::new();
    let first = fixture.acquire().await;
    let second = fixture.acquire().await;
    let (release, finish) = std::sync::mpsc::channel();
    let (started, waiting) = tokio::sync::oneshot::channel();
    let cancel = RuntimeCancellation::default();
    let token = cancel.clone();
    let first_task = tokio::spawn(async move {
        first
            .run_blocking(&token, move |_| {
                started.send(()).unwrap();
                finish.recv_timeout(DEADLINE).unwrap();
            })
            .await
    });
    waiting.await.unwrap();
    cancel.cancel();
    assert!(matches!(
        first_task.await.unwrap(),
        Err(RuntimeError::Cancelled)
    ));
    let entered = Arc::new(AtomicBool::new(false));
    let flag = entered.clone();
    let second_task = tokio::spawn(async move {
        second
            .run_blocking(&RuntimeCancellation::default(), move |_| {
                flag.store(true, Ordering::SeqCst)
            })
            .await
    });
    assert!(!entered.load(Ordering::SeqCst));
    assert_eq!(fixture.runtime.snapshot().await.busy_models, 1);
    release.send(()).unwrap();
    tokio::time::timeout(DEADLINE, second_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(entered.load(Ordering::SeqCst));
    no_leases(&fixture.runtime).await;
}

/// A structurally complete Whisper install. Tests pair its registered ID with
/// a fake adapter, so it doubles as a generic second model.
fn whisper_fixture(root: &Path) {
    let dir = root.join("models/whisper");
    std::fs::create_dir_all(&dir).unwrap();
    let paths = model_paths(&dir, &default_model());
    std::fs::write(&paths.ggml, b"fixture").unwrap();
    std::fs::create_dir_all(&paths.coreml_dir).unwrap();
    std::fs::write(paths.coreml_dir.join("fixture"), b"x").unwrap();
}

#[tokio::test]
async fn registry_resolution_is_lazy_and_rechecks_missing_locations() {
    let dir = tempfile::tempdir().unwrap();
    let pool = nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
        .await
        .unwrap();
    let adapter = Arc::new(FakeAdapter::default());
    let mut runtime = ModelRuntimeManager::new(Arc::new(pool));
    let id = default_model().id;
    runtime
        .adapters
        .insert(ModelId::new(id).unwrap(), adapter.clone());
    assert!(!runtime.is_available(dir.path(), id).await.unwrap());
    assert!(matches!(
        runtime
            .acquire(dir.path(), id, &RuntimeCancellation::default())
            .await,
        Err(RuntimeError::Unavailable)
    ));
    assert!(matches!(
        runtime
            .acquire(dir.path(), "unknown-model", &RuntimeCancellation::default())
            .await,
        Err(RuntimeError::InvalidModel)
    ));
    assert_eq!(adapter.loads.load(Ordering::SeqCst), 0);
    whisper_fixture(dir.path());
    assert!(runtime.is_available(dir.path(), id).await.unwrap());
    assert_eq!(adapter.loads.load(Ordering::SeqCst), 0);
    let lease = runtime
        .acquire(dir.path(), id, &RuntimeCancellation::default())
        .await
        .unwrap();
    std::fs::remove_dir_all(dir.path().join("models")).unwrap();
    assert!(matches!(
        runtime
            .acquire(dir.path(), id, &RuntimeCancellation::default())
            .await,
        Err(RuntimeError::Unavailable)
    ));
    assert_eq!(runtime.unload(id).await, Err(RuntimeError::Busy));
    whisper_fixture(dir.path());
    let restored = runtime
        .acquire(dir.path(), id, &RuntimeCancellation::default())
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&lease.loaded, &restored.loaded));
    drop((lease, restored));
    assert!(runtime.unload(id).await.unwrap());
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn external_grant_survives_use_and_disconnect_preserves_logical_identity() {
    use nova_core::models::{ModelLibrary, RelativeModelPath};
    use nova_storage::ExternalLibraryRecord;
    use objc2_foundation::{NSString, NSURLBookmarkCreationOptions, NSURL};

    let dir = tempfile::tempdir().unwrap();
    let external = dir.path().join("external");
    whisper_fixture(&external);
    let url =
        NSURL::fileURLWithPath_isDirectory(&NSString::from_str(external.to_str().unwrap()), true);
    let data = url
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            NSURLBookmarkCreationOptions::empty(),
            None,
            None,
        )
        .unwrap();
    let mut bookmark_data = vec![0];
    bookmark_data.extend_from_slice(&data.to_vec());
    let pool = Arc::new(
        nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
            .await
            .unwrap(),
    );
    model_registry::register_external(
        &pool,
        ExternalLibraryRecord {
            library: ModelLibrary {
                id: ModelLibraryId::new("external-fixture").unwrap(),
                relative_root: RelativeModelPath::new("models").unwrap(),
            },
            display_name: "test library".into(),
            bookmark_data,
            online: false,
            observed_at: 0,
        },
    )
    .await
    .unwrap();
    let mut runtime = ModelRuntimeManager::new(pool.clone());
    let adapter = Arc::new(FakeAdapter::default());
    let id = default_model().id;
    runtime
        .adapters
        .insert(ModelId::new(id).unwrap(), adapter.clone());
    let lease = runtime
        .acquire(dir.path(), id, &RuntimeCancellation::default())
        .await
        .unwrap();
    assert!(lease.loaded._external_grant.is_some());
    assert_eq!(lease.loaded.library_id.as_str(), "external-fixture");
    std::fs::remove_dir_all(&external).unwrap();
    assert!(matches!(
        runtime
            .acquire(dir.path(), id, &RuntimeCancellation::default())
            .await,
        Err(RuntimeError::LibraryOffline)
    ));
    assert_eq!(runtime.unload(id).await, Err(RuntimeError::Busy));
    let rows = ModelRepository::new(&pool)
        .list_external_models()
        .await
        .unwrap();
    assert!(rows.iter().any(|row| row.model.descriptor.id.as_str() == id
        && row.model.availability == ModelAvailability::Unavailable));
    whisper_fixture(&external);
    drop(lease);
    runtime.unload(id).await.unwrap();
    let restored = runtime
        .acquire(dir.path(), id, &RuntimeCancellation::default())
        .await
        .unwrap();
    assert_eq!(restored.loaded.library_id.as_str(), "external-fixture");
    assert_eq!(adapter.loads.load(Ordering::SeqCst), 2);
}

/// Two models behind fake adapters: E5 and a second one registered under the
/// Whisper ID (resident unless the fake is `transient`).
struct TwoModels {
    dir: tempfile::TempDir,
    runtime: Arc<ModelRuntimeManager>,
    embed: Arc<FakeAdapter>,
    second: Arc<FakeAdapter>,
}

impl TwoModels {
    fn new(budget: ResourceBudget) -> Self {
        Self::with_second(budget, FakeAdapter::default())
    }

    fn with_second(budget: ResourceBudget, second: FakeAdapter) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("models/embeddings")).unwrap();
        whisper_fixture(dir.path());
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let embed = Arc::new(FakeAdapter::default());
        let second = Arc::new(second);
        let mut runtime = ModelRuntimeManager::with_budget(Arc::new(pool), budget);
        runtime.adapters.insert(Self::embed_id(), embed.clone());
        runtime.adapters.insert(Self::second_id(), second.clone());
        Self {
            dir,
            runtime: Arc::new(runtime),
            embed,
            second,
        }
    }

    fn embed_id() -> ModelId {
        ModelId::new(nova_enrich::MODEL_ID).unwrap()
    }

    fn second_id() -> ModelId {
        ModelId::new(default_model().id).unwrap()
    }

    fn resolved(&self, id: &ModelId) -> Resolved {
        let (_, models) = model_registry::discover_internal(self.dir.path()).unwrap();
        Resolved {
            model: models.into_iter().find(|m| m.descriptor.id == *id).unwrap(),
            app_data_dir: self.dir.path().into(),
            external_bookmark: None,
        }
    }

    async fn acquire(
        &self,
        id: ModelId,
        class: WorkClass,
        cancellation: &RuntimeCancellation,
    ) -> Result<ModelLease, RuntimeError> {
        let resolved = self.resolved(&id);
        self.runtime
            .acquire_resolved_at(id, resolved, class, cancellation)
            .await
    }

    fn spawn_acquire(
        &self,
        id: ModelId,
        class: WorkClass,
        cancellation: RuntimeCancellation,
    ) -> tokio::task::JoinHandle<Result<ModelLease, RuntimeError>> {
        let runtime = self.runtime.clone();
        let resolved = self.resolved(&id);
        tokio::spawn(async move {
            runtime
                .acquire_resolved_at(id, resolved, class, &cancellation)
                .await
        })
    }
}

#[tokio::test]
async fn low_resource_budget_evicts_the_idle_model_before_loading_another() {
    let models = TwoModels::new(ResourceBudget::LOW_RESOURCE);
    let cancel = RuntimeCancellation::default();
    let lease = models
        .acquire(TwoModels::embed_id(), WorkClass::Background, &cancel)
        .await
        .unwrap();
    drop(lease);
    let snapshot = models.runtime.snapshot().await;
    assert_eq!(
        (snapshot.resident_models, snapshot.resident_model_budget),
        (1, 1)
    );

    let lease = models
        .acquire(TwoModels::second_id(), WorkClass::Background, &cancel)
        .await
        .unwrap();
    assert_eq!(
        models.embed.drops.load(Ordering::SeqCst),
        1,
        "idle E5 evicted"
    );
    let snapshot = models.runtime.snapshot().await;
    assert_eq!(snapshot.loaded_models, 1);
    assert_eq!(snapshot.resident_models, 1);
    drop(lease);
}

#[tokio::test]
async fn default_budget_keeps_both_in_process_models_resident() {
    let models = TwoModels::new(ResourceBudget::DEFAULT);
    let cancel = RuntimeCancellation::default();
    drop(
        models
            .acquire(TwoModels::embed_id(), WorkClass::Background, &cancel)
            .await
            .unwrap(),
    );
    drop(
        models
            .acquire(TwoModels::second_id(), WorkClass::Background, &cancel)
            .await
            .unwrap(),
    );
    assert_eq!(models.embed.drops.load(Ordering::SeqCst), 0);
    assert_eq!(models.runtime.snapshot().await.resident_models, 2);
}

#[tokio::test]
async fn leased_model_is_never_evicted_and_lower_classes_wait_for_its_release() {
    let models = TwoModels::new(ResourceBudget::LOW_RESOURCE);
    let held = models
        .acquire(
            TwoModels::embed_id(),
            WorkClass::Background,
            &RuntimeCancellation::default(),
        )
        .await
        .unwrap();
    let waiting = models.spawn_acquire(
        TwoModels::second_id(),
        WorkClass::Metadata,
        RuntimeCancellation::default(),
    );
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(!waiting.is_finished(), "must not exceed the budget");
    assert_eq!(models.second.loads.load(Ordering::SeqCst), 0);
    assert_eq!(
        models.embed.drops.load(Ordering::SeqCst),
        0,
        "leased model kept"
    );
    assert_eq!(
        models.runtime.unload(nova_enrich::MODEL_ID).await,
        Err(RuntimeError::Busy),
        "explicit unload also refuses a leased model"
    );

    drop(held);
    let lease = tokio::time::timeout(DEADLINE, waiting)
        .await
        .expect("released lease wakes the residency waiter")
        .unwrap()
        .unwrap();
    assert_eq!(models.embed.drops.load(Ordering::SeqCst), 1);
    assert_eq!(models.second.loads.load(Ordering::SeqCst), 1);
    assert_eq!(models.runtime.snapshot().await.resident_models, 1);
    drop(lease);
}

#[tokio::test]
async fn realtime_work_is_not_blocked_by_a_suspended_low_priority_lease_holder() {
    let models = TwoModels::new(ResourceBudget::LOW_RESOURCE);
    // A background job holds E5 and is suspended by playback.
    let background = models
        .acquire(
            TwoModels::embed_id(),
            WorkClass::Background,
            &RuntimeCancellation::default(),
        )
        .await
        .unwrap();
    let playback = models.runtime.playback_activity();
    let suspended_lease = background.clone();
    let suspended = tokio::spawn(async move {
        suspended_lease
            .run_blocking_at(
                WorkClass::Background,
                &RuntimeCancellation::default(),
                |_| (),
            )
            .await
    });
    tokio::time::timeout(DEADLINE, async {
        while models.runtime.snapshot().await.suspended_work != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    // Realtime work loads a second model over budget instead of inverting.
    let realtime = tokio::time::timeout(
        DEADLINE,
        models.acquire(
            TwoModels::second_id(),
            WorkClass::Realtime,
            &RuntimeCancellation::default(),
        ),
    )
    .await
    .expect("realtime must not wait on a suspended lower-priority lease")
    .unwrap();
    realtime
        .clone()
        .run_blocking_at(WorkClass::Realtime, &RuntimeCancellation::default(), |_| ())
        .await
        .unwrap();
    let snapshot = models.runtime.snapshot().await;
    assert_eq!(snapshot.resident_models, 2, "temporarily over budget");
    assert_eq!(snapshot.resident_model_budget, 1);
    assert!(!suspended.is_finished());

    drop(playback);
    tokio::time::timeout(DEADLINE, suspended)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(background);
    drop(realtime);
    no_leases(&models.runtime).await;
    // The next constrained load restores the budget by evicting idle models.
    let lease = models
        .acquire(
            TwoModels::embed_id(),
            WorkClass::Background,
            &RuntimeCancellation::default(),
        )
        .await
        .unwrap();
    drop(lease);
    assert!(models.runtime.unload(default_model().id).await.unwrap());
    assert_eq!(models.runtime.snapshot().await.resident_models, 1);
}

#[tokio::test]
async fn cancelled_residency_waiter_and_failed_loads_leave_no_reservation() {
    let second = FakeAdapter::default();
    second.fail.store(true, Ordering::SeqCst);
    let models = TwoModels::with_second(ResourceBudget::LOW_RESOURCE, second);
    let held = models
        .acquire(
            TwoModels::embed_id(),
            WorkClass::Background,
            &RuntimeCancellation::default(),
        )
        .await
        .unwrap();
    let cancel = RuntimeCancellation::default();
    let waiting = models.spawn_acquire(
        TwoModels::second_id(),
        WorkClass::Background,
        cancel.clone(),
    );
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    cancel.cancel();
    assert!(matches!(
        tokio::time::timeout(DEADLINE, waiting)
            .await
            .unwrap()
            .unwrap(),
        Err(RuntimeError::Cancelled)
    ));
    assert_eq!(models.runtime.snapshot().await.resident_models, 1);
    drop(held);

    // A failed load releases the reservation it made (after evicting E5).
    assert!(matches!(
        models
            .acquire(
                TwoModels::second_id(),
                WorkClass::Background,
                &RuntimeCancellation::default()
            )
            .await,
        Err(RuntimeError::LoadFailed)
    ));
    assert_eq!(models.runtime.snapshot().await.resident_models, 0);
    let lease = models
        .acquire(
            TwoModels::embed_id(),
            WorkClass::Background,
            &RuntimeCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(models.runtime.snapshot().await.resident_models, 1);
    drop(lease);
}

#[tokio::test]
async fn subprocess_backends_do_not_consume_the_resident_budget() {
    let second = FakeAdapter {
        transient: true,
        ..FakeAdapter::default()
    };
    let models = TwoModels::with_second(ResourceBudget::LOW_RESOURCE, second);
    let cancel = RuntimeCancellation::default();
    let embed = models
        .acquire(TwoModels::embed_id(), WorkClass::Background, &cancel)
        .await
        .unwrap();
    let transient = tokio::time::timeout(
        DEADLINE,
        models.acquire(TwoModels::second_id(), WorkClass::Background, &cancel),
    )
    .await
    .expect("a non-resident backend never waits for residency")
    .unwrap();
    let snapshot = models.runtime.snapshot().await;
    assert_eq!(snapshot.loaded_models, 2);
    assert_eq!(snapshot.resident_models, 1);
    drop((embed, transient));
}

/// A stand-in ASR worker that speaks the protocol and records its pid.
#[cfg(unix)]
fn script_worker(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let program = dir.join("asr-worker.sh");
    std::fs::write(
        &program,
        r#"#!/bin/sh
echo $$ >> "$(dirname "$0")/spawns"
echo '{"ready":{"accel":"metal","load_ms":1}}'
while read header; do
  echo '{"decoded":{"words":[],"language":null,"accel":"coreml","decode_ms":1}}'
done
"#,
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    program
}

#[cfg(unix)]
fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(unix)]
#[tokio::test]
async fn prewarm_starts_one_resident_worker_that_unload_stops() {
    let dir = tempfile::tempdir().unwrap();
    let pool = nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
        .await
        .unwrap();
    whisper_fixture(dir.path());
    let program = script_worker(dir.path());
    let mut runtime = ModelRuntimeManager::new(Arc::new(pool));
    let id = default_model().id;
    runtime.adapters.insert(
        ModelId::new(id).unwrap(),
        Arc::new(WhisperAdapter {
            worker_program: Some(program),
            estimated_bytes: Some(800_000_000),
        }),
    );
    let cancel = RuntimeCancellation::default();
    runtime
        .prewarm_speech(dir.path(), id, &cancel)
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await;
    assert_eq!((snapshot.loaded_models, snapshot.active_leases), (1, 0));
    assert_eq!(snapshot.estimated_resident_bytes, 800_000_000);

    // The caption session that follows reuses the warm worker.
    let lease = runtime
        .acquire_at(dir.path(), id, WorkClass::Realtime, &cancel)
        .await
        .unwrap();
    let engine = lease
        .clone()
        .run_blocking_at(WorkClass::Realtime, &cancel, |lease| lease.speech_engine())
        .await
        .unwrap()
        .unwrap();
    let spawns = std::fs::read_to_string(dir.path().join("spawns")).unwrap();
    let pids: Vec<&str> = spawns.lines().collect();
    assert_eq!(pids.len(), 1, "prewarm and use share one worker");
    assert!(alive(pids[0]));

    // A session may outlive its lease; unloading still stops the worker.
    drop(lease);
    assert!(runtime.unload(id).await.unwrap());
    assert!(!alive(pids[0]), "unloading the model stops its worker");
    assert!(engine.accel().is_none());
}

#[tokio::test]
async fn speech_engine_is_only_offered_by_whisper_backends() {
    let fixture = Fixture::new();
    let lease = fixture.acquire().await;
    assert!(matches!(lease.speech_engine(), Err(RuntimeError::InvalidModel)));
}
