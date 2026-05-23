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
