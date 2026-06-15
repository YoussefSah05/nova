use nova_core::{now_ts, Channel, ChannelGroup, ContentSort};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;
use crate::repositories::enrich::{EnrichUpdate, UnenrichedRow};

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
                "INSERT INTO channels (source_id, tvg_id, name, country, category, raw_group, logo_url, stream_url, added_at, channel_group, region)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                c.source_id,
                c.tvg_id,
                c.name,
                c.country,
                c.category,
                c.raw_group,
                c.logo_url,
                c.stream_url,
                added_at,
                c.channel_group,
                c.region,
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
        region: Option<&str>,
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
                r#"SELECT id as "id!: i64", source_id, tvg_id, name, country, category, raw_group, logo_url, stream_url, is_favorite, is_in_watchlist, last_played_at, added_at, dedup_group, channel_group, region
                 FROM channels
                 WHERE source_id = ?
                   AND (? IS NULL OR country = ?)
                   AND (? IS NULL OR category = ?)
                   AND (? IS NULL OR region = ?)
                   AND (? IS NULL OR name LIKE ? OR category LIKE ? OR raw_group LIKE ?)
                   AND (? = 0 OR is_favorite = 1)
                   AND (? = 0 OR is_in_watchlist = 1)
                 ORDER BY added_at DESC, id DESC LIMIT ? OFFSET ?"#,
                source_id,
                country, country,
                category, category,
                region, region,
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
                dedup_group: r.dedup_group,
                channel_group: r.channel_group,
                region: r.region,
            })
            .collect(),
            ContentSort::Default => sqlx::query!(
                r#"SELECT id as "id!: i64", source_id, tvg_id, name, country, category, raw_group, logo_url, stream_url, is_favorite, is_in_watchlist, last_played_at, added_at, dedup_group, channel_group, region
                 FROM channels
                 WHERE source_id = ?
                   AND (? IS NULL OR country = ?)
                   AND (? IS NULL OR category = ?)
                   AND (? IS NULL OR region = ?)
                   AND (? IS NULL OR name LIKE ? OR category LIKE ? OR raw_group LIKE ?)
                   AND (? = 0 OR is_favorite = 1)
                   AND (? = 0 OR is_in_watchlist = 1)
                 ORDER BY id ASC LIMIT ? OFFSET ?"#,
                source_id,
                country, country,
                category, category,
                region, region,
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
                dedup_group: r.dedup_group,
                channel_group: r.channel_group,
                region: r.region,
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
        region: Option<&str>,
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
               AND (? IS NULL OR region = ?)
               AND (? IS NULL OR name LIKE ? OR category LIKE ? OR raw_group LIKE ?)
               AND (? = 0 OR is_favorite = 1)
               AND (? = 0 OR is_in_watchlist = 1)"#,
            source_id,
            country,
            country,
            category,
            category,
            region,
            region,
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

    /// One row per brand-family (`channel_group`) for the live wall: a
    /// representative channel (lowest id with a logo, else lowest id) plus the
    /// number of channels in the family. Rows lacking a group are singletons.
    /// Filtered by type (`category`) + `region` + free-text `query`.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_groups(
        &self,
        source_id: i64,
        category: Option<&str>,
        region: Option<&str>,
        query: Option<&str>,
        favorites_only: bool,
        watchlist_only: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ChannelGroup>, StorageError> {
        let search = like_query(query);
        let search = search.as_deref();
        let fav_flag: i64 = if favorites_only { 1 } else { 0 };
        let wl_flag: i64 = if watchlist_only { 1 } else { 0 };
        // Group key: COALESCE(channel_group, name) so ungrouped channels are their
        // own singleton. Representative = lowest id; the card poster uses any
        // non-empty logo in the family. `member_count` drives the bundle badge.
        let rows = sqlx::query!(
            r#"SELECT
                 COALESCE(NULLIF(channel_group, ''), name) as "group_key!: String",
                 COUNT(*) as "member_count!: i64",
                 MIN(id) as "rep_id!: i64",
                 MIN(name) as "name!: String",
                 MIN(NULLIF(logo_url, '')) as "logo_url?: String",
                 MIN(category) as "category?: String",
                 MIN(region) as "region?: String",
                 MAX(is_favorite) as "is_favorite!: i64",
                 MAX(is_in_watchlist) as "is_in_watchlist!: i64"
               FROM channels
               WHERE source_id = ?
                 AND (? IS NULL OR category = ?)
                 AND (? IS NULL OR region = ?)
                 AND (? IS NULL OR name LIKE ? OR category LIKE ? OR raw_group LIKE ?)
                 AND (? = 0 OR is_favorite = 1)
                 AND (? = 0 OR is_in_watchlist = 1)
               GROUP BY COALESCE(NULLIF(channel_group, ''), name)
               ORDER BY COUNT(*) DESC, COALESCE(NULLIF(channel_group, ''), name) ASC
               LIMIT ? OFFSET ?"#,
            source_id,
            category,
            category,
            region,
            region,
            search,
            search,
            search,
            search,
            fav_flag,
            wl_flag,
            limit,
            offset,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ChannelGroup {
                group: r.group_key,
                member_count: r.member_count,
                representative_id: r.rep_id,
                title: r.name,
                logo_url: r.logo_url,
                category: r.category,
                region: r.region,
                is_favorite: r.is_favorite != 0,
                is_in_watchlist: r.is_in_watchlist != 0,
            })
            .collect())
    }

    /// The brand-family group key (`COALESCE(channel_group, name)`, same as
    /// `list_groups`) for each of `ids` — maps semantic-search channel hits back
    /// onto the grouped live wall. Ids passed as a JSON array so one static query
    /// handles a variable-length set (`json_each`).
    pub async fn group_keys_for_ids(
        &self,
        source_id: i64,
        ids: &[i64],
    ) -> Result<Vec<(i64, String)>, StorageError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids_json = serde_json::to_string(ids)?;
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64",
                      COALESCE(NULLIF(channel_group, ''), name) as "group_key!: String"
               FROM channels
               WHERE source_id = ? AND id IN (SELECT value FROM json_each(?))"#,
            source_id,
            ids_json,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| (r.id, r.group_key)).collect())
    }

    /// Hydrate brand-family groups by their group key (the aggregate shape of
    /// `list_groups`), for semantic hits whose group wasn't in the lexical page.
    pub async fn groups_by_keys(
        &self,
        source_id: i64,
        keys: &[String],
    ) -> Result<Vec<ChannelGroup>, StorageError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let keys_json = serde_json::to_string(keys)?;
        let rows = sqlx::query!(
            r#"SELECT
                 COALESCE(NULLIF(channel_group, ''), name) as "group_key!: String",
                 COUNT(*) as "member_count!: i64",
                 MIN(id) as "rep_id!: i64",
                 MIN(name) as "name!: String",
                 MIN(NULLIF(logo_url, '')) as "logo_url?: String",
                 MIN(category) as "category?: String",
                 MIN(region) as "region?: String",
                 MAX(is_favorite) as "is_favorite!: i64",
                 MAX(is_in_watchlist) as "is_in_watchlist!: i64"
               FROM channels
               WHERE source_id = ?
                 AND COALESCE(NULLIF(channel_group, ''), name) IN (SELECT value FROM json_each(?))
               GROUP BY COALESCE(NULLIF(channel_group, ''), name)"#,
            source_id,
            keys_json,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ChannelGroup {
                group: r.group_key,
                member_count: r.member_count,
                representative_id: r.rep_id,
                title: r.name,
                logo_url: r.logo_url,
                category: r.category,
                region: r.region,
                is_favorite: r.is_favorite != 0,
                is_in_watchlist: r.is_in_watchlist != 0,
            })
            .collect())
    }

    /// All channels of one brand-family, in display order.
    pub async fn list_in_group(
        &self,
        source_id: i64,
        group: &str,
    ) -> Result<Vec<Channel>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", source_id, tvg_id, name, country, category, raw_group, logo_url, stream_url, is_favorite, is_in_watchlist, last_played_at, added_at, dedup_group, channel_group, region
               FROM channels
               WHERE source_id = ? AND COALESCE(NULLIF(channel_group, ''), name) = ?
               ORDER BY name ASC, id ASC"#,
            source_id,
            group,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
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
                dedup_group: r.dedup_group,
                channel_group: r.channel_group,
                region: r.region,
            })
            .collect())
    }

    pub async fn regions(&self, source_id: i64) -> Result<Vec<String>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT DISTINCT region as "value!" FROM channels
               WHERE source_id = ? AND region IS NOT NULL AND region != ''
               ORDER BY region ASC"#,
            source_id
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.value).collect())
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

    /// Channels for a source that have not yet been through enrichment.
    pub async fn list_unenriched(
        &self,
        source_id: i64,
    ) -> Result<Vec<UnenrichedRow>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", name, category
               FROM channels
               WHERE source_id = ? AND enrich_status IS NULL"#,
            source_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| UnenrichedRow {
                id: r.id,
                text: r.name,
                category: r.category,
            })
            .collect())
    }

    /// Write back enrichment results and mark rows done (idempotent re-run).
    pub async fn apply_enrichment(&self, updates: &[EnrichUpdate]) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        for u in updates {
            sqlx::query!(
                "UPDATE channels
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
