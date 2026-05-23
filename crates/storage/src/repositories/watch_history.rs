use nova_core::ContinueWatchingItem;
use sqlx::SqlitePool;

use crate::error::StorageError;

pub struct WatchHistoryRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> WatchHistoryRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
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
