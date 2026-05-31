use nova_core::{now_ts, ContentSort, SeriesEpisode, SeriesSummary};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;

pub struct SeriesRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SeriesRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert_batch(&self, entries: &[SeriesEpisode]) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        Self::insert_batch_tx(&mut tx, entries).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn insert_batch_tx(
        tx: &mut Transaction<'_, Sqlite>,
        entries: &[SeriesEpisode],
    ) -> Result<(), StorageError> {
        let added_at = now_ts();
        for e in entries {
            sqlx::query!(
                "INSERT INTO series_episodes (source_id, series_title, category, season, episode, episode_title, logo_url, stream_url, added_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                e.source_id,
                e.series_title,
                e.category,
                e.season,
                e.episode,
                e.episode_title,
                e.logo_url,
                e.stream_url,
                added_at,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// Snapshot `(series_title, is_favorite, is_in_watchlist)` for marked
    /// series of a source. Used by refresh to re-apply series-level flags.
    pub async fn snapshot_series_flags_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<Vec<(String, bool, bool)>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT series_title as "series_title!: String",
                      MAX(is_favorite) as "fav!: i64",
                      MAX(is_in_watchlist) as "wl!: i64"
               FROM series_episodes
               WHERE source_id = ?
               GROUP BY series_title
               HAVING MAX(is_favorite) = 1 OR MAX(is_in_watchlist) = 1"#,
            source_id,
        )
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.series_title, r.fav != 0, r.wl != 0))
            .collect())
    }

    pub async fn apply_series_flags_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
        flags: &[(String, bool, bool)],
    ) -> Result<(), StorageError> {
        for (title, fav, wl) in flags {
            let f = *fav as i64;
            let w = *wl as i64;
            sqlx::query!(
                "UPDATE series_episodes SET is_favorite = ?, is_in_watchlist = ? WHERE source_id = ? AND series_title = ?",
                f,
                w,
                source_id,
                title,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    pub async fn delete_by_source_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<(), StorageError> {
        sqlx::query!("DELETE FROM series_episodes WHERE source_id = ?", source_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Replace all rows for a series with freshly-fetched episodes (used by the
    /// lazy Xtream `get_series_info` path). Deletes the catalog sentinel and any
    /// prior episodes, then inserts the new set — preserving any series-level
    /// favorite/watchlist flag the user set before episodes were fetched.
    pub async fn replace_series_episodes(
        &self,
        source_id: i64,
        series_title: &str,
        episodes: &[SeriesEpisode],
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        let flags = sqlx::query!(
            r#"SELECT MAX(is_favorite) as "fav?: i64", MAX(is_in_watchlist) as "wl?: i64"
               FROM series_episodes WHERE source_id = ? AND series_title = ?"#,
            source_id,
            series_title,
        )
        .fetch_one(&mut *tx)
        .await?;
        let fav = flags.fav.unwrap_or(0);
        let wl = flags.wl.unwrap_or(0);

        sqlx::query!(
            "DELETE FROM series_episodes WHERE source_id = ? AND series_title = ?",
            source_id,
            series_title,
        )
        .execute(&mut *tx)
        .await?;

        let added_at = now_ts();
        for e in episodes {
            sqlx::query!(
                "INSERT INTO series_episodes (source_id, series_title, category, season, episode, episode_title, logo_url, stream_url, added_at, is_favorite, is_in_watchlist)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                e.source_id,
                e.series_title,
                e.category,
                e.season,
                e.episode,
                e.episode_title,
                e.logo_url,
                e.stream_url,
                added_at,
                fav,
                wl,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Returns one summary per unique series title.
    ///
    /// `is_favorite` / `is_in_watchlist` are true when *any* episode of the title is
    /// marked — the toggles below always update all episodes at once, so this is consistent.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_series(
        &self,
        source_id: i64,
        category: Option<&str>,
        query: Option<&str>,
        favorites_only: bool,
        watchlist_only: bool,
        sort: ContentSort,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SeriesSummary>, StorageError> {
        let search = like_query(query);
        let search = search.as_deref();
        let fav_flag: i64 = if favorites_only { 1 } else { 0 };
        let wl_flag: i64 = if watchlist_only { 1 } else { 0 };
        let rows = match sort {
            ContentSort::RecentlyAdded => sqlx::query!(
                r#"SELECT series_title as "series_title!: String",
                        MIN(category) as "category?: String",
                        SUM(CASE WHEN season > 0 THEN 1 ELSE 0 END) as "episode_count!: i64",
                        COUNT(DISTINCT CASE WHEN season > 0 THEN season END) as "season_count!: i64",
                        MIN(logo_url) as "poster_url?: String",
                        MAX(is_favorite) as "is_favorite!: i64",
                        MAX(is_in_watchlist) as "is_in_watchlist!: i64",
                        MAX(added_at) as "added_at?: i64"
                 FROM series_episodes
                 WHERE source_id = ?
                   AND (? IS NULL OR category = ?)
                   AND (? IS NULL OR series_title LIKE ?)
                 GROUP BY series_title
                 HAVING (? = 0 OR MAX(is_favorite) = 1)
                    AND (? = 0 OR MAX(is_in_watchlist) = 1)
                 ORDER BY MAX(added_at) DESC, series_title ASC
                 LIMIT ? OFFSET ?"#,
                source_id,
                category,
                category,
                search,
                search,
                fav_flag,
                wl_flag,
                limit,
                offset,
            )
            .fetch_all(self.pool)
            .await?
            .into_iter()
            .map(|r| SeriesSummary {
                series_title: r.series_title,
                source_id,
                category: r.category,
                episode_count: r.episode_count,
                season_count: r.season_count,
                poster_url: r.poster_url,
                is_favorite: r.is_favorite != 0,
                is_in_watchlist: r.is_in_watchlist != 0,
                added_at: r.added_at,
            })
            .collect(),
            ContentSort::Default => sqlx::query!(
                r#"SELECT series_title as "series_title!: String",
                        MIN(category) as "category?: String",
                        SUM(CASE WHEN season > 0 THEN 1 ELSE 0 END) as "episode_count!: i64",
                        COUNT(DISTINCT CASE WHEN season > 0 THEN season END) as "season_count!: i64",
                        MIN(logo_url) as "poster_url?: String",
                        MAX(is_favorite) as "is_favorite!: i64",
                        MAX(is_in_watchlist) as "is_in_watchlist!: i64",
                        MAX(added_at) as "added_at?: i64"
                 FROM series_episodes
                 WHERE source_id = ?
                   AND (? IS NULL OR category = ?)
                   AND (? IS NULL OR series_title LIKE ?)
                 GROUP BY series_title
                 HAVING (? = 0 OR MAX(is_favorite) = 1)
                    AND (? = 0 OR MAX(is_in_watchlist) = 1)
                 ORDER BY series_title ASC
                 LIMIT ? OFFSET ?"#,
                source_id,
                category,
                category,
                search,
                search,
                fav_flag,
                wl_flag,
                limit,
                offset,
            )
            .fetch_all(self.pool)
            .await?
            .into_iter()
            .map(|r| SeriesSummary {
                series_title: r.series_title,
                source_id,
                category: r.category,
                episode_count: r.episode_count,
                season_count: r.season_count,
                poster_url: r.poster_url,
                is_favorite: r.is_favorite != 0,
                is_in_watchlist: r.is_in_watchlist != 0,
                added_at: r.added_at,
            })
            .collect(),
        };
        Ok(rows)
    }

    pub async fn list_episodes(
        &self,
        source_id: i64,
        series_title: &str,
    ) -> Result<Vec<SeriesEpisode>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", source_id, series_title, category, season, episode, episode_title, logo_url, stream_url, added_at
             FROM series_episodes
             WHERE source_id = ? AND series_title = ?
             ORDER BY season ASC, episode ASC"#,
            source_id,
            series_title,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| SeriesEpisode {
                id: r.id,
                source_id: r.source_id,
                series_title: r.series_title,
                category: r.category,
                season: r.season,
                episode: r.episode,
                episode_title: r.episode_title,
                logo_url: r.logo_url,
                stream_url: r.stream_url,
                added_at: r.added_at,
            })
            .collect())
    }

    /// A single episode by id. Used by the player to resolve the series an
    /// episode belongs to (for the cinema-mode episode browser / autoplay).
    pub async fn get_episode(&self, id: i64) -> Result<Option<SeriesEpisode>, StorageError> {
        let row = sqlx::query!(
            r#"SELECT id as "id!: i64", source_id, series_title, category, season, episode, episode_title, logo_url, stream_url, added_at
             FROM series_episodes WHERE id = ?"#,
            id,
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| SeriesEpisode {
            id: r.id,
            source_id: r.source_id,
            series_title: r.series_title,
            category: r.category,
            season: r.season,
            episode: r.episode,
            episode_title: r.episode_title,
            logo_url: r.logo_url,
            stream_url: r.stream_url,
            added_at: r.added_at,
        }))
    }

    pub async fn count_series(
        &self,
        source_id: i64,
        category: Option<&str>,
        query: Option<&str>,
        favorites_only: bool,
        watchlist_only: bool,
    ) -> Result<i64, StorageError> {
        let search = like_query(query);
        let search = search.as_deref();
        let fav_flag: i64 = if favorites_only { 1 } else { 0 };
        let wl_flag: i64 = if watchlist_only { 1 } else { 0 };
        // Count distinct titles that match. Wrap the grouped result so HAVING applies.
        let row = sqlx::query!(
            r#"SELECT COUNT(*) as "n!: i64" FROM (
                SELECT series_title
                FROM series_episodes
                WHERE source_id = ?
                  AND (? IS NULL OR category = ?)
                  AND (? IS NULL OR series_title LIKE ?)
                GROUP BY series_title
                HAVING (? = 0 OR MAX(is_favorite) = 1)
                   AND (? = 0 OR MAX(is_in_watchlist) = 1)
              )"#,
            source_id,
            category,
            category,
            search,
            search,
            fav_flag,
            wl_flag,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(row.n)
    }

    /// Toggle favourite for an entire series (all episodes of `series_title`).
    pub async fn set_favorite(
        &self,
        source_id: i64,
        series_title: &str,
        is_favorite: bool,
    ) -> Result<(), StorageError> {
        let flag: i64 = if is_favorite { 1 } else { 0 };
        sqlx::query!(
            "UPDATE series_episodes SET is_favorite = ? WHERE source_id = ? AND series_title = ?",
            flag,
            source_id,
            series_title,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Toggle watchlist for an entire series (all episodes of `series_title`).
    pub async fn set_in_watchlist(
        &self,
        source_id: i64,
        series_title: &str,
        in_watchlist: bool,
    ) -> Result<(), StorageError> {
        let flag: i64 = if in_watchlist { 1 } else { 0 };
        sqlx::query!(
            "UPDATE series_episodes SET is_in_watchlist = ? WHERE source_id = ? AND series_title = ?",
            flag,
            source_id,
            series_title,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn categories(&self, source_id: i64) -> Result<Vec<String>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT DISTINCT category as "value!" FROM series_episodes
               WHERE source_id = ? AND category IS NOT NULL AND category != ''
               ORDER BY category ASC"#,
            source_id
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.value).collect())
    }
}

fn like_query(query: Option<&str>) -> Option<String> {
    query.map(|q| format!("%{q}%"))
}
