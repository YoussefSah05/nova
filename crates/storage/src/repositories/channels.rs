use nova_core::{now_ts, Channel, ContentSort};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;

pub struct ChannelRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ChannelRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert a batch of channels in a single transaction.
    pub async fn insert_batch(&self, entries: &[Channel]) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        Self::insert_batch_tx(&mut tx, entries).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn insert_batch_tx(
        tx: &mut Transaction<'_, Sqlite>,
        entries: &[Channel],
    ) -> Result<(), StorageError> {
        let added_at = now_ts();
        for c in entries {
            sqlx::query!(
                "INSERT INTO channels (source_id, tvg_id, name, country, category, raw_group, logo_url, stream_url, added_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                c.source_id,
                c.tvg_id,
                c.name,
                c.country,
                c.category,
                c.raw_group,
                c.logo_url,
                c.stream_url,
                added_at,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// Snapshot `(stream_url, is_favorite, is_in_watchlist)` for marked channels
    /// of a source. Used by refresh to re-apply user flags after a re-import.
    pub async fn snapshot_flags_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<Vec<(String, bool, bool)>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT stream_url as "stream_url!: String",
                      is_favorite as "is_favorite!: i64",
                      is_in_watchlist as "is_in_watchlist!: i64"
               FROM channels
               WHERE source_id = ? AND (is_favorite = 1 OR is_in_watchlist = 1)"#,
            source_id,
        )
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.stream_url, r.is_favorite != 0, r.is_in_watchlist != 0))
            .collect())
    }

    /// Re-apply favorite/watchlist flags to freshly re-imported channels by
    /// matching `stream_url`.
    pub async fn restore_flags_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
        flags: &[(String, bool, bool)],
    ) -> Result<(), StorageError> {
        for (url, fav, wl) in flags {
            let f = *fav as i64;
            let w = *wl as i64;
            sqlx::query!(
                "UPDATE channels SET is_favorite = ?, is_in_watchlist = ? WHERE source_id = ? AND stream_url = ?",
                f,
                w,
                source_id,
                url,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// Delete all channels for a source (refresh clears before re-importing).
    pub async fn delete_by_source_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<(), StorageError> {
        sqlx::query!("DELETE FROM channels WHERE source_id = ?", source_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        &self,
        source_id: i64,
        limit: i64,
        offset: i64,
        country: Option<&str>,
        category: Option<&str>,
        query: Option<&str>,
        favorites_only: bool,
        watchlist_only: bool,
        sort: ContentSort,
    ) -> Result<Vec<Channel>, StorageError> {
        let search = like_query(query);
        let search = search.as_deref();
        let fav_flag: i64 = if favorites_only { 1 } else { 0 };
        let wl_flag: i64 = if watchlist_only { 1 } else { 0 };
        let rows = match sort {
            ContentSort::RecentlyAdded => sqlx::query!(
                r#"SELECT id as "id!: i64", source_id, tvg_id, name, country, category, raw_group, logo_url, stream_url, is_favorite, is_in_watchlist, last_played_at, added_at
                 FROM channels
                 WHERE source_id = ?
                   AND (? IS NULL OR country = ?)
                   AND (? IS NULL OR category = ?)
                   AND (? IS NULL OR name LIKE ? OR category LIKE ? OR raw_group LIKE ?)
                   AND (? = 0 OR is_favorite = 1)
                   AND (? = 0 OR is_in_watchlist = 1)
                 ORDER BY added_at DESC, id DESC LIMIT ? OFFSET ?"#,
                source_id,
                country, country,
                category, category,
                search, search, search, search,
                fav_flag,
                wl_flag,
                limit, offset,
            ).fetch_all(self.pool).await?
            .into_iter()
            .map(|r| Channel {
                id: r.id,
                source_id: r.source_id,
                tvg_id: r.tvg_id,
                name: r.name,
                country: r.country,
                category: r.category,
                raw_group: r.raw_group,
                logo_url: r.logo_url,
                stream_url: r.stream_url,
                is_favorite: r.is_favorite != 0,
                is_in_watchlist: r.is_in_watchlist != 0,
                last_played_at: r.last_played_at,
                added_at: r.added_at,
            })
            .collect(),
            ContentSort::Default => sqlx::query!(
                r#"SELECT id as "id!: i64", source_id, tvg_id, name, country, category, raw_group, logo_url, stream_url, is_favorite, is_in_watchlist, last_played_at, added_at
                 FROM channels
                 WHERE source_id = ?
                   AND (? IS NULL OR country = ?)
                   AND (? IS NULL OR category = ?)
                   AND (? IS NULL OR name LIKE ? OR category LIKE ? OR raw_group LIKE ?)
                   AND (? = 0 OR is_favorite = 1)
                   AND (? = 0 OR is_in_watchlist = 1)
                 ORDER BY id ASC LIMIT ? OFFSET ?"#,
                source_id,
                country, country,
                category, category,
                search, search, search, search,
                fav_flag,
                wl_flag,
                limit, offset,
            ).fetch_all(self.pool).await?
            .into_iter()
            .map(|r| Channel {
                id: r.id,
                source_id: r.source_id,
                tvg_id: r.tvg_id,
                name: r.name,
                country: r.country,
                category: r.category,
                raw_group: r.raw_group,
                logo_url: r.logo_url,
                stream_url: r.stream_url,
                is_favorite: r.is_favorite != 0,
                is_in_watchlist: r.is_in_watchlist != 0,
                last_played_at: r.last_played_at,
                added_at: r.added_at,
            })
            .collect(),
        };
        Ok(rows)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn count_filtered(
        &self,
        source_id: i64,
        country: Option<&str>,
        category: Option<&str>,
        query: Option<&str>,
        favorites_only: bool,
        watchlist_only: bool,
    ) -> Result<i64, StorageError> {
        let search = like_query(query);
        let search = search.as_deref();
        let fav_flag: i64 = if favorites_only { 1 } else { 0 };
        let wl_flag: i64 = if watchlist_only { 1 } else { 0 };
        let row = sqlx::query!(
            r#"SELECT COUNT(*) as "n!: i64"
             FROM channels
             WHERE source_id = ?
               AND (? IS NULL OR country = ?)
               AND (? IS NULL OR category = ?)
               AND (? IS NULL OR name LIKE ? OR category LIKE ? OR raw_group LIKE ?)
               AND (? = 0 OR is_favorite = 1)
               AND (? = 0 OR is_in_watchlist = 1)"#,
            source_id,
            country,
            country,
            category,
            category,
            search,
            search,
            search,
            search,
            fav_flag,
            wl_flag,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(row.n)
    }

    pub async fn set_favorite(&self, id: i64, is_favorite: bool) -> Result<(), StorageError> {
        let flag: i64 = if is_favorite { 1 } else { 0 };
        sqlx::query!("UPDATE channels SET is_favorite = ? WHERE id = ?", flag, id,)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_in_watchlist(&self, id: i64, in_watchlist: bool) -> Result<(), StorageError> {
        let flag: i64 = if in_watchlist { 1 } else { 0 };
        sqlx::query!(
            "UPDATE channels SET is_in_watchlist = ? WHERE id = ?",
            flag,
            id,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn countries(&self, source_id: i64) -> Result<Vec<String>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT DISTINCT country as "value!" FROM channels
               WHERE source_id = ? AND country IS NOT NULL AND country != ''
               ORDER BY country ASC"#,
            source_id
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.value).collect())
    }

    pub async fn categories(&self, source_id: i64) -> Result<Vec<String>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT DISTINCT category as "value!" FROM channels
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
