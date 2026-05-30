use nova_core::{now_ts, Source};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;

pub struct SourceRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SourceRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, kind: &str, name: &str, url: &str) -> Result<Source, StorageError> {
        let row = sqlx::query!(
            "INSERT INTO sources (kind, name, url) VALUES (?, ?, ?) RETURNING id, kind, name, url, last_refreshed_at, auto_refresh_hours",
            kind,
            name,
            url,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(Source {
            id: row.id,
            kind: row.kind,
            name: row.name,
            url: row.url,
            last_refreshed_at: row.last_refreshed_at,
            auto_refresh_hours: row.auto_refresh_hours,
        })
    }

    pub async fn get(&self, id: i64) -> Result<Option<Source>, StorageError> {
        let row = sqlx::query!(
            "SELECT id, kind, name, url, last_refreshed_at, auto_refresh_hours FROM sources WHERE id = ?",
            id,
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| Source {
            id: r.id,
            kind: r.kind,
            name: r.name,
            url: r.url,
            last_refreshed_at: r.last_refreshed_at,
            auto_refresh_hours: r.auto_refresh_hours,
        }))
    }

    pub async fn list(&self) -> Result<Vec<Source>, StorageError> {
        let rows = sqlx::query!(
            "SELECT id, kind, name, url, last_refreshed_at, auto_refresh_hours FROM sources ORDER BY id ASC"
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Source {
                id: r.id,
                kind: r.kind,
                name: r.name,
                url: r.url,
                last_refreshed_at: r.last_refreshed_at,
                auto_refresh_hours: r.auto_refresh_hours,
            })
            .collect())
    }

    pub async fn touch_refreshed(&self, id: i64) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        Self::touch_refreshed_tx(&mut tx, id).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn touch_refreshed_tx(
        tx: &mut Transaction<'_, Sqlite>,
        id: i64,
    ) -> Result<(), StorageError> {
        let now = now_ts();
        sqlx::query!(
            "UPDATE sources SET last_refreshed_at = ? WHERE id = ?",
            now,
            id
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}
