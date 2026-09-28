//! Response cache for the opt-in TMDB enrichment pass (Phase C). Stores raw JSON
//! bodies keyed by a caller-built `cache_key` so repeat searches don't re-hit the
//! API (TMDB ToS permits caching). Bodies are parsed by `nova_enrich::tmdb`.

use nova_core::now_ts;
use sqlx::SqlitePool;

use crate::error::StorageError;

pub struct TmdbCacheRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> TmdbCacheRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Cached raw body for `key`, if present.
    pub async fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        let row = sqlx::query!(
            r#"SELECT body as "body!: String" FROM tmdb_cache WHERE cache_key = ?"#,
            key,
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| r.body))
    }

    /// Cached raw body for `key` fetched at or after `not_before` (epoch
    /// seconds); older bodies count as missing so the caller refetches.
    pub async fn get_since(
        &self,
        key: &str,
        not_before: i64,
    ) -> Result<Option<String>, StorageError> {
        let row = sqlx::query!(
            r#"SELECT body as "body!: String" FROM tmdb_cache
               WHERE cache_key = ? AND fetched_at >= ?"#,
            key,
            not_before,
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| r.body))
    }

    /// Store (or replace) the raw body for `key`.
    pub async fn put(&self, key: &str, body: &str) -> Result<(), StorageError> {
        let fetched_at = now_ts();
        sqlx::query!(
            "INSERT INTO tmdb_cache (cache_key, body, fetched_at) VALUES (?, ?, ?)
             ON CONFLICT(cache_key) DO UPDATE SET body = excluded.body, fetched_at = excluded.fetched_at",
            key,
            body,
            fetched_at,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
