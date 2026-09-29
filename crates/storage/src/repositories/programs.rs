//! EPG programmes — feed-owned bulk replace, prune, and time-window queries.
//!
//! Every programme belongs to the XMLTV feed (`epg_sources` row) that
//! supplied it. A feed's refresh stages its programmes, then replaces only
//! its own rows, so feeds that cover the same channel ID never overwrite each
//! other, and deleting a source removes its feeds' programmes (cascade). A
//! channel shows one feed's guide, preferring the feeds of the source whose
//! channels are on screen.

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

/// Now-and-next for one channel row (`channels.id`), from the guide of the
/// channel's own source where it has one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelNowAndNext {
    pub channel_id: i64,
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

    /// Start a feed refresh: drop anything an interrupted refresh of this
    /// feed left in staging.
    pub async fn clear_staged(&self, epg_source_id: i64) -> Result<u64, StorageError> {
        let r = sqlx::query!(
            "DELETE FROM program_staging WHERE epg_source_id = ?",
            epg_source_id
        )
        .execute(self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Stage a batch of a feed's programmes in its own short transaction, so
    /// other writers interleave with a long refresh.
    pub async fn stage_batch(
        &self,
        epg_source_id: i64,
        programs: &[Program],
    ) -> Result<u64, StorageError> {
        let mut tx = self.pool.begin().await?;
        for p in programs {
            sqlx::query!(
                "INSERT INTO program_staging (epg_source_id, channel_tvg_id, title, sub_title, description, start_utc, stop_utc, category, icon_url)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                epg_source_id,
                p.channel_tvg_id,
                p.title,
                p.sub_title,
                p.description,
                p.start_utc,
                p.stop_utc,
                p.category,
                p.icon_url,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(programs.len() as u64)
    }

    /// Replace the feed's programmes with what it staged: its previous rows
    /// go, and so do programmes written before feeds owned them (no feed) for
    /// the channels the feed now covers. Returns the programmes published.
    pub async fn publish_staged_tx(
        tx: &mut Transaction<'_, Sqlite>,
        epg_source_id: i64,
    ) -> Result<u64, StorageError> {
        sqlx::query!(
            "DELETE FROM programs WHERE epg_source_id = ?",
            epg_source_id
        )
        .execute(&mut **tx)
        .await?;
        sqlx::query!(
            "DELETE FROM programs WHERE epg_source_id IS NULL AND channel_tvg_id IN
               (SELECT channel_tvg_id FROM program_staging WHERE epg_source_id = ?)",
            epg_source_id
        )
        .execute(&mut **tx)
        .await?;
        let published = sqlx::query!(
            "INSERT INTO programs (channel_tvg_id, title, sub_title, description, start_utc, stop_utc, category, icon_url, epg_source_id)
             SELECT channel_tvg_id, title, sub_title, description, start_utc, stop_utc, category, icon_url, epg_source_id
             FROM program_staging WHERE epg_source_id = ? ORDER BY id",
            epg_source_id
        )
        .execute(&mut **tx)
        .await?
        .rows_affected();
        sqlx::query!(
            "DELETE FROM program_staging WHERE epg_source_id = ?",
            epg_source_id
        )
        .execute(&mut **tx)
        .await?;
        Ok(published)
    }

    /// Drop rows that ended before `cutoff_utc`. Called after each refresh so
    /// the table doesn't grow unbounded over weeks/months of EPG history.
    pub async fn prune_before(&self, cutoff_utc: i64) -> Result<u64, StorageError> {
        let r = sqlx::query!("DELETE FROM programs WHERE stop_utc < ?", cutoff_utc)
            .execute(self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// The one feed whose guide a channel shows, so "now", "next" and the
    /// grid never mix feeds: with `prefer_source`, that source's first feed
    /// covering the channel; otherwise (or if it has none) the first feed
    /// covering it; programmes written before feeds owned them last. `None`
    /// also when nothing covers the channel.
    async fn guide_feed(
        &self,
        tvg_id: &str,
        prefer_source: Option<i64>,
    ) -> Result<Option<i64>, StorageError> {
        let row = sqlx::query!(
            r#"SELECT f.feed AS "feed?: i64"
               FROM (SELECT DISTINCT epg_source_id AS feed FROM programs WHERE channel_tvg_id = ?1) f
               LEFT JOIN epg_sources e ON e.id = f.feed
               ORDER BY (?2 IS NOT NULL AND e.source_id = ?2) DESC, f.feed IS NULL, f.feed
               LIMIT 1"#,
            tvg_id,
            prefer_source,
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row.and_then(|r| r.feed))
    }

    /// Now-and-next for one channel, both from the channel's guide feed
    /// ([`Self::guide_feed`]).
    pub async fn now_and_next(
        &self,
        tvg_id: &str,
        now_utc: i64,
        prefer_source: Option<i64>,
    ) -> Result<NowAndNext, StorageError> {
        let feed = self.guide_feed(tvg_id, prefer_source).await?;
        let now = sqlx::query!(
            r#"SELECT channel_tvg_id, title, sub_title, description,
                      start_utc as "start_utc!: i64", stop_utc as "stop_utc!: i64",
                      category, icon_url
               FROM programs
               WHERE channel_tvg_id = ? AND epg_source_id IS ? AND start_utc <= ? AND stop_utc > ?
               ORDER BY start_utc DESC
               LIMIT 1"#,
            tvg_id,
            feed,
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
               WHERE channel_tvg_id = ? AND epg_source_id IS ? AND start_utc > ?
               ORDER BY start_utc ASC
               LIMIT 1"#,
            tvg_id,
            feed,
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
        prefer_source: Option<i64>,
    ) -> Result<Vec<NowAndNext>, StorageError> {
        let mut out = Vec::with_capacity(tvg_ids.len());
        for id in tvg_ids {
            out.push(self.now_and_next(id, now_utc, prefer_source).await?);
        }
        Ok(out)
    }

    /// Now-and-next for channel rows, each read from its guide feed with its
    /// own source preferred. One entry per requested id, in order; channels
    /// that don't exist or have no `tvg_id` come back empty.
    pub async fn now_and_next_for_channels(
        &self,
        channel_ids: &[i64],
        now_utc: i64,
    ) -> Result<Vec<ChannelNowAndNext>, StorageError> {
        let mut out = Vec::with_capacity(channel_ids.len());
        for &channel_id in channel_ids {
            let channel = sqlx::query!(
                "SELECT tvg_id, source_id FROM channels WHERE id = ?",
                channel_id
            )
            .fetch_optional(self.pool)
            .await?;
            let guide = match channel {
                Some(c) => match c.tvg_id.filter(|t| !t.is_empty()) {
                    Some(tvg_id) => Some(
                        self.now_and_next(&tvg_id, now_utc, Some(c.source_id))
                            .await?,
                    ),
                    None => None,
                },
                None => None,
            };
            let (now, next) = guide.map(|g| (g.now, g.next)).unwrap_or_default();
            out.push(ChannelNowAndNext {
                channel_id,
                now,
                next,
            });
        }
        Ok(out)
    }

    /// Range query for the full guide grid: each channel's programmes from
    /// its guide feed ([`Self::guide_feed`]) whose [start, stop] interval
    /// overlaps `[from_utc, to_utc)` — never duplicate rows from other feeds.
    pub async fn range(
        &self,
        tvg_ids: &[String],
        from_utc: i64,
        to_utc: i64,
        prefer_source: Option<i64>,
    ) -> Result<Vec<Program>, StorageError> {
        let mut out = Vec::new();
        for id in tvg_ids {
            let feed = self.guide_feed(id, prefer_source).await?;
            let rows = sqlx::query!(
                r#"SELECT channel_tvg_id, title, sub_title, description,
                          start_utc as "start_utc!: i64", stop_utc as "stop_utc!: i64",
                          category, icon_url
                   FROM programs
                   WHERE channel_tvg_id = ? AND epg_source_id IS ? AND stop_utc > ? AND start_utc < ?
                   ORDER BY start_utc ASC"#,
                id,
                feed,
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
