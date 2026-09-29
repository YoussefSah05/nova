//! EPG programmes — feed-owned bulk replace, prune, and time-window queries.
//!
//! Every programme belongs to the XMLTV feed (`epg_sources` row) that
//! supplied it. A feed's refresh replaces only its own rows, so feeds that
//! cover the same channel ID never overwrite each other, and deleting a
//! source removes its feeds' programmes (cascade). Reads can prefer the feeds
//! of the source whose channels are on screen.

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

    /// Start replacing a feed's programmes: drop everything it supplied
    /// before. Runs in the refresh transaction, before its inserts.
    pub async fn clear_feed_tx(
        tx: &mut Transaction<'_, Sqlite>,
        epg_source_id: i64,
    ) -> Result<u64, StorageError> {
        let r = sqlx::query!(
            "DELETE FROM programs WHERE epg_source_id = ?",
            epg_source_id
        )
        .execute(&mut **tx)
        .await?;
        Ok(r.rows_affected())
    }

    /// Drop programmes written before feeds owned them (no feed) for the
    /// channels a refresh now covers.
    pub async fn clear_unowned_tx(
        tx: &mut Transaction<'_, Sqlite>,
        tvg_ids: &[String],
    ) -> Result<u64, StorageError> {
        let mut total = 0;
        for id in tvg_ids {
            let r = sqlx::query!(
                "DELETE FROM programs WHERE epg_source_id IS NULL AND channel_tvg_id = ?",
                id
            )
            .execute(&mut **tx)
            .await?;
            total += r.rows_affected();
        }
        Ok(total)
    }

    /// Insert a batch of a feed's programmes.
    pub async fn insert_batch_tx(
        tx: &mut Transaction<'_, Sqlite>,
        epg_source_id: i64,
        programs: &[Program],
    ) -> Result<u64, StorageError> {
        for p in programs {
            sqlx::query!(
                "INSERT INTO programs (channel_tvg_id, title, sub_title, description, start_utc, stop_utc, category, icon_url, epg_source_id)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                p.channel_tvg_id,
                p.title,
                p.sub_title,
                p.description,
                p.start_utc,
                p.stop_utc,
                p.category,
                p.icon_url,
                epg_source_id,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(programs.len() as u64)
    }

    /// Drop rows that ended before `cutoff_utc`. Called after each refresh so
    /// the table doesn't grow unbounded over weeks/months of EPG history.
    pub async fn prune_before(&self, cutoff_utc: i64) -> Result<u64, StorageError> {
        let r = sqlx::query!("DELETE FROM programs WHERE stop_utc < ?", cutoff_utc)
            .execute(self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Now-and-next for one channel. With `prefer_source`, that source's
    /// feeds win when several feeds cover the channel.
    pub async fn now_and_next(
        &self,
        tvg_id: &str,
        now_utc: i64,
        prefer_source: Option<i64>,
    ) -> Result<NowAndNext, StorageError> {
        let now = sqlx::query!(
            r#"SELECT p.channel_tvg_id, p.title, p.sub_title, p.description,
                      p.start_utc as "start_utc!: i64", p.stop_utc as "stop_utc!: i64",
                      p.category, p.icon_url
               FROM programs p
               LEFT JOIN epg_sources e ON e.id = p.epg_source_id
               WHERE p.channel_tvg_id = ? AND p.start_utc <= ? AND p.stop_utc > ?
               ORDER BY (e.source_id IS ?) DESC, p.start_utc DESC
               LIMIT 1"#,
            tvg_id,
            now_utc,
            now_utc,
            prefer_source,
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
            r#"SELECT p.channel_tvg_id, p.title, p.sub_title, p.description,
                      p.start_utc as "start_utc!: i64", p.stop_utc as "stop_utc!: i64",
                      p.category, p.icon_url
               FROM programs p
               LEFT JOIN epg_sources e ON e.id = p.epg_source_id
               WHERE p.channel_tvg_id = ? AND p.start_utc > ?
               ORDER BY (e.source_id IS ?) DESC, p.start_utc ASC
               LIMIT 1"#,
            tvg_id,
            now_utc,
            prefer_source,
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
        prefer_source: Option<i64>,
    ) -> Result<Vec<NowAndNext>, StorageError> {
        let mut out = Vec::with_capacity(tvg_ids.len());
        for id in tvg_ids {
            out.push(self.now_and_next(id, now_utc, prefer_source).await?);
        }
        Ok(out)
    }

    /// Range query for the full guide grid: all programmes for the given
    /// channels whose [start, stop] interval overlaps `[from_utc, to_utc)`.
    /// With `prefer_source`, a channel that source's feeds cover shows only
    /// their programmes (no duplicate rows from other feeds).
    pub async fn range(
        &self,
        tvg_ids: &[String],
        from_utc: i64,
        to_utc: i64,
        prefer_source: Option<i64>,
    ) -> Result<Vec<Program>, StorageError> {
        let mut out = Vec::new();
        for id in tvg_ids {
            let rows = sqlx::query!(
                r#"SELECT p.channel_tvg_id, p.title, p.sub_title, p.description,
                          p.start_utc as "start_utc!: i64", p.stop_utc as "stop_utc!: i64",
                          p.category, p.icon_url
                   FROM programs p
                   LEFT JOIN epg_sources e ON e.id = p.epg_source_id
                   WHERE p.channel_tvg_id = ?1 AND p.stop_utc > ?2 AND p.start_utc < ?3
                     AND (?4 IS NULL OR e.source_id IS ?4 OR NOT EXISTS (
                           SELECT 1 FROM programs p2
                           JOIN epg_sources e2 ON e2.id = p2.epg_source_id
                           WHERE p2.channel_tvg_id = ?1 AND e2.source_id = ?4))
                   ORDER BY p.start_utc ASC"#,
                id,
                from_utc,
                to_utc,
                prefer_source,
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
