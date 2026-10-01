//! Read-only viewing history over durable progress, scoped to a playlist.
//! One entry per film, episode or channel: this is not a playback-event log.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::StorageError;

/// Secret-free history row. `media_id` is absent while an item is unavailable.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(
    test,
    ts(export, export_to = "../../../apps/desktop/src/types/generated/")
)]
#[serde(rename_all = "camelCase")]
pub struct LibraryHistoryItem {
    pub key: String,
    #[cfg_attr(test, ts(type = "'vod' | 'episode' | 'live'"))]
    pub media_kind: String,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub media_id: Option<i64>,
    pub entity_id: Option<String>,
    pub title: String,
    pub poster_url: Option<String>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub season: Option<i64>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub episode: Option<i64>,
    pub episode_title: Option<String>,
    pub numbered: bool,
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    #[cfg_attr(test, ts(type = "number"))]
    pub watched_at: i64,
    pub completed: bool,
}

pub struct LibraryHistoryRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> LibraryHistoryRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(
        &self,
        source_id: i64,
        kind: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LibraryHistoryItem>, StorageError> {
        let rows = sqlx::query!(
            r#"WITH history AS (
              SELECT 'movie:' || c.id AS key, 'vod' AS media_kind, v.id AS media_id,
                     c.id AS entity_id, COALESCE(v.canonical_title, c.title) AS title,
                     COALESCE(v.poster_url, v.logo_url) AS poster_url,
                     NULL AS season, NULL AS episode, NULL AS episode_title, 0 AS numbered,
                     w.position_sec, w.duration_sec, w.watched_at, w.completed
              FROM entity_watch_state w
              JOIN canonical_entities c ON c.id = w.entity_id AND c.state = 'active'
              LEFT JOIN vod_items v ON v.id = (
                SELECT MIN(s.vod_item_id) FROM stream_variants s
                WHERE s.entity_id = c.id AND s.source_id = ?1)
              WHERE EXISTS (SELECT 1 FROM stream_variants s
                            WHERE s.entity_id = c.id AND s.source_id = ?1)
              UNION ALL
              SELECT 'episode:' || w.series_entity_id || ':' || w.episode_key,
                     'episode', e.id, c.id, COALESCE(sm.canonical_title, c.title),
                     COALESCE(sm.poster_url, e.logo_url),
                     COALESCE(e.season, w.season), COALESCE(e.episode, w.episode),
                     e.episode_title, COALESCE(e.numbering = 'season_episode', 0),
                     w.position_sec, w.duration_sec, w.watched_at, w.completed
              FROM episode_watch_state w
              JOIN canonical_entities c ON c.id = w.series_entity_id AND c.state = 'active'
              JOIN series_bindings b ON b.source_id = ?1 AND b.binding_key = (
                SELECT MIN(b2.binding_key) FROM series_bindings b2
                WHERE b2.entity_id = c.id AND b2.source_id = ?1)
              LEFT JOIN series_episodes e ON e.id = (
                SELECT MIN(e2.id) FROM series_episodes e2
                JOIN series_bindings b2 ON b2.source_id = e2.source_id
                  AND b2.series_title = e2.series_title
                WHERE b2.entity_id = c.id AND b2.source_id = ?1
                  AND e2.episode_key = w.episode_key)
              LEFT JOIN series_meta sm ON sm.source_id = ?1
                AND sm.series_title = COALESCE(e.series_title, b.series_title)
              UNION ALL
              SELECT 'live:' || c.id, 'live', c.id, NULL, c.name,
                     c.logo_url, NULL, NULL, NULL, 0,
                     0.0, NULL, w.watched_at, 0
              FROM watch_history w
              JOIN channels c ON w.media_kind = 'live' AND c.id = w.media_id
              WHERE c.source_id = ?1
            )
            SELECT key as "key!: String", media_kind as "media_kind!: String",
                   media_id as "media_id?: i64", entity_id as "entity_id?: String",
                   title as "title!: String", poster_url as "poster_url?: String",
                   season as "season?: i64", episode as "episode?: i64",
                   episode_title as "episode_title?: String", numbered as "numbered!: bool",
                   position_sec as "position_sec!: f64", duration_sec as "duration_sec?: f64",
                   watched_at as "watched_at!: i64", completed as "completed!: bool"
            FROM history WHERE (?2 IS NULL OR media_kind = ?2)
            ORDER BY watched_at DESC, key ASC LIMIT ?3 OFFSET ?4"#,
            source_id,
            kind,
            limit,
            offset,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| LibraryHistoryItem {
                key: r.key,
                media_kind: r.media_kind,
                media_id: r.media_id,
                entity_id: r.entity_id,
                title: r.title,
                poster_url: r.poster_url,
                season: r.season,
                episode: r.episode,
                episode_title: r.episode_title,
                numbered: r.numbered,
                position_sec: r.position_sec,
                duration_sec: r.duration_sec,
                watched_at: r.watched_at,
                completed: r.completed,
            })
            .collect())
    }
}
