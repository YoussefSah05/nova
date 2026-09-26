//! Background job processing over the per-profile `jobs` table.
//!
//! Nova's job queue is **kick-driven** rather than a persistent polling loop:
//! callers `enqueue` a job and then `kick()` the queue, which drains all
//! ready jobs of a kind and then idles. A `tokio::Mutex` ensures only one drain
//! runs per [`JobQueue`] at a time, giving a natural concurrency cap of one
//! (transcription is CPU/ANE-heavy — we never want two at once). This fits the
//! app's per-profile pool model without a long-lived worker tied to profile
//! switches; durable `queued` rows are also drained on startup via [`kick`].
//!
//! The actual work for a kind is supplied by a [`JobHandler`] living in the
//! desktop crate (it needs the Tauri event bus, app data dir, and storage repos
//! that this crate must not depend on).

use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nova_storage::{Job, JobRepository};
use sqlx::SqlitePool;
use tokio::sync::Mutex;

/// A boxed, `'static` future returned by a [`JobHandler`].
pub type JobFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// Resource admission obtained by [`JobHandler::ready`] and owned by the job
/// for its whole run. It is dropped when the handler's future finishes (or
/// unwinds), or when no job remained to claim. Handlers downcast it to their concrete permit type.
pub type JobPermit = Box<dyn Any + Send>;

/// Future returned by [`JobHandler::ready`].
pub type ReadyFuture = Pin<Box<dyn Future<Output = Result<JobPermit, String>> + Send>>;

/// Processes one claimed job. `Ok(())` marks it done; `Err(message)` marks it
/// failed and records the message in `jobs.last_error`. Implementors clone any
/// state they need into the returned future so it can be `'static`.
pub trait JobHandler: Send + Sync + 'static {
    /// Wait for shared resource admission before a queued job is claimed, so
    /// a resource-blocked job stays durably `queued`. An `Err` (for example
    /// cancellation) stops this drain without claiming anything.
    fn ready(&self) -> ReadyFuture {
        Box::pin(async { Ok(Box::new(()) as JobPermit) })
    }

    /// Run one claimed job while owning `permit`. Long-running handlers yield
    /// at their own bounded work boundaries.
    fn handle(&self, job: Job, permit: JobPermit) -> JobFuture;
}

/// A drain-on-demand queue for a single job `kind` against one profile pool.
pub struct JobQueue {
    pool: Arc<SqlitePool>,
    kind: String,
    handler: Arc<dyn JobHandler>,
    /// Held for the duration of a drain so at most one runs at a time.
    drain_lock: Arc<Mutex<()>>,
    drain_running: AtomicBool,
    kick_requested: AtomicBool,
}

impl JobQueue {
    pub fn new(
        pool: Arc<SqlitePool>,
        kind: impl Into<String>,
        handler: Arc<dyn JobHandler>,
    ) -> Self {
        Self {
            pool,
            kind: kind.into(),
            handler,
            drain_lock: Arc::new(Mutex::new(())),
            drain_running: AtomicBool::new(false),
            kick_requested: AtomicBool::new(false),
        }
    }

    /// Enqueue a job of this queue's kind and return its id. Does **not** start
    /// processing — call [`kick`](Self::kick) after.
    pub async fn enqueue(&self, payload: &str, priority: i64) -> Result<i64, String> {
        JobRepository::new(&self.pool)
            .enqueue(&self.kind, payload, priority)
            .await
            .map_err(|e| e.to_string())
    }

    /// Enqueue unless the same payload is already queued. Returns whether a
    /// new job was inserted; the queue depth stays bounded by distinct work.
    pub async fn enqueue_unique(&self, payload: &str, priority: i64) -> Result<bool, String> {
        JobRepository::new(&self.pool)
            .enqueue_unique(&self.kind, payload, priority)
            .await
            .map(|(_, inserted)| inserted)
            .map_err(|e| e.to_string())
    }

    /// Requeue jobs an interrupted process left `running`. Call once, before
    /// the first [`kick`](Self::kick) of this queue in the process.
    pub async fn recover_interrupted(&self) -> Result<u64, String> {
        JobRepository::new(&self.pool)
            .requeue_running(&self.kind)
            .await
            .map_err(|e| e.to_string())
    }

    /// Remove a not-yet-started job. Returns how many rows were removed.
    pub async fn cancel_queued(&self, payload: &str) -> Result<u64, String> {
        JobRepository::new(&self.pool)
            .delete_queued(&self.kind, payload)
            .await
            .map_err(|e| e.to_string())
    }

    /// Spawn at most one background drain. Concurrent kicks coalesce without
    /// losing a job inserted after the worker's final empty claim.
    ///
    /// A kick during an active drain records one follow-up pass. This includes
    /// jobs inserted after that drain's final empty claim.
    pub fn kick(self: &Arc<Self>) {
        self.kick_requested.store(true, Ordering::Release);
        if self
            .drain_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let _guard = me.drain_lock.lock().await;
            loop {
                me.kick_requested.store(false, Ordering::Release);
                let repo = JobRepository::new(&me.pool);
                loop {
                    match repo.has_queued(&me.kind).await {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(e) => {
                            tracing::error!(error = %e, kind = %me.kind, "checking queued jobs failed");
                            break;
                        }
                    }
                    let handler = Arc::clone(&me.handler);
                    let permit = match tokio::spawn(async move { handler.ready().await }).await {
                        Ok(Ok(permit)) => permit,
                        Ok(Err(msg)) => {
                            tracing::warn!(kind = %me.kind, error = %msg, "job admission stopped");
                            break;
                        }
                        Err(_) => {
                            tracing::warn!(kind = %me.kind, "job admission task stopped unexpectedly");
                            break;
                        }
                    };
                    // A job removed while waiting leaves nothing to claim; the
                    // permit is then released without running work.
                    match repo.claim_next(&me.kind).await {
                        Ok(Some(job)) => {
                            let id = job.id;
                            let handler = Arc::clone(&me.handler);
                            let result = match tokio::spawn(async move {
                                handler.handle(job, permit).await
                            })
                            .await
                            {
                                Ok(result) => result,
                                Err(_) => Err("job worker stopped unexpectedly".to_string()),
                            };
                            let outcome = match &result {
                                Ok(()) => repo.mark_done(id).await,
                                Err(msg) => repo.mark_failed(id, msg).await,
                            };
                            if let Err(e) = outcome {
                                tracing::error!(job_id = id, error = %e, "failed to record job outcome");
                            }
                            if let Err(msg) = result {
                                tracing::warn!(job_id = id, error = %msg, kind = %me.kind, "job failed");
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            tracing::error!(error = %e, kind = %me.kind, "claim_next failed; stopping drain");
                            break;
                        }
                    }
                }
                me.drain_running.store(false, Ordering::Release);
                if !me.kick_requested.swap(false, Ordering::AcqRel)
                    || me
                        .drain_running
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                {
                    break;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nova_storage::init_profile_db;

    struct CountingHandler {
        seen: Arc<AtomicUsize>,
        fail_payload: &'static str,
    }

    impl JobHandler for CountingHandler {
        fn handle(&self, job: Job, _permit: JobPermit) -> JobFuture {
            let seen = Arc::clone(&self.seen);
            let fail = self.fail_payload;
            Box::pin(async move {
                seen.fetch_add(1, Ordering::SeqCst);
                if job.payload == fail {
                    Err("boom".to_string())
                } else {
                    Ok(())
                }
            })
        }
    }

    /// Block until the queue has no queued/running jobs left, or panic on timeout.
    async fn wait_drained(pool: &SqlitePool) {
        for _ in 0..200 {
            let repo = JobRepository::new(pool);
            let queued = repo.list_by_state("queued").await.unwrap().len();
            let running = repo.list_by_state("running").await.unwrap().len();
            if queued == 0 && running == 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("queue did not drain in time");
    }

    #[tokio::test]
    async fn kick_during_drain_shutdown_is_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(
            init_profile_db(dir.path().join("jobs.db").to_str().unwrap())
                .await
                .unwrap(),
        );
        let seen = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(JobQueue::new(
            pool.clone(),
            "test",
            Arc::new(CountingHandler {
                seen: seen.clone(),
                fail_payload: "fail",
            }),
        ));
        // Model a drain that has observed an empty queue but still owns its lock.
        let guard = queue.drain_lock.lock().await;
        queue.enqueue("late arrival", 0).await.unwrap();
        queue.kick();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        drop(guard);
        wait_drained(&pool).await;
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn drains_jobs_and_records_outcomes() {
        // A real file DB, not `sqlite::memory:` — pooled connections must share
        // one database, and in-memory SQLite gives each connection its own.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("jobs.db");
        let pool = init_profile_db(db_path.to_str().unwrap()).await.unwrap();
        let pool = Arc::new(pool);
        let seen = Arc::new(AtomicUsize::new(0));
        let handler = Arc::new(CountingHandler {
            seen: Arc::clone(&seen),
            fail_payload: "fail-me",
        });
        let queue = Arc::new(JobQueue::new(Arc::clone(&pool), "transcribe_vod", handler));

        queue.enqueue("ok-1", 0).await.unwrap();
        queue.enqueue("fail-me", 0).await.unwrap();
        queue.enqueue("ok-2", 0).await.unwrap();
        queue.kick();

        wait_drained(&pool).await;
        assert_eq!(seen.load(Ordering::SeqCst), 3);

        let repo = JobRepository::new(&pool);
        assert_eq!(repo.list_by_state("done").await.unwrap().len(), 2);
        assert_eq!(repo.list_by_state("failed").await.unwrap().len(), 1);
    }

    struct OrderingHandler {
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl JobHandler for OrderingHandler {
        fn handle(&self, job: Job, _permit: JobPermit) -> JobFuture {
            let seen = Arc::clone(&self.seen);
            Box::pin(async move {
                seen.lock().await.push(job.payload);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn persisted_jobs_use_priority_then_stable_enqueue_order() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(
            init_profile_db(dir.path().join("jobs.db").to_str().unwrap())
                .await
                .unwrap(),
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let queue = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            "ordered",
            Arc::new(OrderingHandler {
                seen: Arc::clone(&seen),
            }),
        ));
        queue.enqueue("first", 300).await.unwrap();
        queue.enqueue("higher", 400).await.unwrap();
        queue.enqueue("second", 300).await.unwrap();
        for _ in 0..100 {
            queue.kick();
        }
        wait_drained(&pool).await;
        assert_eq!(seen.lock().await.as_slice(), ["higher", "first", "second"]);
    }

    struct PanicThenContinue {
        seen: Arc<AtomicUsize>,
    }

    impl JobHandler for PanicThenContinue {
        fn handle(&self, job: Job, _permit: JobPermit) -> JobFuture {
            let seen = Arc::clone(&self.seen);
            Box::pin(async move {
                if job.payload == "panic" {
                    panic!("synthetic worker failure");
                }
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn panicked_handler_fails_its_job_without_stopping_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(
            init_profile_db(dir.path().join("jobs.db").to_str().unwrap())
                .await
                .unwrap(),
        );
        let seen = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            "panic_test",
            Arc::new(PanicThenContinue {
                seen: Arc::clone(&seen),
            }),
        ));
        queue.enqueue("panic", 400).await.unwrap();
        queue.enqueue("after", 400).await.unwrap();
        queue.kick();
        wait_drained(&pool).await;
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        let repo = JobRepository::new(&pool);
        assert_eq!(repo.list_by_state("failed").await.unwrap().len(), 1);
        assert_eq!(repo.list_by_state("done").await.unwrap().len(), 1);
    }

    async fn temp_pool(dir: &tempfile::TempDir) -> Arc<SqlitePool> {
        Arc::new(
            init_profile_db(dir.path().join("jobs.db").to_str().unwrap())
                .await
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn duplicate_requests_coalesce_into_one_queued_job() {
        let dir = tempfile::tempdir().unwrap();
        let pool = temp_pool(&dir).await;
        let seen = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            "dedup",
            Arc::new(CountingHandler {
                seen: Arc::clone(&seen),
                fail_payload: "",
            }),
        ));
        assert!(queue.enqueue_unique("source-1", 400).await.unwrap());
        for _ in 0..50 {
            assert!(!queue.enqueue_unique("source-1", 400).await.unwrap());
        }
        assert!(queue.enqueue_unique("source-2", 400).await.unwrap());
        let repo = JobRepository::new(&pool);
        assert_eq!(repo.list_by_state("queued").await.unwrap().len(), 2);
        queue.kick();
        wait_drained(&pool).await;
        assert_eq!(seen.load(Ordering::SeqCst), 2);
        // Once the first finished, a new request is accepted again.
        assert!(queue.enqueue_unique("source-1", 400).await.unwrap());
    }

    #[tokio::test]
    async fn interrupted_running_jobs_are_requeued_and_cancelled_ones_removed() {
        let dir = tempfile::tempdir().unwrap();
        let pool = temp_pool(&dir).await;
        let repo = JobRepository::new(&pool);
        repo.enqueue("recover", "stale", 400).await.unwrap();
        repo.enqueue("recover", "removed", 400).await.unwrap();
        repo.enqueue("other", "untouched", 400).await.unwrap();
        // Simulate a crash after claiming: the row is left `running`.
        assert_eq!(
            repo.claim_next("recover").await.unwrap().unwrap().payload,
            "stale"
        );
        repo.claim_next("other").await.unwrap().unwrap();

        let seen = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            "recover",
            Arc::new(CountingHandler {
                seen: Arc::clone(&seen),
                fail_payload: "",
            }),
        ));
        assert_eq!(queue.cancel_queued("removed").await.unwrap(), 1);
        assert_eq!(queue.recover_interrupted().await.unwrap(), 1);
        queue.kick();
        for _ in 0..200 {
            if repo.list_by_state("done").await.unwrap().len() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        let running = repo.list_by_state("running").await.unwrap();
        assert_eq!(running.len(), 1, "another kind's job is not recovered");
        assert_eq!(running[0].kind, "other");
    }

    struct PermitDrop(Arc<AtomicUsize>);
    impl Drop for PermitDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct PermitHandler {
        released: Arc<AtomicUsize>,
        observed_while_running: Arc<AtomicUsize>,
    }

    impl JobHandler for PermitHandler {
        fn ready(&self) -> ReadyFuture {
            let released = Arc::clone(&self.released);
            Box::pin(async move { Ok(Box::new(PermitDrop(released)) as JobPermit) })
        }

        fn handle(&self, _job: Job, permit: JobPermit) -> JobFuture {
            let released = Arc::clone(&self.released);
            let observed = Arc::clone(&self.observed_while_running);
            Box::pin(async move {
                assert!(permit.downcast_ref::<PermitDrop>().is_some());
                tokio::task::yield_now().await;
                observed.store(released.load(Ordering::SeqCst), Ordering::SeqCst);
                drop(permit);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn admission_permit_is_owned_for_the_whole_job() {
        let dir = tempfile::tempdir().unwrap();
        let pool = temp_pool(&dir).await;
        let released = Arc::new(AtomicUsize::new(0));
        let observed = Arc::new(AtomicUsize::new(usize::MAX));
        let queue = Arc::new(JobQueue::new(
            Arc::clone(&pool),
            "permit",
            Arc::new(PermitHandler {
                released: Arc::clone(&released),
                observed_while_running: Arc::clone(&observed),
            }),
        ));
        queue.enqueue("one", 400).await.unwrap();
        queue.kick();
        wait_drained(&pool).await;
        assert_eq!(
            observed.load(Ordering::SeqCst),
            0,
            "permit held during work"
        );
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }
}
