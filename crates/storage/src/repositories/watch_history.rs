use nova_core::{now_ts, ContinueWatchingItem};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;
use crate::repositories::canonical::CanonicalRepository;

/// A `watch_history` row captured before a refresh, keyed by the media's
/// `stream_url` so it can be re-pointed at the new row IDs after re-import.
/// Only `vod`/`live` are snapshot — episodes are lazy-fetched and get new IDs,
/// so their resume can't be remapped.
#[derive(Debug, Clone)]
pub struct WatchSnapshot {
    pub media_kind: String,
    pub stream_url: String,
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    pub completed: bool,
    pub watched_at: i64,
    pub caption_mode: Option<String>,
}

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
        let mut tx = self.pool.begin().await?;
        // Movie resume/completion is owned by the canonical entity; the
        // row-keyed history is kept as its legacy mirror.
        if media_kind == "vod" {
            if let Some(entity) = CanonicalRepository::entity_for_vod_tx(&mut tx, media_id).await? {
                CanonicalRepository::record_progress_conn(
                    &mut tx,
                    &entity,
                    position_sec,
                    duration_sec,
                    completed,
                    caption_mode,
                )
                .await?;
            }
        }
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
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Snapshot resume state for a source's `vod`/`live` media, keyed by
    /// `stream_url`, so refresh can restore it onto the re-imported rows.
    pub async fn snapshot_for_source_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<Vec<WatchSnapshot>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT
                wh.media_kind   as "media_kind!: String",
                wh.position_sec as "position_sec!: f64",
                wh.duration_sec as "duration_sec?: f64",
                wh.completed    as "completed!: i64",
                wh.watched_at   as "watched_at!: i64",
                wh.caption_mode as "caption_mode?: String",
                COALESCE(v.stream_url, c.stream_url) as "stream_url!: String"
               FROM watch_history wh
               LEFT JOIN vod_items v ON wh.media_kind = 'vod'  AND v.id = wh.media_id AND v.source_id = ?
               LEFT JOIN channels  c ON wh.media_kind = 'live' AND c.id = wh.media_id AND c.source_id = ?
               WHERE v.id IS NOT NULL OR c.id IS NOT NULL"#,
            source_id,
            source_id,
        )
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| WatchSnapshot {
                media_kind: r.media_kind,
                stream_url: r.stream_url,
                position_sec: r.position_sec,
                duration_sec: r.duration_sec,
                completed: r.completed != 0,
                watched_at: r.watched_at,
                caption_mode: r.caption_mode,
            })
            .collect())
    }

    /// Delete a source's `watch_history` rows (vod/live/episode) ahead of a
    /// re-import. Must run while the media rows still exist (the subqueries
    /// resolve `media_id`). Episodes are cleared (can't be remapped).
    pub async fn delete_for_source_media_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            r#"DELETE FROM watch_history
               WHERE (media_kind = 'vod'     AND media_id IN (SELECT id FROM vod_items       WHERE source_id = ?))
                  OR (media_kind = 'live'    AND media_id IN (SELECT id FROM channels        WHERE source_id = ?))
                  OR (media_kind = 'episode' AND media_id IN (SELECT id FROM series_episodes WHERE source_id = ?))"#,
            source_id,
            source_id,
            source_id,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Re-insert snapshot rows, resolving the new `media_id` from the
    /// re-imported `vod_items`/`channels` row with the same `stream_url`.
    /// Rows whose URL no longer exists (removed upstream) are skipped.
    pub async fn restore_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
        rows: &[WatchSnapshot],
    ) -> Result<(), StorageError> {
        for r in rows {
            let new_id: Option<i64> = match r.media_kind.as_str() {
                "vod" => sqlx::query_scalar!(
                    "SELECT id FROM vod_items WHERE source_id = ? AND stream_url = ? LIMIT 1",
                    source_id,
                    r.stream_url,
                )
                .fetch_optional(&mut **tx)
                .await?
                .flatten(),
                "live" => sqlx::query_scalar!(
                    "SELECT id FROM channels WHERE source_id = ? AND stream_url = ? LIMIT 1",
                    source_id,
                    r.stream_url,
                )
                .fetch_optional(&mut **tx)
                .await?
                .flatten(),
                _ => None,
            };
            let Some(media_id) = new_id else { continue };
            let completed_flag: i64 = if r.completed { 1 } else { 0 };
            sqlx::query!(
                r#"INSERT INTO watch_history (media_kind, media_id, position_sec, duration_sec, watched_at, completed, caption_mode)
                   VALUES (?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(media_kind, media_id) DO UPDATE SET
                       position_sec = excluded.position_sec,
                       duration_sec = excluded.duration_sec,
                       watched_at   = excluded.watched_at,
                       completed    = excluded.completed,
                       caption_mode = excluded.caption_mode"#,
                r.media_kind,
                media_id,
                r.position_sec,
                r.duration_sec,
                r.watched_at,
                completed_flag,
                r.caption_mode,
            )
            .execute(&mut **tx)
            .await?;
        }
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
        if media_kind == "vod" {
            let mut conn = self.pool.acquire().await?;
            let entity = CanonicalRepository::entity_for_vod_conn(&mut conn, media_id).await?;
            if let Some(entity) = entity {
                let state = CanonicalRepository::resume_conn(&mut conn, &entity).await?;
                return Ok(state.map(|s| ResumePosition {
                    position_sec: s.position_sec,
                    duration_sec: s.duration_sec,
                    completed: s.completed,
                }));
            }
        }
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

    /// Continue Watching shelf, excluding completed entries. Movies come from
    /// entity-owned resume state (surviving refresh/credential rotation),
    /// shown through the entity's first available variant; episodes still use
    /// `watch_history`.
    pub async fn list_continue_watching(
        &self,
        limit: i64,
    ) -> Result<Vec<ContinueWatchingItem>, StorageError> {
        let rows = sqlx::query!(
            r#"
            SELECT media_kind as "media_kind!: String",
                   media_id as "media_id!: i64",
                   position_sec as "position_sec!: f64",
                   duration_sec as "duration_sec?: f64",
                   watched_at as "watched_at!: i64",
                   source_id as "source_id!: i64",
                   title as "title!: String",
                   subtitle as "subtitle?: String",
                   poster_url as "poster_url?: String"
            FROM (
                SELECT 'vod' AS media_kind, v.id AS media_id,
                       ews.position_sec, ews.duration_sec, ews.watched_at,
                       v.source_id, v.name AS title, NULL AS subtitle,
                       v.logo_url AS poster_url
                FROM entity_watch_state ews
                JOIN vod_items v ON v.id = (
                    SELECT MIN(s.vod_item_id) FROM stream_variants s
                    WHERE s.entity_id = ews.entity_id AND s.vod_item_id IS NOT NULL)
                WHERE ews.completed = 0
                UNION ALL
                SELECT 'episode', e.id, wh.position_sec, wh.duration_sec, wh.watched_at,
                       e.source_id, e.series_title,
                       'S' || printf('%02d', e.season) || 'E' || printf('%02d', e.episode),
                       e.logo_url
                FROM watch_history wh
                JOIN series_episodes e ON e.id = wh.media_id
                WHERE wh.media_kind = 'episode' AND wh.completed = 0
            )
            ORDER BY watched_at DESC
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
