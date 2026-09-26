use super::*;
use crate::resource_policy::WorkClass;
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
}
impl BackendAdapter for FakeAdapter {
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

fn nllb_fixture(root: &Path) {
    let dir = root.join("models/nllb");
    std::fs::create_dir_all(&dir).unwrap();
    for path in nova_translate::model::required_artifact_paths(&dir) {
        std::fs::write(path, b"fixture").unwrap();
    }
    std::fs::write(dir.join("config.json"), b"{}").unwrap();
}

#[tokio::test]
async fn registry_resolution_is_lazy_and_rechecks_missing_locations() {
    let dir = tempfile::tempdir().unwrap();
    let pool = nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
        .await
        .unwrap();
    let adapter = Arc::new(FakeAdapter::default());
    let mut runtime = ModelRuntimeManager::new(Arc::new(pool));
    let id = nova_translate::model::MODEL_ID;
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
    nllb_fixture(dir.path());
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
    nllb_fixture(dir.path());
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
    nllb_fixture(&external);
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
    let id = nova_translate::model::MODEL_ID;
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
    nllb_fixture(&external);
    drop(lease);
    runtime.unload(id).await.unwrap();
    let restored = runtime
        .acquire(dir.path(), id, &RuntimeCancellation::default())
        .await
        .unwrap();
    assert_eq!(restored.loaded.library_id.as_str(), "external-fixture");
    assert_eq!(adapter.loads.load(Ordering::SeqCst), 2);
}
