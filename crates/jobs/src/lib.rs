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

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nova_storage::{Job, JobRepository};
use sqlx::SqlitePool;
use tokio::sync::Mutex;

/// A boxed, `'static` future returned by a [`JobHandler`].
pub type JobFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// Processes one claimed job. `Ok(())` marks it done; `Err(message)` marks it
/// failed and records the message in `jobs.last_error`. Implementors clone any
/// state they need into the returned future so it can be `'static`.
pub trait JobHandler: Send + Sync + 'static {
    fn handle(&self, job: Job) -> JobFuture;
}

/// A drain-on-demand queue for a single job `kind` against one profile pool.
pub struct JobQueue {
    pool: Arc<SqlitePool>,
    kind: String,
    handler: Arc<dyn JobHandler>,
    /// Held for the duration of a drain so at most one runs at a time.
    drain_lock: Arc<Mutex<()>>,
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

    /// Spawn a background drain if one isn't already running. Safe to call
    /// liberally (after every enqueue, and once at startup).
    ///
    /// Known micro-race: a job enqueued in the instant between the running
    /// drain's final empty `claim_next` and its lock release may wait for the
    /// next `kick`. Jobs are durable, so this self-heals on the next enqueue or
    /// app start; acceptable for a local single-user queue.
    pub fn kick(self: &Arc<Self>) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            // If another drain holds the lock it will observe our row (it was
            // inserted before this kick), so we can simply bow out.
            let _guard = match me.drain_lock.try_lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            let repo = JobRepository::new(&me.pool);
            loop {
                match repo.claim_next(&me.kind).await {
                    Ok(Some(job)) => {
                        let id = job.id;
                        let result = me.handler.handle(job).await;
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
        fn handle(&self, job: Job) -> JobFuture {
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
}
