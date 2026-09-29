//! Persisted, per-profile desktop work governed by the shared resource
//! governor. Payloads contain IDs only; provider URLs and credentials stay in
//! the source repository.
//!
//! Each queue drains one job at a time; every running job additionally holds a
//! [`JobSlot`] from the process-wide job lane, so concurrency is bounded across
//! kinds and profiles. A job waits for its slot while still `queued`, owns the
//! slot for its whole run, and long-running handlers yield it at cooperative
//! checkpoints.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nova_core::{ImportProgress, ImportStage};
use nova_jobs::{JobFuture, JobHandler, JobPermit, JobQueue, ReadyFuture};
use nova_storage::{Job, SourceRepository};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::commands::{enrich, epg, run_source_import, ImportMode};
use crate::model_runtime::{ModelRuntimeManager, RuntimeCancellation};
use crate::resource_policy::{AdmissionController, JobSlot, WorkClass};

const EXPLICIT_PRIORITY: i64 = 400;
const METADATA_PRIORITY: i64 = 300;

const IMPORT_KIND: &str = "desktop_import";
const EPG_KIND: &str = "desktop_epg_refresh";
const ENRICH_KIND: &str = "desktop_enrichment";

fn emit_import_stage(app: &AppHandle, source_id: i64, stage: ImportStage) {
    let _ = app.emit(
        "playlist:import:progress",
        ImportProgress {
            source_id,
            stage,
            parsed: 0,
            inserted: 0,
            live_count: 0,
            movie_count: 0,
            series_count: 0,
            failed_count: 0,
            skipped_count: 0,
            elapsed_ms: 0,
            done: false,
            error: None,
            warning: None,
        },
    );
}

/// Cancellation handles of running jobs, keyed by (kind, source).
#[derive(Default)]
struct JobControl {
    running: Mutex<HashMap<(&'static str, i64), RuntimeCancellation>>,
}

/// Registered for the duration of one handler run; removed on drop.
struct RunningJob {
    control: Arc<JobControl>,
    key: (&'static str, i64),
    cancellation: RuntimeCancellation,
}

impl Drop for RunningJob {
    fn drop(&mut self) {
        lock(&self.control.running).remove(&self.key);
    }
}

impl JobControl {
    fn start(self: &Arc<Self>, kind: &'static str, source_id: i64) -> RunningJob {
        let cancellation = RuntimeCancellation::default();
        lock(&self.running).insert((kind, source_id), cancellation.clone());
        RunningJob {
            control: Arc::clone(self),
            key: (kind, source_id),
            cancellation,
        }
    }

    fn cancel_source(&self, source_id: i64) -> usize {
        let running = lock(&self.running);
        let mut cancelled = 0;
        for ((_, id), cancellation) in running.iter() {
            if *id == source_id {
                cancellation.cancel();
                cancelled += 1;
            }
        }
        cancelled
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn take_slot(permit: JobPermit) -> Result<JobSlot, String> {
    permit
        .downcast::<JobSlot>()
        .map(|slot| *slot)
        .map_err(|_| "job started without a resource slot".to_string())
}

fn job_ready(admission: &Arc<AdmissionController>, class: WorkClass) -> ReadyFuture {
    let admission = Arc::clone(admission);
    Box::pin(async move {
        // Held by the job for its whole run; released on completion, failure
        // or panic unwind. Queued jobs stay durable while they wait.
        let slot = admission
            .acquire_job_slot(class, &RuntimeCancellation::default())
            .await
            .map_err(|_| "job admission cancelled".to_string())?;
        Ok(Box::new(slot) as JobPermit)
    })
}

pub struct DesktopJobs {
    app: AppHandle,
    imports: Arc<JobQueue>,
    epg: Arc<JobQueue>,
    enrichment: Arc<JobQueue>,
    control: Arc<JobControl>,
}

impl DesktopJobs {
    pub fn new(pool: Arc<SqlitePool>, app: AppHandle, runtime: Arc<ModelRuntimeManager>) -> Self {
        let admission = runtime.resource_admission();
        let control = Arc::new(JobControl::default());
        let epg = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            EPG_KIND,
            Arc::new(EpgHandler {
                pool: Arc::clone(&pool),
                app: app.clone(),
                admission: Arc::clone(&admission),
                control: Arc::clone(&control),
            }),
        ));
        let imports = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            IMPORT_KIND,
            Arc::new(ImportHandler {
                pool: Arc::clone(&pool),
                app: app.clone(),
                admission: Arc::clone(&admission),
                epg: Arc::clone(&epg),
            }),
        ));
        let enrichment = Arc::new(JobQueue::new(
            pool.clone(),
            ENRICH_KIND,
            Arc::new(EnrichmentHandler {
                pool,
                app: app.clone(),
                runtime,
                admission,
                control: Arc::clone(&control),
            }),
        ));
        Self {
            app: app.clone(),
            imports,
            epg,
            enrichment,
            control,
        }
    }

    /// Requeue work an interrupted process left `running`. Called once when
    /// the profile's workers are created, before any queue is kicked. Append
    /// imports stay idempotent through the source's commit stamp.
    pub async fn recover_interrupted(&self) {
        for queue in [&self.imports, &self.epg, &self.enrichment] {
            if let Err(error) = queue.recover_interrupted().await {
                tracing::warn!(%error, "could not requeue interrupted jobs");
            }
        }
    }

    pub fn kick_all(&self) {
        self.imports.kick();
        self.epg.kick();
        self.enrichment.kick();
    }

    pub(crate) async fn import(&self, source_id: i64, mode: ImportMode) -> Result<(), String> {
        let mode = match mode {
            ImportMode::Append => "append",
            ImportMode::Replace => "replace",
        };
        self.imports
            .enqueue_unique(&format!("{source_id}:{mode}"), EXPLICIT_PRIORITY)
            .await?;
        emit_import_stage(&self.app, source_id, ImportStage::Queued);
        self.imports.kick();
        Ok(())
    }

    pub async fn epg_refresh(&self, source_id: i64) -> Result<(), String> {
        self.epg
            .enqueue_unique(&source_id.to_string(), EXPLICIT_PRIORITY)
            .await?;
        self.epg.kick();
        Ok(())
    }

    pub async fn enrich(&self, source_id: i64) -> Result<(), String> {
        self.enrichment
            .enqueue_unique(&source_id.to_string(), METADATA_PRIORITY)
            .await?;
        let _ = self.app.emit(
            "catalog:enrich:progress",
            enrich::EnrichProgress {
                source_id,
                stage: "queued".into(),
                processed: 0,
                total: 0,
                done: false,
                error: None,
            },
        );
        self.enrichment.kick();
        Ok(())
    }

    /// Drop a source's queued work and cancel its running EPG/enrichment at
    /// their next checkpoint. A running import is not interrupted: it holds
    /// one atomic catalog transaction that commits or rolls back as a unit.
    pub async fn cancel_source(&self, source_id: i64) {
        let id = source_id.to_string();
        let removals = [
            (&self.imports, format!("{id}:append")),
            (&self.imports, format!("{id}:replace")),
            (&self.epg, id.clone()),
            (&self.enrichment, id),
        ];
        for (queue, payload) in removals {
            if let Err(error) = queue.cancel_queued(&payload).await {
                tracing::warn!(%error, source_id, "could not remove queued job");
            }
        }
        self.control.cancel_source(source_id);
    }
}

struct ImportHandler {
    pool: Arc<SqlitePool>,
    app: AppHandle,
    admission: Arc<AdmissionController>,
    /// A successful import refreshes the source's guide: its feeds may be
    /// new (an M3U `url-tvg`), and its channels' tvg-ids may have changed.
    epg: Arc<JobQueue>,
}

impl JobHandler for ImportHandler {
    fn ready(&self) -> ReadyFuture {
        job_ready(&self.admission, WorkClass::Explicit)
    }

    fn handle(&self, job: Job, permit: JobPermit) -> JobFuture {
        let pool = Arc::clone(&self.pool);
        let app = self.app.clone();
        let epg = Arc::clone(&self.epg);
        Box::pin(async move {
            let _slot = take_slot(permit)?;
            let (id, mode) = job
                .payload
                .split_once(':')
                .ok_or_else(|| "invalid import job".to_string())?;
            let source_id = id.parse::<i64>().map_err(|_| "invalid import job")?;
            let mode = match mode {
                "append" => ImportMode::Append,
                "replace" => ImportMode::Replace,
                _ => return Err("invalid import job".into()),
            };
            let source = SourceRepository::new(&pool)
                .get(source_id)
                .await
                .map_err(|_| "could not load source")?
                .ok_or_else(|| "source removed".to_string())?;
            // A recovered append may have committed before the previous
            // process could mark its job done. Replaying it would duplicate
            // catalog rows; a committed import stamps the source atomically.
            if mode == ImportMode::Append && source.last_refreshed_at.is_some() {
                return Ok(());
            }
            emit_import_stage(&app, source_id, ImportStage::Connecting);
            if run_source_import(pool, app, source, mode).await {
                match epg
                    .enqueue_unique(&source_id.to_string(), EXPLICIT_PRIORITY)
                    .await
                {
                    Ok(_) => epg.kick(),
                    Err(error) => tracing::warn!(%error, source_id, "could not queue EPG refresh"),
                }
                Ok(())
            } else {
                Err("import failed".into())
            }
        })
    }
}

struct EpgHandler {
    pool: Arc<SqlitePool>,
    app: AppHandle,
    admission: Arc<AdmissionController>,
    control: Arc<JobControl>,
}

impl JobHandler for EpgHandler {
    fn ready(&self) -> ReadyFuture {
        job_ready(&self.admission, WorkClass::Explicit)
    }

    fn handle(&self, job: Job, permit: JobPermit) -> JobFuture {
        let pool = Arc::clone(&self.pool);
        let app = self.app.clone();
        let control = Arc::clone(&self.control);
        Box::pin(async move {
            let _slot = take_slot(permit)?;
            let source_id = job.payload.parse().map_err(|_| "invalid EPG job")?;
            let running = control.start(EPG_KIND, source_id);
            epg::run_epg_refresh_job(source_id, pool, app, &running.cancellation).await
        })
    }
}

struct EnrichmentHandler {
    pool: Arc<SqlitePool>,
    app: AppHandle,
    runtime: Arc<ModelRuntimeManager>,
    admission: Arc<AdmissionController>,
    control: Arc<JobControl>,
}

impl JobHandler for EnrichmentHandler {
    fn ready(&self) -> ReadyFuture {
        job_ready(&self.admission, WorkClass::Metadata)
    }

    fn handle(&self, job: Job, permit: JobPermit) -> JobFuture {
        let pool = Arc::clone(&self.pool);
        let app = self.app.clone();
        let runtime = Arc::clone(&self.runtime);
        let control = Arc::clone(&self.control);
        Box::pin(async move {
            let mut slot = take_slot(permit)?;
            let source_id = job.payload.parse().map_err(|_| "invalid enrichment job")?;
            let running = control.start(ENRICH_KIND, source_id);
            let cancellation = &running.cancellation;
            if enrich::run_enrichment(pool, source_id, runtime, &app, &mut slot, cancellation)
                .await
                .is_err()
            {
                let (stage, error) = if cancellation.is_cancelled() {
                    ("cancelled", "enrichment cancelled")
                } else {
                    ("failed", "enrichment failed")
                };
                let _ = app.emit(
                    "catalog:enrich:progress",
                    enrich::EnrichProgress {
                        source_id,
                        stage: stage.into(),
                        processed: 0,
                        total: 0,
                        done: true,
                        error: Some(error.into()),
                    },
                );
                return Err(error.into());
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::{mpsc, Notify};

    use nova_storage::{init_profile_db, JobRepository};

    use crate::resource_policy::ResourceBudget;

    const DEADLINE: Duration = Duration::from_secs(5);

    #[derive(Clone, Copy)]
    enum Behaviour {
        Finish,
        Panic,
        Fail,
        /// Checkpoint until `release` is notified, recording each unit.
        CheckpointUntilReleased,
    }

    struct ProbeHandler {
        admission: Arc<AdmissionController>,
        class: WorkClass,
        cancellation: RuntimeCancellation,
        behaviour: Behaviour,
        events: mpsc::UnboundedSender<String>,
        handled: Arc<AtomicUsize>,
        release: Arc<Notify>,
    }

    impl ProbeHandler {
        fn new(
            admission: &Arc<AdmissionController>,
            class: WorkClass,
            behaviour: Behaviour,
            events: &mpsc::UnboundedSender<String>,
        ) -> Self {
            Self {
                admission: Arc::clone(admission),
                class,
                cancellation: RuntimeCancellation::default(),
                behaviour,
                events: events.clone(),
                handled: Arc::new(AtomicUsize::new(0)),
                release: Arc::new(Notify::new()),
            }
        }
    }

    impl JobHandler for ProbeHandler {
        fn ready(&self) -> ReadyFuture {
            let admission = Arc::clone(&self.admission);
            let class = self.class;
            let cancellation = self.cancellation.clone();
            Box::pin(async move {
                let slot = admission
                    .acquire_job_slot(class, &cancellation)
                    .await
                    .map_err(|_| "cancelled".to_string())?;
                Ok(Box::new(slot) as JobPermit)
            })
        }

        fn handle(&self, job: Job, permit: JobPermit) -> JobFuture {
            let handled = Arc::clone(&self.handled);
            let events = self.events.clone();
            let behaviour = self.behaviour;
            let release = Arc::clone(&self.release);
            let cancellation = self.cancellation.clone();
            Box::pin(async move {
                let mut slot = take_slot(permit)?;
                let _ = events.send(format!("start:{}", job.payload));
                match behaviour {
                    Behaviour::Finish => {}
                    Behaviour::Panic => panic!("synthetic job failure"),
                    Behaviour::Fail => return Err("synthetic failure".into()),
                    Behaviour::CheckpointUntilReleased => loop {
                        let released = release.notified();
                        tokio::pin!(released);
                        released.as_mut().enable();
                        slot.checkpoint(&cancellation)
                            .await
                            .map_err(|_| "cancelled".to_string())?;
                        let _ = events.send(format!("unit:{}", job.payload));
                        tokio::select! {
                            _ = &mut released => break,
                            _ = tokio::time::sleep(Duration::from_millis(5)) => {}
                        }
                    },
                }
                handled.fetch_add(1, Ordering::SeqCst);
                let _ = events.send(format!("done:{}", job.payload));
                Ok(())
            })
        }
    }

    async fn profile_pool(dir: &tempfile::TempDir) -> Arc<SqlitePool> {
        Arc::new(
            init_profile_db(dir.path().join("profile.db").to_str().unwrap())
                .await
                .unwrap(),
        )
    }

    async fn until(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(DEADLINE, async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("condition not reached");
    }

    async fn next_event(seen: &mut mpsc::UnboundedReceiver<String>) -> String {
        tokio::time::timeout(DEADLINE, seen.recv())
            .await
            .expect("event expected")
            .unwrap()
    }

    #[tokio::test]
    async fn playback_suspends_queued_metadata_jobs_but_not_explicit_import() {
        let dir = tempfile::tempdir().unwrap();
        let pool = profile_pool(&dir).await;
        let admission = Arc::new(AdmissionController::new(ResourceBudget::LOW_RESOURCE));
        let playback = admission.playback_started();
        let (events, mut seen) = mpsc::unbounded_channel();

        let metadata = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            "metadata",
            Arc::new(ProbeHandler::new(
                &admission,
                WorkClass::Metadata,
                Behaviour::Finish,
                &events,
            )),
        ));
        metadata
            .enqueue_unique("enrich", METADATA_PRIORITY)
            .await
            .unwrap();
        metadata.kick();
        until(|| admission.snapshot().suspended_work == 1).await;

        let explicit = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            "explicit",
            Arc::new(ProbeHandler::new(
                &admission,
                WorkClass::Explicit,
                Behaviour::Finish,
                &events,
            )),
        ));
        explicit
            .enqueue_unique("import", EXPLICIT_PRIORITY)
            .await
            .unwrap();
        explicit.kick();
        assert_eq!(next_event(&mut seen).await, "start:import");
        assert_eq!(next_event(&mut seen).await, "done:import");

        // The suspended job stays durably queued, not claimed.
        let repo = JobRepository::new(&pool);
        let queued = repo.list_by_state("queued").await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].payload, "enrich");

        drop(playback);
        assert_eq!(next_event(&mut seen).await, "start:enrich");
        assert_eq!(next_event(&mut seen).await, "done:enrich");
        until(|| admission.snapshot().active_jobs == 0).await;
    }

    #[tokio::test]
    async fn running_job_yields_its_slot_during_playback_and_resumes_afterwards() {
        let dir = tempfile::tempdir().unwrap();
        let pool = profile_pool(&dir).await;
        let admission = Arc::new(AdmissionController::new(ResourceBudget::DEFAULT));
        let (events, mut seen) = mpsc::unbounded_channel();
        let handler = Arc::new(ProbeHandler::new(
            &admission,
            WorkClass::Background,
            Behaviour::CheckpointUntilReleased,
            &events,
        ));
        let release = Arc::clone(&handler.release);
        let queue = Arc::new(JobQueue::new(Arc::clone(&pool), "indexing", handler));
        queue.enqueue_unique("index", 100).await.unwrap();
        queue.kick();
        assert_eq!(next_event(&mut seen).await, "start:index");
        assert_eq!(next_event(&mut seen).await, "unit:index");

        let playback = admission.playback_started();
        until(|| {
            let snapshot = admission.snapshot();
            snapshot.active_jobs == 0 && snapshot.suspended_work == 1
        })
        .await;
        // Drain units produced before the checkpoint observed playback.
        while seen.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(seen.try_recv().is_err(), "no work while suspended");

        drop(playback);
        assert_eq!(next_event(&mut seen).await, "unit:index");
        release.notify_waiters();
        loop {
            if next_event(&mut seen).await == "done:index" {
                break;
            }
            release.notify_waiters();
        }
        until(|| admission.snapshot().active_jobs == 0).await;
        let repo = JobRepository::new(&pool);
        tokio::time::timeout(DEADLINE, async {
            while repo.list_by_state("done").await.unwrap().len() != 1 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn job_lane_is_bounded_and_higher_priority_jobs_take_freed_slots_first() {
        let dir = tempfile::tempdir().unwrap();
        let pool = profile_pool(&dir).await;
        let admission = Arc::new(AdmissionController::new(ResourceBudget::DEFAULT));
        let (events, mut seen) = mpsc::unbounded_channel();

        // Two long-running metadata jobs fill both slots.
        let mut releases = Vec::new();
        for kind in ["long-a", "long-b"] {
            let handler = Arc::new(ProbeHandler::new(
                &admission,
                WorkClass::Metadata,
                Behaviour::CheckpointUntilReleased,
                &events,
            ));
            releases.push(Arc::clone(&handler.release));
            let queue = Arc::new(JobQueue::new(Arc::clone(&pool), kind, handler));
            queue.enqueue_unique(kind, 0).await.unwrap();
            queue.kick();
        }
        until(|| admission.snapshot().active_jobs == 2).await;
        let mut long_starts = 0;
        while long_starts < 2 {
            if next_event(&mut seen).await.starts_with("start:long") {
                long_starts += 1;
            }
        }

        let enqueue = |kind: &'static str, class: WorkClass| {
            let queue = Arc::new(JobQueue::new(
                Arc::clone(&pool),
                kind,
                Arc::new(ProbeHandler::new(
                    &admission,
                    class,
                    Behaviour::Finish,
                    &events,
                )),
            ));
            async move {
                queue.enqueue_unique(kind, 0).await.unwrap();
                queue.kick();
            }
        };
        // An equal-priority waiter never preempts running work.
        enqueue("meta", WorkClass::Metadata).await;
        until(|| admission.snapshot().queued_jobs == 1).await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        while let Ok(event) = seen.try_recv() {
            assert!(!event.starts_with("start:"), "unexpected {event}");
        }
        assert_eq!(admission.snapshot().active_jobs, 2, "never above the bound");

        // A higher-priority waiter takes a slot at the next checkpoint; the
        // preempted job re-queues behind the earlier metadata waiter.
        enqueue("explicit", WorkClass::Explicit).await;
        let mut starts = Vec::new();
        while starts.len() < 2 {
            let event = next_event(&mut seen).await;
            if let Some(kind) = event.strip_prefix("start:") {
                starts.push(kind.to_string());
            }
            assert!(admission.snapshot().active_jobs <= 2);
        }
        assert_eq!(starts, ["explicit", "meta"]);
        for release in &releases {
            release.notify_waiters();
        }
        tokio::time::timeout(DEADLINE, async {
            loop {
                let done = JobRepository::new(&pool)
                    .list_by_state("done")
                    .await
                    .unwrap()
                    .len();
                if done == 4 {
                    break;
                }
                for release in &releases {
                    release.notify_waiters();
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("no deadlock: every job completes");
        until(|| admission.snapshot().active_jobs == 0).await;
    }

    #[tokio::test]
    async fn failed_or_panicking_jobs_release_their_slots_and_later_jobs_run() {
        let dir = tempfile::tempdir().unwrap();
        let pool = profile_pool(&dir).await;
        let admission = Arc::new(AdmissionController::new(ResourceBudget::LOW_RESOURCE));
        let (events, mut seen) = mpsc::unbounded_channel();
        for (kind, behaviour) in [
            ("panics", Behaviour::Panic),
            ("fails", Behaviour::Fail),
            ("ok", Behaviour::Finish),
        ] {
            let queue = Arc::new(JobQueue::new(
                Arc::clone(&pool),
                kind,
                Arc::new(ProbeHandler::new(
                    &admission,
                    WorkClass::Background,
                    behaviour,
                    &events,
                )),
            ));
            queue.enqueue_unique(kind, 0).await.unwrap();
            queue.kick();
        }
        loop {
            if next_event(&mut seen).await == "done:ok" {
                break;
            }
        }
        let repo = JobRepository::new(&pool);
        tokio::time::timeout(DEADLINE, async {
            while repo.list_by_state("failed").await.unwrap().len() != 2
                || repo.list_by_state("done").await.unwrap().len() != 1
            {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        until(|| {
            let snapshot = admission.snapshot();
            snapshot.active_jobs == 0 && snapshot.queued_jobs == 0
        })
        .await;
    }

    #[tokio::test]
    async fn cancelling_an_unadmitted_job_keeps_it_queued_for_a_later_worker() {
        let dir = tempfile::tempdir().unwrap();
        let pool = profile_pool(&dir).await;
        let admission = Arc::new(AdmissionController::default());
        let playback = admission.playback_started();
        let (events, mut seen) = mpsc::unbounded_channel();
        let handler = ProbeHandler::new(
            &admission,
            WorkClass::Maintenance,
            Behaviour::Finish,
            &events,
        );
        let cancellation = handler.cancellation.clone();
        let queue = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            "cancelled",
            Arc::new(handler),
        ));
        queue.enqueue_unique("synthetic", 100).await.unwrap();
        queue.kick();
        until(|| admission.snapshot().queued_jobs == 1).await;
        cancellation.cancel();

        until(|| admission.snapshot().queued_jobs == 0).await;
        assert!(seen.try_recv().is_err());
        let repo = JobRepository::new(&pool);
        assert_eq!(repo.list_by_state("queued").await.unwrap().len(), 1);
        assert!(repo.list_by_state("failed").await.unwrap().is_empty());

        let resumed_handler = ProbeHandler::new(
            &admission,
            WorkClass::Maintenance,
            Behaviour::Finish,
            &events,
        );
        let handled = Arc::clone(&resumed_handler.handled);
        let resumed = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            "cancelled",
            Arc::new(resumed_handler),
        ));
        resumed.kick();
        until(|| admission.snapshot().queued_jobs == 1).await;
        drop(playback);
        tokio::time::timeout(DEADLINE, async {
            while repo.list_by_state("done").await.unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(handled.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelling_a_source_stops_its_running_job_at_the_next_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let pool = profile_pool(&dir).await;
        let admission = Arc::new(AdmissionController::default());
        let control = Arc::new(JobControl::default());
        let (events, mut seen) = mpsc::unbounded_channel();
        let mut handler = ProbeHandler::new(
            &admission,
            WorkClass::Metadata,
            Behaviour::CheckpointUntilReleased,
            &events,
        );
        let running = control.start(ENRICH_KIND, 7);
        let other = control.start(ENRICH_KIND, 8);
        handler.cancellation = running.cancellation.clone();
        let queue = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            "enrich",
            Arc::new(handler),
        ));
        queue.enqueue_unique("7", METADATA_PRIORITY).await.unwrap();
        queue.kick();
        assert_eq!(next_event(&mut seen).await, "start:7");

        assert_eq!(control.cancel_source(7), 1);
        assert!(
            !other.cancellation.is_cancelled(),
            "other sources keep running"
        );
        let repo = JobRepository::new(&pool);
        tokio::time::timeout(DEADLINE, async {
            while repo.list_by_state("failed").await.unwrap().len() != 1 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        until(|| admission.snapshot().active_jobs == 0).await;
        drop(running);
        assert_eq!(lock(&control.running).len(), 1);
        drop(other);
        assert!(lock(&control.running).is_empty());
    }
}
