//! Per-source EPG URL registry.
//!
//! One row per resolved XMLTV feed for a source. A source may have multiple
//! rows — the provider-supplied feed (Xtream `xmltv.php` or M3U `url-tvg`)
//! plus any community fallbacks. The refresh worker iterates over these rows
//! and downloads each in turn.

use nova_core::now_ts;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};

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

/// What the UI may know about a guide feed: never its URL (an Xtream feed URL
/// carries the account's credentials).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpgFeedStatus {
    pub id: i64,
    pub kind: String,
    pub last_refresh_utc: Option<i64>,
    pub last_error: Option<String>,
    pub programme_count: i64,
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

    /// A source's feeds with their refresh state and programme counts.
    pub async fn feed_status(&self, source_id: i64) -> Result<Vec<EpgFeedStatus>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT e.id as "id!: i64", e.kind, e.last_refresh_utc, e.last_error,
                      (SELECT COUNT(*) FROM programs p WHERE p.epg_source_id = e.id) as "programme_count!: i64"
               FROM epg_sources e WHERE e.source_id = ? ORDER BY e.id ASC"#,
            source_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| EpgFeedStatus {
                id: r.id,
                kind: r.kind,
                last_refresh_utc: r.last_refresh_utc,
                // Errors stored before URLs were stripped from them can still
                // name a feed URL (with an Xtream account's credentials).
                last_error: r.last_error.as_deref().map(nova_core::redact_urls),
                programme_count: r.programme_count,
            })
            .collect())
    }

    /// Distinct non-empty tvg-ids of a source's channels (what XMLTV channel
    /// ids are matched against).
    pub async fn channel_tvg_ids(&self, source_id: i64) -> Result<Vec<String>, StorageError> {
        let rows = sqlx::query_scalar!(
            r#"SELECT DISTINCT tvg_id as "tvg_id!: String" FROM channels
               WHERE source_id = ? AND tvg_id IS NOT NULL AND tvg_id <> ''"#,
            source_id
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    /// Replace the channel logos one feed contributes to its source.
    /// `logos` are `(channel_logo_key, url)` pairs (`nova_epg::match_channel_icons`).
    pub async fn replace_channel_logos_tx(
        tx: &mut Transaction<'_, Sqlite>,
        epg_source_id: i64,
        source_id: i64,
        logos: &[(String, String)],
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "DELETE FROM epg_channel_logos WHERE epg_source_id = ?",
            epg_source_id
        )
        .execute(&mut **tx)
        .await?;
        let now = now_ts();
        for (key, url) in logos {
            sqlx::query!(
                "INSERT OR REPLACE INTO epg_channel_logos
                     (epg_source_id, source_id, channel_key, url, updated_at)
                 VALUES (?, ?, ?, ?, ?)",
                epg_source_id,
                source_id,
                key,
                url,
                now,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
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
