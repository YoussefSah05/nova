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
                   ORDER BY priority DESC, created_at ASC
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
             ORDER BY priority DESC, created_at ASC"#,
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
