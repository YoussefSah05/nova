//! Live projections and resolver on the existing channel repository.
use super::ChannelRepository;
use crate::StorageError;
use nova_core::live::{
    self, LiveChannelSummary, LiveEvidence, LiveIdentity, LivePlaybackTarget, LiveStreamVariant,
};
use nova_core::{now_ts, ChannelGroup};
use sqlx::{Sqlite, Transaction};
use std::collections::BTreeMap;

impl ChannelRepository<'_> {
    pub async fn ensure_live(&self) -> Result<(), StorageError> {
        let version = live::LIVE_RULESET;
        let pending = sqlx::query!("SELECT COUNT(*) as \"n!: i64\" FROM channels WHERE live_ruleset IS NULL OR live_ruleset <> ?", version)
            .fetch_one(self.pool).await?.n;
        if pending == 0 {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        Self::compile_live_tx(&mut tx, None, false).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn compile_live_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source: Option<i64>,
        force: bool,
    ) -> Result<(), StorageError> {
        let version = live::LIVE_RULESET;
        let pending = sqlx::query!("SELECT COUNT(*) as \"n!: i64\" FROM channels WHERE (? IS NULL OR source_id = ?) AND (live_ruleset IS NULL OR live_ruleset <> ?)", source, source, version)
            .fetch_one(&mut **tx).await?.n;
        if !force && pending == 0 {
            return Ok(());
        }
        let rows = sqlx::query!("SELECT id, source_id, COALESCE(raw_name, name) as \"raw!: String\", country, raw_group, tvg_id, stream_url FROM channels WHERE (? IS NULL OR source_id = ?) ORDER BY source_id, id", source, source)
            .fetch_all(&mut **tx).await?;
        let mut sources = BTreeMap::new();
        for row in &rows {
            sources
                .entry(row.source_id)
                .or_insert_with(Vec::new)
                .push(row);
        }
        for members in sources.values() {
            let evidence: Vec<_> = members
                .iter()
                .map(|r| LiveEvidence {
                    raw_name: r.raw.clone(),
                    country: r.country.clone(),
                    group: r.raw_group.clone(),
                    tvg_id: r.tvg_id.clone(),
                })
                .collect();
            for (row, meta) in members.iter().zip(live::compile(&evidence)) {
                // Stable Xtream stream ID when present; otherwise the protected locator digest.
                let tail = row
                    .stream_url
                    .split('?')
                    .next()
                    .unwrap_or("")
                    .rsplit('/')
                    .next()
                    .unwrap_or("");
                let id = tail.split('.').next().unwrap_or("");
                let locator = if row.stream_url.contains("/live/")
                    && !id.is_empty()
                    && id.bytes().all(|b| b.is_ascii_digit())
                {
                    format!("xtream:live:{id}")
                } else {
                    live::digest(&["locator", &row.stream_url])
                };
                let variant = live::digest(&["live-variant-v1", &locator]);
                let metadata = serde_json::to_string(&meta)?;
                sqlx::query!("UPDATE channels SET channel_group = ?, live_channel_key = ?, live_variant_key = ?, live_metadata = ?, live_ruleset = ?, live_edition = ?, live_sort_key = ? WHERE id = ?", meta.family, meta.channel_key, variant, metadata, version, meta.edition, meta.sort_key, row.id)
                    .execute(&mut **tx).await?;
            }
        }
        Ok(())
    }

    /// Indexed SQL limits the selected scope before decoding its metadata.
    #[allow(clippy::too_many_arguments)]
    pub async fn live_channels(
        &self,
        source: i64,
        family: Option<&str>,
        category: Option<&str>,
        region: Option<&str>,
        country: Option<&str>,
        query: Option<&str>,
        favorites: bool,
        watchlist: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LiveChannelSummary>, StorageError> {
        self.live_channels_by_key(
            source, family, category, region, country, query, favorites, watchlist, limit, offset,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn live_channels_by_key(
        &self,
        source: i64,
        family: Option<&str>,
        category: Option<&str>,
        region: Option<&str>,
        country: Option<&str>,
        query: Option<&str>,
        favorites: bool,
        watchlist: bool,
        limit: i64,
        offset: i64,
        key: Option<&str>,
    ) -> Result<Vec<LiveChannelSummary>, StorageError> {
        self.ensure_live().await?;
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        let q = search_tokens(query)?;
        let fav = i64::from(favorites);
        let wl = i64::from(watchlist);
        // Filters choose identities; every variant of those identities is returned.
        // The page of keys is materialized once and drives the join through
        // the identity index; a plain CTE was re-run for every channel row.
        let rows = sqlx::query!(r#"WITH selected AS MATERIALIZED (
            SELECT DISTINCT live_channel_key FROM channels
            WHERE source_id = ? AND (? IS NULL OR channel_group = ?)
              AND (? IS NULL OR live_channel_key = ?)
              AND (? IS NULL OR category = ?) AND (? IS NULL OR region = ?)
              AND (? IS NULL OR country = ?)
              AND (? IS NULL OR NOT EXISTS (SELECT 1 FROM json_each(?) t WHERE
                CASE WHEN t.value GLOB '[0-9]*' AND t.value NOT GLOB '*[^0-9]*'
                  THEN COALESCE(json_extract(live_metadata, '$.number'), -1) != CAST(t.value AS INTEGER)
                    AND instr(' ' || json_extract(live_metadata, '$.title') || ' ', ' ' || t.value || ' ') = 0
                  ELSE instr(lower(COALESCE(raw_name, name) || ' ' || channel_group || ' ' || COALESCE(raw_group, '') || ' ' || COALESCE(json_extract(live_metadata, '$.edition'), '') || ' ' || COALESCE(json_extract(live_metadata, '$.quality'), '')), lower(t.value)) = 0
                END))
              AND (? = 0 OR is_favorite = 1) AND (? = 0 OR is_in_watchlist = 1)
            ORDER BY live_edition, live_sort_key, live_channel_key LIMIT ? OFFSET ?
        ) SELECT c.id as "id!: i64", c.source_id, c.live_channel_key as "key!: String",
            c.live_variant_key as "variant!: String", c.live_metadata as "metadata!: String",
            COALESCE(c.raw_name, c.name) as "raw!: String", c.logo_url, c.category, c.region,
            c.is_favorite, c.is_in_watchlist, h.last_success_at, h.last_failure_at,
            p.variant_key as "preferred?: String"
          FROM selected s CROSS JOIN channels c
            ON c.source_id = ? AND c.live_channel_key = s.live_channel_key
          LEFT JOIN live_variant_health h ON h.source_id = c.source_id AND h.variant_key = c.live_variant_key
          LEFT JOIN live_channel_preferences p ON p.source_id = c.source_id AND p.channel_key = c.live_channel_key
          ORDER BY c.id"#,
          source, family, family, key, key, category, category, region, region, country, country,
          q, q, fav, wl, limit, offset, source)
          .fetch_all(self.pool).await?;
        let mut channels: BTreeMap<String, (LiveChannelSummary, Option<String>)> = BTreeMap::new();
        for row in rows {
            let meta: LiveIdentity = serde_json::from_str(&row.metadata)?;
            let (summary, _) = channels.entry(row.key.clone()).or_insert_with(|| {
                (
                    LiveChannelSummary {
                        key: row.key,
                        source_id: source,
                        family: meta.family.clone(),
                        title: meta.title.clone(),
                        edition: meta.edition,
                        language: meta.language,
                        country: meta.country,
                        number: meta.number,
                        representative_id: row.id,
                        logo_url: row.logo_url.clone(),
                        category: row.category.clone(),
                        region: row.region.clone(),
                        is_favorite: false,
                        is_in_watchlist: false,
                        variants: Vec::new(),
                    },
                    row.preferred,
                )
            });
            summary.is_favorite |= row.is_favorite != 0;
            summary.is_in_watchlist |= row.is_in_watchlist != 0;
            if summary.logo_url.is_none() {
                summary.logo_url = row.logo_url;
            }
            summary.variants.push(LiveStreamVariant {
                variant_key: row.variant,
                channel_id: row.id,
                raw_name: row.raw,
                quality: meta.quality,
                codecs: meta.codecs,
                recently_failed: row
                    .last_failure_at
                    .is_some_and(|f| row.last_success_at.is_none_or(|s| f >= s)),
                last_success_at: row.last_success_at,
            });
        }
        let mut out: Vec<_> = channels
            .into_values()
            .map(|(mut c, preferred)| {
                c.variants.sort_by_key(|v| {
                    (
                        v.recently_failed,
                        preferred.as_ref() != Some(&v.variant_key),
                        live::quality_rank(v.quality.as_deref()),
                        v.channel_id,
                    )
                });
                if let Some(v) = c.variants.first() {
                    c.representative_id = v.channel_id;
                }
                c
            })
            .collect();
        out.sort_by(|a, b| {
            a.edition
                .cmp(&b.edition)
                .then_with(|| natural_cmp(&a.title, &b.title))
                .then(a.key.cmp(&b.key))
        });
        Ok(out)
    }

    pub async fn live_channel_for_row(
        &self,
        id: i64,
    ) -> Result<Option<LiveChannelSummary>, StorageError> {
        self.ensure_live().await?;
        let row = sqlx::query!(
            "SELECT source_id, channel_group, live_channel_key FROM channels WHERE id = ?",
            id
        )
        .fetch_optional(self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        // Same family only, never the entire provider catalog in playback.
        Ok(self
            .live_channels_by_key(
                row.source_id,
                row.channel_group.as_deref(),
                None,
                None,
                None,
                None,
                false,
                false,
                1,
                0,
                row.live_channel_key.as_deref(),
            )
            .await?
            .into_iter()
            .find(|c| Some(&c.key) == row.live_channel_key.as_ref()))
    }

    pub async fn resolve_live(
        &self,
        source: i64,
        key: &str,
        explicit: Option<&str>,
        excluded: &[String],
    ) -> Result<LivePlaybackTarget, StorageError> {
        self.ensure_live().await?;
        let family = sqlx::query!("SELECT channel_group FROM channels WHERE source_id = ? AND live_channel_key = ? LIMIT 1", source, key).fetch_optional(self.pool).await?.ok_or(StorageError::NotFound)?;
        let channel = self
            .live_channels_by_key(
                source,
                family.channel_group.as_deref(),
                None,
                None,
                None,
                None,
                false,
                false,
                1,
                0,
                Some(key),
            )
            .await?
            .into_iter()
            .find(|c| c.key == key)
            .ok_or(StorageError::NotFound)?;
        let variant = if let Some(explicit) = explicit {
            channel.variants.iter().find(|v| v.variant_key == explicit)
        } else {
            channel
                .variants
                .iter()
                .find(|v| !v.recently_failed && !excluded.contains(&v.variant_key))
        }
        .ok_or(StorageError::NotFound)?;
        Ok(LivePlaybackTarget {
            channel_id: variant.channel_id,
            variant_key: variant.variant_key.clone(),
            channel,
        })
    }

    pub async fn record_live_outcome(&self, id: i64, success: bool) -> Result<(), StorageError> {
        let row = sqlx::query!(
            "SELECT source_id, live_channel_key, live_variant_key FROM channels WHERE id = ?",
            id
        )
        .fetch_optional(self.pool)
        .await?;
        let Some(row) = row else { return Ok(()) };
        let (Some(key), Some(variant)) = (row.live_channel_key, row.live_variant_key) else {
            return Ok(());
        };
        self.record_live_variant(row.source_id, &key, &variant, success)
            .await
    }

    pub async fn record_live_variant(
        &self,
        source: i64,
        key: &str,
        variant: &str,
        success: bool,
    ) -> Result<(), StorageError> {
        let now = now_ts();
        let mut tx = self.pool.begin().await?;
        let ok = success.then_some(now);
        let failed = (!success).then_some(now);
        sqlx::query!("INSERT INTO live_variant_health (source_id, variant_key, last_success_at, last_failure_at) VALUES (?, ?, ?, ?) ON CONFLICT(source_id, variant_key) DO UPDATE SET last_success_at = COALESCE(excluded.last_success_at, last_success_at), last_failure_at = COALESCE(excluded.last_failure_at, last_failure_at)", source, variant, ok, failed).execute(&mut *tx).await?;
        if success {
            sqlx::query!("INSERT INTO live_channel_preferences (source_id, channel_key, variant_key) VALUES (?, ?, ?) ON CONFLICT(source_id, channel_key) DO UPDATE SET variant_key = excluded.variant_key", source,key,variant).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Match families before aggregation so search keeps complete counts.
    #[allow(clippy::too_many_arguments)]
    pub async fn live_groups(
        &self,
        source: i64,
        category: Option<&str>,
        region: Option<&str>,
        query: Option<&str>,
        favorites: bool,
        watchlist: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ChannelGroup>, StorageError> {
        self.live_groups_with_keys(
            source, category, region, query, favorites, watchlist, limit, offset, None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn live_groups_with_keys(
        &self,
        source: i64,
        category: Option<&str>,
        region: Option<&str>,
        query: Option<&str>,
        favorites: bool,
        watchlist: bool,
        limit: i64,
        offset: i64,
        keys: Option<&[String]>,
    ) -> Result<Vec<ChannelGroup>, StorageError> {
        self.ensure_live().await?;
        let q = search_tokens(query)?;
        let fav = i64::from(favorites);
        let wl = i64::from(watchlist);
        let keys = keys.map(serde_json::to_string).transpose()?;
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        let rows = sqlx::query!(r#"SELECT c.channel_group as "group!: String",
            COUNT(DISTINCT c.live_channel_key) as "count!: i64", COUNT(*) as "streams!: i64",
            MIN(c.id) as "id!: i64", MIN(json_extract(c.live_metadata, '$.familyTitle')) as "title!: String",
            json_group_array(DISTINCT json_extract(c.live_metadata, '$.edition')) as "editions!: String",
            COALESCE(MIN(NULLIF(c.logo_url, '')), MIN((SELECT url FROM epg_channel_logos l WHERE l.source_id = c.source_id AND l.channel_key = lower(c.tvg_id) ORDER BY updated_at DESC LIMIT 1)),
            MIN((SELECT url FROM directory_channel_logos d WHERE d.source_id = c.source_id AND d.channel_key = COALESCE(lower(NULLIF(c.tvg_id, '')), 'name:' || lower(c.name))))) as "logo?: String",
            MIN(c.category) as "category?: String", MIN(c.region) as "region?: String",
            MAX(c.is_favorite) as "fav!: i64", MAX(c.is_in_watchlist) as "wl!: i64"
          FROM channels c WHERE c.source_id = ?
            AND (? IS NULL OR c.channel_group IN (SELECT value FROM json_each(?)))
            AND c.channel_group IN (SELECT channel_group FROM channels WHERE source_id = ?
              AND (? IS NULL OR category = ?) AND (? IS NULL OR region = ?)
              AND (? IS NULL OR NOT EXISTS (SELECT 1 FROM json_each(?) t WHERE
                CASE WHEN t.value GLOB '[0-9]*' AND t.value NOT GLOB '*[^0-9]*'
                  THEN COALESCE(json_extract(live_metadata, '$.number'), -1) != CAST(t.value AS INTEGER)
                    AND instr(' ' || json_extract(live_metadata, '$.title') || ' ', ' ' || t.value || ' ') = 0
                  ELSE instr(lower(COALESCE(raw_name, name) || ' ' || channel_group || ' ' || COALESCE(raw_group, '') || ' ' || COALESCE(json_extract(live_metadata, '$.edition'), '') || ' ' || COALESCE(json_extract(live_metadata, '$.quality'), '')), lower(t.value)) = 0
                END))
              AND (? = 0 OR is_favorite = 1) AND (? = 0 OR is_in_watchlist = 1))
          GROUP BY c.channel_group ORDER BY COUNT(DISTINCT c.live_channel_key) DESC, c.channel_group LIMIT ? OFFSET ?"#,
          source, keys, keys, source, category, category, region, region, q, q, fav, wl, limit, offset)
          .fetch_all(self.pool).await?;
        rows.into_iter()
            .map(|r| {
                let mut editions: Vec<String> = serde_json::from_str(&r.editions)?;
                editions.sort();
                Ok(ChannelGroup {
                    group: r.group,
                    title: r.title,
                    member_count: r.count,
                    stream_count: r.streams,
                    editions,
                    representative_id: r.id,
                    logo_url: r.logo,
                    category: r.category,
                    region: r.region,
                    is_favorite: r.fav != 0,
                    is_in_watchlist: r.wl != 0,
                })
            })
            .collect()
    }
}
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let chunks = |s: &str| {
        let mut out = Vec::<String>::new();
        let mut digit = None;
        for c in s.chars() {
            let d = c.is_ascii_digit();
            if digit != Some(d) {
                out.push(String::new());
                digit = Some(d);
            }
            if let Some(last) = out.last_mut() {
                last.push(c.to_ascii_lowercase());
            }
        }
        out
    };
    for (a, b) in chunks(a).iter().zip(chunks(b).iter()) {
        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            _ => a.cmp(b),
        };
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

fn search_tokens(query: Option<&str>) -> Result<Option<String>, StorageError> {
    query
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            let mut tokens = Vec::new();
            for word in s
                .split(|c: char| !c.is_alphanumeric())
                .filter(|s| !s.is_empty())
            {
                match word.to_lowercase().as_str() {
                    "beinsport" | "beinsports" => {
                        tokens.extend(["bein".to_string(), "sports".to_string()])
                    }
                    _ => tokens.push(word.to_lowercase()),
                }
            }
            serde_json::to_string(&tokens).map_err(Into::into)
        })
        .transpose()
}
