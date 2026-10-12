mod live;
use nova_core::{now_ts, Channel, ChannelGroup, ContentSort, SourceEvidence};
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
        Self::compile_live_tx(&mut tx, None, true).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn insert_batch_tx(
        tx: &mut Transaction<'_, Sqlite>,
        entries: &[Channel],
    ) -> Result<(), StorageError> {
        let added_at = now_ts();
        for c in entries {
            let ev = &c.evidence;
            let parser_version = ev.parser_version.map(i64::from);
            let quality = SourceEvidence::csv(&ev.quality_hints);
            let language = SourceEvidence::csv(&ev.language_hints);
            let genre_source = ev.genre_source();
            sqlx::query!(
                "INSERT INTO channels (source_id, tvg_id, name, country, category, raw_group, logo_url, stream_url, added_at, channel_group, region, raw_name, parser_version, quality_hints, language_hints, collection, genre, genre_source)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                ev.raw_name,
                parser_version,
                quality,
                language,
                ev.collection,
                ev.provider_genre,
                genre_source,
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
                r#"SELECT id as "id!: i64", source_id, tvg_id, name, country, category, raw_group,
                        COALESCE(NULLIF(logo_url, ''), (SELECT l.url FROM epg_channel_logos l
                            WHERE l.source_id = channels.source_id
                              AND l.channel_key = lower(channels.tvg_id)
                            ORDER BY l.updated_at DESC LIMIT 1),
                                 (SELECT d.url FROM directory_channel_logos d
                            WHERE d.source_id = channels.source_id
                              AND d.channel_key = COALESCE(lower(NULLIF(channels.tvg_id, '')),
                                                           'name:' || lower(channels.name)))) as "logo_url?: String",
                        stream_url, is_favorite, is_in_watchlist, last_played_at, added_at, dedup_group, channel_group, region
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
                evidence: SourceEvidence::default(),
            })
            .collect(),
            // Title/year/rating orders are movie-only: natural order here.
            ContentSort::Default | ContentSort::Title | ContentSort::Year | ContentSort::Rating => sqlx::query!(
                r#"SELECT id as "id!: i64", source_id, tvg_id, name, country, category, raw_group,
                        COALESCE(NULLIF(logo_url, ''), (SELECT l.url FROM epg_channel_logos l
                            WHERE l.source_id = channels.source_id
                              AND l.channel_key = lower(channels.tvg_id)
                            ORDER BY l.updated_at DESC LIMIT 1),
                                 (SELECT d.url FROM directory_channel_logos d
                            WHERE d.source_id = channels.source_id
                              AND d.channel_key = COALESCE(lower(NULLIF(channels.tvg_id, '')),
                                                           'name:' || lower(channels.name)))) as "logo_url?: String",
                        stream_url, is_favorite, is_in_watchlist, last_played_at, added_at, dedup_group, channel_group, region
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
                evidence: SourceEvidence::default(),
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
        self.live_groups(
            source_id,
            category,
            region,
            query,
            favorites_only,
            watchlist_only,
            limit,
            offset,
        )
        .await
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
        self.ensure_live().await?;
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
        self.live_groups_with_keys(
            source_id,
            None,
            None,
            None,
            false,
            false,
            200,
            0,
            Some(keys),
        )
        .await
    }

    /// All channels of one brand-family, in display order.
    pub async fn list_in_group(
        &self,
        source_id: i64,
        group: &str,
    ) -> Result<Vec<Channel>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", source_id, tvg_id, name, country, category, raw_group,
                        COALESCE(NULLIF(logo_url, ''), (SELECT l.url FROM epg_channel_logos l
                            WHERE l.source_id = channels.source_id
                              AND l.channel_key = lower(channels.tvg_id)
                            ORDER BY l.updated_at DESC LIMIT 1),
                                 (SELECT d.url FROM directory_channel_logos d
                            WHERE d.source_id = channels.source_id
                              AND d.channel_key = COALESCE(lower(NULLIF(channels.tvg_id, '')),
                                                           'name:' || lower(channels.name)))) as "logo_url?: String",
                        stream_url, is_favorite, is_in_watchlist, last_played_at, added_at, dedup_group, channel_group, region
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
                evidence: SourceEvidence::default(),
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
            r#"SELECT id as "id!: i64", name, genre
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
                genre: r.genre,
            })
            .collect())
    }

    /// Write back enrichment results and mark rows done (idempotent re-run).
    pub async fn apply_enrichment(&self, updates: &[EnrichUpdate]) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        for u in updates {
            sqlx::query!(
                "UPDATE channels
                 SET dedup_group = ?,
                     genre = COALESCE(genre, ?),
                     genre_source = CASE WHEN genre IS NULL AND ? IS NOT NULL
                                         THEN 'inferred' ELSE genre_source END,
                     category = COALESCE(genre, ?, collection, category),
                     enrich_status = 'done'
                 WHERE id = ?",
                u.dedup_group,
                u.genre,
                u.genre,
                u.genre,
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

/// The Live section's overview of a playlist: how many channels, how many
/// have guide data (a `tvg_id` some feed has a programme on now or later
/// for), and the channel count per category.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveOverview {
    pub channels: i64,
    pub with_guide: i64,
    pub categories: Vec<crate::repositories::FacetCount>,
}

impl ChannelRepository<'_> {
    pub async fn live_overview(
        &self,
        source_id: i64,
        now_utc: i64,
    ) -> Result<LiveOverview, StorageError> {
        let totals = sqlx::query!(
            r#"SELECT COUNT(*) as "channels!: i64",
                      COALESCE(SUM(CASE WHEN c.tvg_id IS NOT NULL AND c.tvg_id <> ''
                                         AND EXISTS (SELECT 1 FROM programs p
                                                     WHERE p.channel_tvg_id = c.tvg_id
                                                       AND p.stop_utc > ?2)
                                    THEN 1 ELSE 0 END), 0) as "with_guide!: i64"
               FROM channels c WHERE c.source_id = ?1"#,
            source_id,
            now_utc,
        )
        .fetch_one(self.pool)
        .await?;
        let categories = sqlx::query!(
            r#"SELECT category as "name!: String", COUNT(*) as "count!: i64"
               FROM channels
               WHERE source_id = ? AND category IS NOT NULL AND category <> ''
               GROUP BY category
               ORDER BY COUNT(*) DESC, category ASC"#,
            source_id,
        )
        .fetch_all(self.pool)
        .await?
        .into_iter()
        .map(|r| crate::repositories::FacetCount {
            name: r.name,
            count: r.count,
        })
        .collect();
        Ok(LiveOverview {
            channels: totals.channels,
            with_guide: totals.with_guide,
            categories,
        })
    }
}

fn like_query(query: Option<&str>) -> Option<String> {
    query.map(|q| format!("%{q}%"))
}
