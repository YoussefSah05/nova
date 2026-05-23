//! EPG programmes — bulk insert, prune, and time-window queries.

use nova_epg::Program;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;

/// "What's on now" + "what's on next" for a single channel.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowAndNext {
    pub channel_tvg_id: String,
    pub now: Option<Program>,
    pub next: Option<Program>,
}

pub struct ProgramsRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ProgramsRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert a batch of programmes in a single transaction. Caller is
    /// expected to call [`Self::delete_for_channels_tx`] first when doing a
    /// full refresh, to avoid leaving stale rows behind.
    pub async fn insert_batch(&self, programs: &[Program]) -> Result<u64, StorageError> {
        let mut tx = self.pool.begin().await?;
        let n = Self::insert_batch_tx(&mut tx, programs).await?;
        tx.commit().await?;
        Ok(n)
    }

    pub async fn insert_batch_tx(
        tx: &mut Transaction<'_, Sqlite>,
        programs: &[Program],
    ) -> Result<u64, StorageError> {
        for p in programs {
            sqlx::query!(
                "INSERT INTO programs (channel_tvg_id, title, sub_title, description, start_utc, stop_utc, category, icon_url)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                p.channel_tvg_id,
                p.title,
                p.sub_title,
                p.description,
                p.start_utc,
                p.stop_utc,
                p.category,
                p.icon_url,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(programs.len() as u64)
    }

    /// Remove all programmes for the given channel IDs. Used before re-inserting
    /// from a freshly-fetched XMLTV feed so the table stays in sync with the
    /// upstream source rather than accumulating duplicates.
    pub async fn delete_for_channels_tx(
        tx: &mut Transaction<'_, Sqlite>,
        tvg_ids: &[String],
    ) -> Result<u64, StorageError> {
        if tvg_ids.is_empty() {
            return Ok(0);
        }
        let mut total: u64 = 0;
        // SQLite doesn't bind arrays — issue one DELETE per ID. Cheap since
        // each chunk is fully indexed by (channel_tvg_id, start_utc).
        for id in tvg_ids {
            let r = sqlx::query!("DELETE FROM programs WHERE channel_tvg_id = ?", id)
                .execute(&mut **tx)
                .await?;
            total += r.rows_affected();
        }
        Ok(total)
    }

    /// Drop rows that ended before `cutoff_utc`. Called after each refresh so
    /// the table doesn't grow unbounded over weeks/months of EPG history.
    pub async fn prune_before(&self, cutoff_utc: i64) -> Result<u64, StorageError> {
        let r = sqlx::query!("DELETE FROM programs WHERE stop_utc < ?", cutoff_utc)
            .execute(self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Now-and-next for one channel.
    pub async fn now_and_next(
        &self,
        tvg_id: &str,
        now_utc: i64,
    ) -> Result<NowAndNext, StorageError> {
        let now = sqlx::query!(
            r#"SELECT channel_tvg_id, title, sub_title, description,
                      start_utc as "start_utc!: i64", stop_utc as "stop_utc!: i64",
                      category, icon_url
               FROM programs
               WHERE channel_tvg_id = ? AND start_utc <= ? AND stop_utc > ?
               ORDER BY start_utc DESC
               LIMIT 1"#,
            tvg_id,
            now_utc,
            now_utc,
        )
        .fetch_optional(self.pool)
        .await?
        .map(|r| Program {
            channel_tvg_id: r.channel_tvg_id,
            title: r.title,
            sub_title: r.sub_title,
            description: r.description,
            start_utc: r.start_utc,
            stop_utc: r.stop_utc,
            category: r.category,
            icon_url: r.icon_url,
        });

        let next = sqlx::query!(
            r#"SELECT channel_tvg_id, title, sub_title, description,
                      start_utc as "start_utc!: i64", stop_utc as "stop_utc!: i64",
                      category, icon_url
               FROM programs
               WHERE channel_tvg_id = ? AND start_utc > ?
               ORDER BY start_utc ASC
               LIMIT 1"#,
            tvg_id,
            now_utc,
        )
        .fetch_optional(self.pool)
        .await?
        .map(|r| Program {
            channel_tvg_id: r.channel_tvg_id,
            title: r.title,
            sub_title: r.sub_title,
            description: r.description,
            start_utc: r.start_utc,
            stop_utc: r.stop_utc,
            category: r.category,
            icon_url: r.icon_url,
        });

        Ok(NowAndNext {
            channel_tvg_id: tvg_id.to_string(),
            now,
            next,
        })
    }

    /// Batched now-and-next for a list of channel IDs. Returns one entry per
    /// requested ID — entries with no EPG data still come back with both
    /// fields set to None so the frontend can render a stable list.
    pub async fn now_and_next_batch(
        &self,
        tvg_ids: &[String],
        now_utc: i64,
    ) -> Result<Vec<NowAndNext>, StorageError> {
        let mut out = Vec::with_capacity(tvg_ids.len());
        for id in tvg_ids {
            out.push(self.now_and_next(id, now_utc).await?);
        }
        Ok(out)
    }

    /// Range query for the full guide grid: all programmes for the given
    /// channels whose [start, stop] interval overlaps `[from_utc, to_utc)`.
    pub async fn range(
        &self,
        tvg_ids: &[String],
        from_utc: i64,
        to_utc: i64,
    ) -> Result<Vec<Program>, StorageError> {
        let mut out = Vec::new();
        for id in tvg_ids {
            let rows = sqlx::query!(
                r#"SELECT channel_tvg_id, title, sub_title, description,
                          start_utc as "start_utc!: i64", stop_utc as "stop_utc!: i64",
                          category, icon_url
                   FROM programs
                   WHERE channel_tvg_id = ? AND stop_utc > ? AND start_utc < ?
                   ORDER BY start_utc ASC"#,
                id,
                from_utc,
                to_utc,
            )
            .fetch_all(self.pool)
            .await?;
            out.extend(rows.into_iter().map(|r| Program {
                channel_tvg_id: r.channel_tvg_id,
                title: r.title,
                sub_title: r.sub_title,
                description: r.description,
                start_utc: r.start_utc,
                stop_utc: r.stop_utc,
                category: r.category,
                icon_url: r.icon_url,
            }));
        }
        Ok(out)
    }
}
