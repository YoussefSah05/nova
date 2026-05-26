use nova_core::{now_ts, ContinueWatchingItem};
use sqlx::SqlitePool;

use crate::error::StorageError;

/// Resume state for a single media item. Returned by [`WatchHistoryRepository::resume_position`].
#[derive(Debug, Clone, Copy)]
pub struct ResumePosition {
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    pub completed: bool,
}

pub struct WatchHistoryRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> WatchHistoryRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// INSERT-OR-REPLACE on (media_kind, media_id). Called periodically while
    /// the player is open (e.g. every 5s) so the Continue Watching shelf and
    /// the player's "resume" hint stay current.
    ///
    /// `caption_mode` is the user's caption selection at the time of write
    /// ('off' | 'source' | 'translation:<lang>' | 'dual:<src>:<tgt>'). In
    /// Phase 5 the frontend always sends 'off'; Phase 6/7 will pass real values.
    /// `None` leaves the existing value untouched so a Phase 5 client can't
    /// clobber a preference written by a future build.
    pub async fn record_progress(
        &self,
        media_kind: &str,
        media_id: i64,
        position_sec: f64,
        duration_sec: Option<f64>,
        completed: bool,
        caption_mode: Option<&str>,
    ) -> Result<(), StorageError> {
        let watched_at = now_ts();
        let completed_flag: i64 = if completed { 1 } else { 0 };
        sqlx::query!(
            r#"
            INSERT INTO watch_history (media_kind, media_id, position_sec, duration_sec, watched_at, completed, caption_mode)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(media_kind, media_id) DO UPDATE SET
                position_sec = excluded.position_sec,
                duration_sec = COALESCE(excluded.duration_sec, watch_history.duration_sec),
                watched_at   = excluded.watched_at,
                completed    = excluded.completed,
                caption_mode = COALESCE(excluded.caption_mode, watch_history.caption_mode)
            "#,
            media_kind,
            media_id,
            position_sec,
            duration_sec,
            watched_at,
            completed_flag,
            caption_mode,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Returns the latest position for a (kind, id) - used when opening the
    /// player to seek to the resume point. `None` if the user never started
    /// the item, or if they previously marked it completed.
    pub async fn resume_position(
        &self,
        media_kind: &str,
        media_id: i64,
    ) -> Result<Option<ResumePosition>, StorageError> {
        let row = sqlx::query!(
            r#"SELECT
                position_sec as "position_sec!: f64",
                duration_sec as "duration_sec?: f64",
                completed    as "completed!: i64"
               FROM watch_history
               WHERE media_kind = ? AND media_id = ?
               LIMIT 1"#,
            media_kind,
            media_id,
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| ResumePosition {
            position_sec: r.position_sec,
            duration_sec: r.duration_sec,
            completed: r.completed != 0,
        }))
    }

    /// Continue Watching shelf. Joins `watch_history` with `vod_items` /
    /// `series_episodes` and excludes completed entries.
    ///
    /// Until Phase 5 starts writing playback progress, `watch_history` is empty
    /// and this returns `Vec::new()`.
    pub async fn list_continue_watching(
        &self,
        limit: i64,
    ) -> Result<Vec<ContinueWatchingItem>, StorageError> {
        let rows = sqlx::query!(
            r#"
            SELECT
                wh.media_kind  as "media_kind!: String",
                wh.media_id    as "media_id!: i64",
                wh.position_sec as "position_sec!: f64",
                wh.duration_sec as "duration_sec?: f64",
                wh.watched_at  as "watched_at!: i64",
                COALESCE(v.source_id, e.source_id)        as "source_id!: i64",
                COALESCE(v.name, e.series_title)          as "title!: String",
                CASE
                    WHEN wh.media_kind = 'episode'
                    THEN 'S' || printf('%02d', e.season) || 'E' || printf('%02d', e.episode)
                    ELSE NULL
                END                                       as "subtitle?: String",
                COALESCE(v.logo_url, e.logo_url)          as "poster_url?: String"
            FROM watch_history wh
            LEFT JOIN vod_items       v ON wh.media_kind = 'vod'     AND v.id = wh.media_id
            LEFT JOIN series_episodes e ON wh.media_kind = 'episode' AND e.id = wh.media_id
            WHERE wh.completed = 0
              AND (v.id IS NOT NULL OR e.id IS NOT NULL)
            ORDER BY wh.watched_at DESC
            LIMIT ?
            "#,
            limit,
        )
        .fetch_all(self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ContinueWatchingItem {
                media_kind: r.media_kind,
                media_id: r.media_id,
                source_id: r.source_id,
                title: r.title,
                subtitle: r.subtitle,
                poster_url: r.poster_url,
                position_sec: r.position_sec,
                duration_sec: r.duration_sec,
                watched_at: r.watched_at,
            })
            .collect())
    }
}
