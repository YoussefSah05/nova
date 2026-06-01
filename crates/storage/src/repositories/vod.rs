use nova_core::{now_ts, ContentSort, VodItem};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;
use crate::repositories::enrich::{EnrichUpdate, UnenrichedRow};

pub struct VodRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> VodRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert_batch(&self, entries: &[VodItem]) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        Self::insert_batch_tx(&mut tx, entries).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn insert_batch_tx(
        tx: &mut Transaction<'_, Sqlite>,
        entries: &[VodItem],
    ) -> Result<(), StorageError> {
        let added_at = now_ts();
        for v in entries {
            sqlx::query!(
                "INSERT INTO vod_items (source_id, name, clean_title, year, logo_url, stream_url, category, added_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                v.source_id,
                v.name,
                v.clean_title,
                v.year,
                v.logo_url,
                v.stream_url,
                v.category,
                added_at,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// Snapshot `(stream_url, is_favorite, is_in_watchlist)` for marked VOD
    /// items of a source. Used by refresh to re-apply user flags.
    pub async fn snapshot_flags_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<Vec<(String, bool, bool)>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT stream_url as "stream_url!: String",
                      is_favorite as "is_favorite!: i64",
                      is_in_watchlist as "is_in_watchlist!: i64"
               FROM vod_items
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

    pub async fn restore_flags_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
        flags: &[(String, bool, bool)],
    ) -> Result<(), StorageError> {
        for (url, fav, wl) in flags {
            let f = *fav as i64;
            let w = *wl as i64;
            sqlx::query!(
                "UPDATE vod_items SET is_favorite = ?, is_in_watchlist = ? WHERE source_id = ? AND stream_url = ?",
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

    pub async fn delete_by_source_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<(), StorageError> {
        sqlx::query!("DELETE FROM vod_items WHERE source_id = ?", source_id)
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
        category: Option<&str>,
        query: Option<&str>,
        favorites_only: bool,
        watchlist_only: bool,
        sort: ContentSort,
    ) -> Result<Vec<VodItem>, StorageError> {
        let search = like_query(query);
        let search = search.as_deref();
        let fav_flag: i64 = if favorites_only { 1 } else { 0 };
        let wl_flag: i64 = if watchlist_only { 1 } else { 0 };
        let rows = match sort {
            ContentSort::RecentlyAdded => sqlx::query!(
                r#"SELECT id as "id!: i64", source_id, name, clean_title, year, logo_url, stream_url, category, is_favorite, is_in_watchlist, added_at
                 FROM vod_items
                 WHERE source_id = ?
                   AND (? IS NULL OR category = ?)
                   AND (? IS NULL OR name LIKE ? OR clean_title LIKE ? OR category LIKE ?)
                   AND (? = 0 OR is_favorite = 1)
                   AND (? = 0 OR is_in_watchlist = 1)
                 ORDER BY added_at DESC, id DESC LIMIT ? OFFSET ?"#,
                source_id,
                category, category,
                search, search, search, search,
                fav_flag,
                wl_flag,
                limit, offset,
            ).fetch_all(self.pool).await?
            .into_iter()
            .map(|r| VodItem {
                id: r.id,
                source_id: r.source_id,
                name: r.name,
                clean_title: r.clean_title,
                year: r.year,
                logo_url: r.logo_url,
                stream_url: r.stream_url,
                category: r.category,
                is_favorite: r.is_favorite != 0,
                is_in_watchlist: r.is_in_watchlist != 0,
                added_at: r.added_at,
            })
            .collect(),
            ContentSort::Default => sqlx::query!(
                r#"SELECT id as "id!: i64", source_id, name, clean_title, year, logo_url, stream_url, category, is_favorite, is_in_watchlist, added_at
                 FROM vod_items
                 WHERE source_id = ?
                   AND (? IS NULL OR category = ?)
                   AND (? IS NULL OR name LIKE ? OR clean_title LIKE ? OR category LIKE ?)
                   AND (? = 0 OR is_favorite = 1)
                   AND (? = 0 OR is_in_watchlist = 1)
                 ORDER BY id ASC LIMIT ? OFFSET ?"#,
                source_id,
                category, category,
                search, search, search, search,
                fav_flag,
                wl_flag,
                limit, offset,
            ).fetch_all(self.pool).await?
            .into_iter()
            .map(|r| VodItem {
                id: r.id,
                source_id: r.source_id,
                name: r.name,
                clean_title: r.clean_title,
                year: r.year,
                logo_url: r.logo_url,
                stream_url: r.stream_url,
                category: r.category,
                is_favorite: r.is_favorite != 0,
                is_in_watchlist: r.is_in_watchlist != 0,
                added_at: r.added_at,
            })
            .collect(),
        };
        Ok(rows)
    }

    pub async fn count_filtered(
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
        let row = sqlx::query!(
            r#"SELECT COUNT(*) as "n!: i64"
             FROM vod_items
             WHERE source_id = ?
               AND (? IS NULL OR category = ?)
               AND (? IS NULL OR name LIKE ? OR clean_title LIKE ? OR category LIKE ?)
               AND (? = 0 OR is_favorite = 1)
               AND (? = 0 OR is_in_watchlist = 1)"#,
            source_id,
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
        sqlx::query!(
            "UPDATE vod_items SET is_favorite = ? WHERE id = ?",
            flag,
            id,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_in_watchlist(&self, id: i64, in_watchlist: bool) -> Result<(), StorageError> {
        let flag: i64 = if in_watchlist { 1 } else { 0 };
        sqlx::query!(
            "UPDATE vod_items SET is_in_watchlist = ? WHERE id = ?",
            flag,
            id,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn categories(&self, source_id: i64) -> Result<Vec<String>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT DISTINCT category as "value!" FROM vod_items
               WHERE source_id = ? AND category IS NOT NULL AND category != ''
               ORDER BY category ASC"#,
            source_id
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.value).collect())
    }

    /// Movies for a source that have not yet been through enrichment.
    pub async fn list_unenriched(
        &self,
        source_id: i64,
    ) -> Result<Vec<UnenrichedRow>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", name, clean_title, category
               FROM vod_items
               WHERE source_id = ? AND enrich_status IS NULL"#,
            source_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| UnenrichedRow {
                id: r.id,
                text: r.clean_title.unwrap_or(r.name),
                category: r.category,
            })
            .collect())
    }

    /// Write back enrichment results and mark rows done (idempotent re-run).
    pub async fn apply_enrichment(&self, updates: &[EnrichUpdate]) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        for u in updates {
            sqlx::query!(
                "UPDATE vod_items
                 SET dedup_group = ?, category = COALESCE(?, category), enrich_status = 'done'
                 WHERE id = ?",
                u.dedup_group,
                u.category,
                u.id,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

fn like_query(query: Option<&str>) -> Option<String> {
    query.map(|q| format!("%{q}%"))
}
