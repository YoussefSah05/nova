//! Per-source EPG URL registry.
//!
//! One row per resolved XMLTV feed for a source. A source may have multiple
//! rows — the provider-supplied feed (Xtream `xmltv.php` or M3U `url-tvg`)
//! plus any community fallbacks. The refresh worker iterates over these rows
//! and downloads each in turn.

use nova_core::now_ts;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::StorageError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpgSource {
    pub id: i64,
    pub source_id: i64,
    pub url: String,
    pub kind: String, // 'xtream' | 'm3u_url_tvg' | 'community'
    pub last_refresh_utc: Option<i64>,
    pub last_error: Option<String>,
}

pub struct EpgSourcesRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> EpgSourcesRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert a row if `(source_id, url)` is new, otherwise leave it alone.
    /// The migration has a UNIQUE constraint on (source_id, url) — using
    /// `INSERT OR IGNORE` keeps the call idempotent so callers can register
    /// the same URL on every import without churning the row.
    pub async fn upsert(&self, source_id: i64, url: &str, kind: &str) -> Result<(), StorageError> {
        sqlx::query!(
            "INSERT OR IGNORE INTO epg_sources (source_id, url, kind) VALUES (?, ?, ?)",
            source_id,
            url,
            kind,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_for_source(&self, source_id: i64) -> Result<Vec<EpgSource>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", source_id, url, kind, last_refresh_utc, last_error
               FROM epg_sources WHERE source_id = ? ORDER BY id ASC"#,
            source_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| EpgSource {
                id: r.id,
                source_id: r.source_id,
                url: r.url,
                kind: r.kind,
                last_refresh_utc: r.last_refresh_utc,
                last_error: r.last_error,
            })
            .collect())
    }

    pub async fn touch_success(&self, id: i64) -> Result<(), StorageError> {
        let now = now_ts();
        sqlx::query!(
            "UPDATE epg_sources SET last_refresh_utc = ?, last_error = NULL WHERE id = ?",
            now,
            id,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn touch_failure(&self, id: i64, error: &str) -> Result<(), StorageError> {
        let now = now_ts();
        sqlx::query!(
            "UPDATE epg_sources SET last_refresh_utc = ?, last_error = ? WHERE id = ?",
            now,
            error,
            id,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
