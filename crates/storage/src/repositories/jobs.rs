use nova_core::now_ts;
use sqlx::SqlitePool;

use crate::error::StorageError;

pub struct JobRepository<'a> {
    pool: &'a SqlitePool,
}

#[derive(Debug)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    pub payload: String,
    pub state: String,
    pub priority: i64,
    pub created_at: i64,
}

impl<'a> JobRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn enqueue(
        &self,
        kind: &str,
        payload: &str,
        priority: i64,
    ) -> Result<i64, StorageError> {
        let now = now_ts();
        let row = sqlx::query!(
            "INSERT INTO jobs (kind, payload, state, priority, created_at) VALUES (?, ?, 'queued', ?, ?) RETURNING id",
            kind,
            payload,
            priority,
            now,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(row.id)
    }

    /// Atomically claim the highest-priority queued job of `kind`: flip it to
    /// `running`, stamp `started_at`, bump `attempts`, and return it. `None`
    /// when the queue has no such job. The `UPDATE ... WHERE id = (SELECT ...)`
    /// form makes the claim race-safe across workers.
    pub async fn claim_next(&self, kind: &str) -> Result<Option<Job>, StorageError> {
        let now = now_ts();
        let row = sqlx::query!(
            r#"UPDATE jobs
                 SET state = 'running', started_at = ?, attempts = attempts + 1
               WHERE id = (
                   SELECT id FROM jobs
                   WHERE state = 'queued' AND kind = ?
                   ORDER BY priority DESC, created_at ASC, id ASC
                   LIMIT 1
               )
               RETURNING
                 id         as "id!: i64",
                 kind,
                 payload,
                 state,
                 priority   as "priority!: i64",
                 created_at as "created_at!: i64""#,
            now,
            kind,
        )
        .fetch_optional(self.pool)
        .await?;

        Ok(row.map(|r| Job {
            id: r.id,
            kind: r.kind,
            payload: r.payload,
            state: r.state,
            priority: r.priority,
            created_at: r.created_at,
        }))
    }

    /// A resource-blocked job remains durably queued until admission succeeds.
    pub async fn has_queued(&self, kind: &str) -> Result<bool, StorageError> {
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM jobs WHERE state = 'queued' AND kind = ?",
            kind,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(count > 0)
    }

    /// Whether a job of `kind` for `payload` is queued or running.
    pub async fn has_active(&self, kind: &str, payload: &str) -> Result<bool, StorageError> {
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM jobs
             WHERE state IN ('queued', 'running') AND kind = ? AND payload = ?",
            kind,
            payload,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(count > 0)
    }

    /// Enqueue unless an identical job is already queued (bounded backpressure:
    /// repeated requests for the same work coalesce). Returns the queued job's
    /// id and whether a new row was inserted. A running job does not suppress
    /// a follow-up request, which may observe newer state.
    pub async fn enqueue_unique(
        &self,
        kind: &str,
        payload: &str,
        priority: i64,
    ) -> Result<(i64, bool), StorageError> {
        let now = now_ts();
        let inserted = sqlx::query_scalar!(
            r#"INSERT INTO jobs (kind, payload, state, priority, created_at)
               SELECT ?, ?, 'queued', ?, ?
               WHERE NOT EXISTS (
                   SELECT 1 FROM jobs WHERE state = 'queued' AND kind = ? AND payload = ?
               )
               RETURNING id as "id!: i64""#,
            kind,
            payload,
            priority,
            now,
            kind,
            payload,
        )
        .fetch_optional(self.pool)
        .await?;
        if let Some(id) = inserted {
            return Ok((id, true));
        }
        let existing = sqlx::query_scalar!(
            r#"SELECT id as "id!: i64" FROM jobs
               WHERE state = 'queued' AND kind = ? AND payload = ?
               ORDER BY id ASC LIMIT 1"#,
            kind,
            payload,
        )
        .fetch_one(self.pool)
        .await?;
        Ok((existing, false))
    }

    /// Return jobs left `running` by an interrupted process to the queue.
    /// Call only before any worker for `kind` starts in this process.
    pub async fn requeue_running(&self, kind: &str) -> Result<u64, StorageError> {
        let result = sqlx::query!(
            "UPDATE jobs SET state = 'queued', started_at = NULL WHERE state = 'running' AND kind = ?",
            kind,
        )
        .execute(self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Remove not-yet-started jobs; running jobs are cancelled by their owner.
    pub async fn delete_queued(&self, kind: &str, payload: &str) -> Result<u64, StorageError> {
        let result = sqlx::query!(
            "DELETE FROM jobs WHERE state = 'queued' AND kind = ? AND payload = ?",
            kind,
            payload,
        )
        .execute(self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Mark a running job done.
    pub async fn mark_done(&self, id: i64) -> Result<(), StorageError> {
        let now = now_ts();
        sqlx::query!(
            "UPDATE jobs SET state = 'done', finished_at = ?, last_error = NULL WHERE id = ?",
            now,
            id,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Mark a running job failed, recording the error for diagnostics.
    pub async fn mark_failed(&self, id: i64, error: &str) -> Result<(), StorageError> {
        let now = now_ts();
        sqlx::query!(
            "UPDATE jobs SET state = 'failed', finished_at = ?, last_error = ? WHERE id = ?",
            now,
            error,
            id,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_by_state(&self, state: &str) -> Result<Vec<Job>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", kind, payload, state, priority, created_at
             FROM jobs
             WHERE state = ?
             ORDER BY priority DESC, created_at ASC, id ASC"#,
            state,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Job {
                id: r.id,
                kind: r.kind,
                payload: r.payload,
                state: r.state,
                priority: r.priority,
                created_at: r.created_at,
            })
            .collect())
    }
}
