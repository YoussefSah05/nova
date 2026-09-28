use nova_core::catalog::media::episode_binding_key;
use nova_core::{now_ts, ContinueSection, ContinueWatchingItem, EpisodeNumberingKind};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;
use crate::repositories::canonical::CanonicalRepository;

/// A `watch_history` row captured before a refresh, keyed by the media's
/// `stream_url` so it can be re-pointed at the new row IDs after re-import.
/// Episodes whose rows are not back after the refresh (Xtream episodes are
/// re-fetched lazily) are parked in `pending_episode_history` instead.
#[derive(Debug, Clone)]
pub struct WatchSnapshot {
    pub media_kind: String,
    pub stream_url: String,
    /// The series an episode belonged to (episodes only).
    pub series_title: Option<String>,
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

    /// Snapshot resume state for a source's `vod`/`live`/`episode` media,
    /// keyed by `stream_url`, so refresh can restore it onto the new rows.
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
                COALESCE(v.stream_url, c.stream_url, e.stream_url) as "stream_url!: String",
                e.series_title  as "series_title?: String"
               FROM watch_history wh
               LEFT JOIN vod_items v ON wh.media_kind = 'vod'  AND v.id = wh.media_id AND v.source_id = ?
               LEFT JOIN channels  c ON wh.media_kind = 'live' AND c.id = wh.media_id AND c.source_id = ?
               LEFT JOIN series_episodes e
                 ON wh.media_kind = 'episode' AND e.id = wh.media_id AND e.source_id = ?
               WHERE v.id IS NOT NULL OR c.id IS NOT NULL OR e.id IS NOT NULL"#,
            source_id,
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
                series_title: r.series_title,
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
    /// resolve `media_id`); take [`Self::snapshot_for_source_tx`] first.
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
    /// re-imported row with the same `stream_url`. Movie/live rows whose URL
    /// no longer exists (removed upstream) are skipped. An episode whose row is
    /// not back yet is parked under its durable key and re-attached when the
    /// series' episodes are re-fetched ([`Self::reattach_pending_episodes_tx`]).
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
                "episode" => sqlx::query_scalar!(
                    "SELECT id FROM series_episodes WHERE source_id = ? AND stream_url = ? LIMIT 1",
                    source_id,
                    r.stream_url,
                )
                .fetch_optional(&mut **tx)
                .await?
                .flatten(),
                _ => None,
            };
            let Some(media_id) = new_id else {
                if r.media_kind == "episode" {
                    Self::park_episode_tx(tx, source_id, r).await?;
                }
                continue;
            };
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

    async fn park_episode_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
        r: &WatchSnapshot,
    ) -> Result<(), StorageError> {
        let key = episode_binding_key(&r.stream_url);
        let series = r.series_title.as_deref().unwrap_or("");
        let completed: i64 = r.completed.into();
        sqlx::query!(
            r#"INSERT INTO pending_episode_history
                   (source_id, episode_key, series_title, position_sec, duration_sec,
                    watched_at, completed, caption_mode)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(source_id, episode_key) DO UPDATE SET
                   series_title = excluded.series_title,
                   position_sec = excluded.position_sec,
                   duration_sec = excluded.duration_sec,
                   watched_at   = excluded.watched_at,
                   completed    = excluded.completed,
                   caption_mode = excluded.caption_mode
               WHERE excluded.watched_at >= pending_episode_history.watched_at"#,
            source_id,
            key,
            series,
            r.position_sec,
            r.duration_sec,
            r.watched_at,
            completed,
            r.caption_mode,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Re-attach parked episode history to freshly inserted episode rows
    /// (`(row id, stream_url)`), matching on the durable episode key. A newer
    /// history row for the episode (watched since) is never overwritten.
    pub async fn reattach_pending_episodes_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
        episodes: &[(i64, String)],
    ) -> Result<u64, StorageError> {
        let mut attached = 0;
        for (media_id, stream_url) in episodes {
            let key = episode_binding_key(stream_url);
            let Some(p) = sqlx::query!(
                r#"SELECT position_sec as "position_sec!: f64",
                          duration_sec as "duration_sec?: f64",
                          watched_at as "watched_at!: i64",
                          completed as "completed!: i64",
                          caption_mode as "caption_mode?: String"
                   FROM pending_episode_history WHERE source_id = ? AND episode_key = ?"#,
                source_id,
                key,
            )
            .fetch_optional(&mut **tx)
            .await?
            else {
                continue;
            };
            sqlx::query!(
                r#"INSERT INTO watch_history
                       (media_kind, media_id, position_sec, duration_sec, watched_at, completed, caption_mode)
                   VALUES ('episode', ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(media_kind, media_id) DO UPDATE SET
                       position_sec = excluded.position_sec,
                       duration_sec = excluded.duration_sec,
                       watched_at   = excluded.watched_at,
                       completed    = excluded.completed,
                       caption_mode = excluded.caption_mode
                   WHERE excluded.watched_at > watch_history.watched_at"#,
                media_id,
                p.position_sec,
                p.duration_sec,
                p.watched_at,
                p.completed,
                p.caption_mode,
            )
            .execute(&mut **tx)
            .await?;
            sqlx::query!(
                "DELETE FROM pending_episode_history WHERE source_id = ? AND episode_key = ?",
                source_id,
                key,
            )
            .execute(&mut **tx)
            .await?;
            attached += 1;
        }
        Ok(attached)
    }

    /// Re-attach any parked episode history of a source to its current
    /// episode rows. Cheap no-op when nothing is parked (the common case).
    pub async fn reattach_pending_for_source_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<u64, StorageError> {
        let pending = sqlx::query_scalar!(
            r#"SELECT COUNT(*) as "n!: i64" FROM pending_episode_history WHERE source_id = ?"#,
            source_id,
        )
        .fetch_one(&mut **tx)
        .await?;
        if pending == 0 {
            return Ok(0);
        }
        let rows: Vec<(i64, String)> = sqlx::query!(
            r#"SELECT id as "id!: i64", stream_url as "stream_url!: String"
               FROM series_episodes WHERE source_id = ? AND season > 0"#,
            source_id,
        )
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|r| (r.id, r.stream_url))
        .collect();
        Self::reattach_pending_episodes_tx(tx, source_id, &rows).await
    }

    /// Series of a source with parked episode history, most recent first:
    /// the ones worth re-fetching so Continue Watching refills after refresh.
    pub async fn series_with_pending_episodes(
        &self,
        source_id: i64,
        limit: i64,
    ) -> Result<Vec<String>, StorageError> {
        let rows = sqlx::query_scalar!(
            r#"SELECT series_title as "t!: String" FROM pending_episode_history
               WHERE source_id = ? AND series_title <> ''
               GROUP BY series_title
               ORDER BY MAX(watched_at) DESC
               LIMIT ?"#,
            source_id,
            limit,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
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
                ..ContinueWatchingItem::default()
            })
            .collect())
    }

    /// The Continue Watching rail of one browse tab in one source.
    pub async fn continue_watching_section(
        &self,
        section: ContinueSection,
        source_id: i64,
        limit: i64,
    ) -> Result<Vec<ContinueWatchingItem>, StorageError> {
        match section {
            ContinueSection::Movies => self.continue_movies(source_id, limit).await,
            ContinueSection::Series => self.continue_series(source_id, limit).await,
            ContinueSection::Live => self.recent_channels(source_id, limit).await,
        }
    }

    /// Unfinished canonical movies with a variant in the source, newest first,
    /// under their display title and a wide image when there is one.
    async fn continue_movies(
        &self,
        source_id: i64,
        limit: i64,
    ) -> Result<Vec<ContinueWatchingItem>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT e.id as "entity_id!: String",
                      v.id as "media_id!: i64",
                      ews.position_sec as "position_sec!: f64",
                      ews.duration_sec as "duration_sec?: f64",
                      ews.watched_at as "watched_at!: i64",
                      COALESCE(v.canonical_title, e.title, v.name) as "title!: String",
                      COALESCE(v.backdrop_url, v.poster_url, v.logo_url) as "poster_url?: String"
               FROM entity_watch_state ews
               JOIN canonical_entities e ON e.id = ews.entity_id
               JOIN vod_items v ON v.id = (
                   SELECT MIN(s.vod_item_id) FROM stream_variants s
                   WHERE s.entity_id = ews.entity_id AND s.source_id = ?
                     AND s.vod_item_id IS NOT NULL)
               WHERE ews.completed = 0
               ORDER BY ews.watched_at DESC
               LIMIT ?"#,
            source_id,
            limit,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ContinueWatchingItem {
                media_kind: "vod".into(),
                media_id: r.media_id,
                source_id,
                title: r.title,
                poster_url: r.poster_url,
                position_sec: r.position_sec,
                duration_sec: r.duration_sec,
                watched_at: r.watched_at,
                entity_id: Some(r.entity_id),
                ..ContinueWatchingItem::default()
            })
            .collect())
    }

    /// One card per series: the most recently watched episode when it is
    /// unfinished, otherwise the next episode after it ("Up next"). A series
    /// whose last watched episode was its final one is done and left out.
    async fn continue_series(
        &self,
        source_id: i64,
        limit: i64,
    ) -> Result<Vec<ContinueWatchingItem>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT e.id as "id!: i64",
                      e.series_title as "series_title!: String",
                      e.season as "season!: i64",
                      e.episode as "episode!: i64",
                      e.numbering as "numbering!: String",
                      e.episode_title,
                      e.logo_url,
                      wh.position_sec as "position_sec!: f64",
                      wh.duration_sec as "duration_sec?: f64",
                      wh.watched_at as "watched_at!: i64",
                      wh.completed as "completed!: i64",
                      sm.canonical_title as "canonical_title?: String",
                      sm.backdrop_url as "backdrop_url?: String",
                      sm.poster_url as "series_poster?: String"
               FROM watch_history wh
               JOIN series_episodes e ON e.id = wh.media_id
               LEFT JOIN series_meta sm
                 ON sm.source_id = e.source_id AND sm.series_title = e.series_title
               WHERE wh.media_kind = 'episode' AND e.source_id = ?
               ORDER BY wh.watched_at DESC, e.season DESC, e.episode DESC"#,
            source_id,
        )
        .fetch_all(self.pool)
        .await?;

        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for r in rows {
            if !seen.insert(r.series_title.clone()) {
                continue; // only the latest episode of a series decides its card
            }
            let title = nova_core::naming::display_title(nova_core::naming::DisplayNameSources {
                user: None,
                trusted_metadata: r.canonical_title.as_deref(),
                parsed: Some(&r.series_title),
                raw: &r.series_title,
            });
            let series_art = r.backdrop_url.clone().or(r.series_poster.clone());
            let mut item = ContinueWatchingItem {
                media_kind: "episode".into(),
                source_id,
                title,
                watched_at: r.watched_at,
                series_title: Some(r.series_title.clone()),
                ..ContinueWatchingItem::default()
            };
            if r.completed == 0 {
                item.media_id = r.id;
                item.position_sec = r.position_sec;
                item.duration_sec = r.duration_sec;
                item.season = Some(r.season);
                item.episode = Some(r.episode);
                item.numbering = Some(EpisodeNumberingKind::from_db(&r.numbering));
                item.episode_title = r.episode_title;
                item.poster_url = r.logo_url.or(series_art);
            } else {
                let Some(next) = sqlx::query!(
                    r#"SELECT id as "id!: i64", season as "season!: i64",
                              episode as "episode!: i64", numbering as "numbering!: String",
                              episode_title, logo_url
                       FROM series_episodes
                       WHERE source_id = ? AND series_title = ?
                         AND (season > ? OR (season = ? AND episode > ?))
                       ORDER BY season, episode
                       LIMIT 1"#,
                    source_id,
                    r.series_title,
                    r.season,
                    r.season,
                    r.episode,
                )
                .fetch_optional(self.pool)
                .await?
                else {
                    continue; // finished the series
                };
                item.media_id = next.id;
                item.season = Some(next.season);
                item.episode = Some(next.episode);
                item.numbering = Some(EpisodeNumberingKind::from_db(&next.numbering));
                item.episode_title = next.episode_title;
                item.poster_url = next.logo_url.or(series_art);
                item.up_next = true;
            }
            out.push(item);
            if out.len() as i64 >= limit {
                break;
            }
        }
        Ok(out)
    }

    /// Channels of the source watched most recently (live has no progress).
    /// The logo follows the channel read path's fallbacks (provider, EPG,
    /// directory).
    async fn recent_channels(
        &self,
        source_id: i64,
        limit: i64,
    ) -> Result<Vec<ContinueWatchingItem>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT c.id as "id!: i64",
                      c.name as "name!: String",
                      COALESCE(NULLIF(c.logo_url, ''),
                               (SELECT l.url FROM epg_channel_logos l
                                WHERE l.source_id = c.source_id
                                  AND l.channel_key = lower(c.tvg_id)
                                ORDER BY l.updated_at DESC LIMIT 1),
                               (SELECT d.url FROM directory_channel_logos d
                                WHERE d.source_id = c.source_id
                                  AND d.channel_key = COALESCE(lower(NULLIF(c.tvg_id, '')),
                                                               'name:' || lower(c.name))))
                          as "logo_url?: String",
                      wh.watched_at as "watched_at!: i64"
               FROM watch_history wh
               JOIN channels c ON c.id = wh.media_id
               WHERE wh.media_kind = 'live' AND c.source_id = ?
               ORDER BY wh.watched_at DESC
               LIMIT ?"#,
            source_id,
            limit,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ContinueWatchingItem {
                media_kind: "live".into(),
                media_id: r.id,
                source_id,
                title: r.name,
                poster_url: r.logo_url,
                watched_at: r.watched_at,
                ..ContinueWatchingItem::default()
            })
            .collect())
    }
}
