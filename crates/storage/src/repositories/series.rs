use nova_core::catalog::media::episode_binding_key;
use nova_core::ProviderFacts;
use nova_core::{
    now_ts, ContentSort, EpisodeNumberingKind, SeriesEpisode, SeriesSummary, SourceEvidence,
};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;
use crate::repositories::embeddings::{from_blob, to_blob};
use crate::repositories::enrich::{SeriesTmdbQuery, SeriesTmdbWrite};

/// User flags of one series, captured before a refresh (see
/// [`SeriesRepository::snapshot_series_flags_tx`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesFlagSnapshot {
    pub series_title: String,
    pub is_favorite: bool,
    pub is_in_watchlist: bool,
    pub stream_urls: Vec<String>,
}

mod browse;

pub use browse::{SeriesFilters, SeriesStatusFilter};

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
            let ev = &e.evidence;
            let parser_version = ev.parser_version.map(i64::from);
            let quality = SourceEvidence::csv(&ev.quality_hints);
            let language = SourceEvidence::csv(&ev.language_hints);
            let numbering = e.numbering.as_str();
            let genre_source = ev.genre_source();
            let facts = ev.facts_json();
            // An Xtream catalog row (season/episode 0) speaks for the series.
            if e.season == 0
                && e.episode == 0
                && (ev.provider_facts.is_some() || ev.provider_tmdb_id.is_some())
            {
                Self::upsert_series_facts_tx(
                    tx,
                    e.source_id,
                    &e.series_title,
                    ev.provider_facts.as_ref(),
                    ev.provider_tmdb_id,
                )
                .await?;
            }
            let episode_key = episode_binding_key(&e.stream_url);
            sqlx::query!(
                "INSERT INTO series_episodes (source_id, series_title, category, season, episode, episode_title, logo_url, stream_url, added_at, raw_name, raw_group, parser_version, quality_hints, language_hints, numbering, collection, genre, genre_source, provider_facts, episode_key)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                e.source_id,
                e.series_title,
                e.category,
                e.season,
                e.episode,
                e.episode_title,
                e.logo_url,
                e.stream_url,
                added_at,
                ev.raw_name,
                ev.raw_group,
                parser_version,
                quality,
                language,
                numbering,
                ev.collection,
                ev.provider_genre,
                genre_source,
                facts,
                episode_key,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// Record a series' provider facts and claimed TMDB id. New facts win,
    /// gaps are filled from what was stored; a missing id keeps the old one.
    pub async fn upsert_series_facts_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
        series_title: &str,
        facts: Option<&ProviderFacts>,
        provider_tmdb_id: Option<i64>,
    ) -> Result<(), StorageError> {
        let stored = sqlx::query_scalar!(
            r#"SELECT facts as "facts?: String" FROM series_provider_facts
               WHERE source_id = ? AND series_title = ?"#,
            source_id,
            series_title
        )
        .fetch_optional(&mut **tx)
        .await?
        .flatten()
        .and_then(|j| ProviderFacts::from_json(&j));
        let merged = match (facts.cloned(), stored) {
            (Some(new), Some(old)) => Some(new.or(old)),
            (new, old) => new.or(old),
        };
        let json = merged
            .and_then(ProviderFacts::non_empty)
            .map(|f| f.to_json());
        let now = now_ts();
        sqlx::query!(
            "INSERT INTO series_provider_facts (source_id, series_title, facts, provider_tmdb_id, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(source_id, series_title) DO UPDATE SET
                 facts = excluded.facts,
                 provider_tmdb_id = COALESCE(excluded.provider_tmdb_id, series_provider_facts.provider_tmdb_id),
                 updated_at = excluded.updated_at",
            source_id,
            series_title,
            json,
            provider_tmdb_id,
            now,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// A series' provider facts and claimed TMDB TV id.
    pub async fn series_provider_facts(
        &self,
        source_id: i64,
        series_title: &str,
    ) -> Result<(Option<ProviderFacts>, Option<i64>), StorageError> {
        let row = sqlx::query!(
            r#"SELECT facts as "facts?: String", provider_tmdb_id as "tmdb?: i64"
               FROM series_provider_facts WHERE source_id = ? AND series_title = ?"#,
            source_id,
            series_title
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map_or((None, None), |r| {
            (
                r.facts.as_deref().and_then(ProviderFacts::from_json),
                r.tmdb,
            )
        }))
    }

    /// Provider facts of a series' episodes, by episode row id.
    pub async fn episode_provider_facts(
        &self,
        source_id: i64,
        series_title: &str,
    ) -> Result<std::collections::HashMap<i64, ProviderFacts>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", provider_facts as "facts!: String"
               FROM series_episodes
               WHERE source_id = ? AND series_title = ? AND provider_facts IS NOT NULL"#,
            source_id,
            series_title
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| ProviderFacts::from_json(&r.facts).map(|f| (r.id, f)))
            .collect())
    }

    /// Snapshot user flags of marked series before a refresh replaces the
    /// source's rows. Each entry keeps the series' stream URLs as well as its
    /// title: URLs are stable across refreshes (the Xtream catalog sentinel is
    /// `xtream://series/<id>`), titles are not when parsing rules change.
    pub async fn snapshot_series_flags_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<Vec<SeriesFlagSnapshot>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT series_title as "series_title!: String",
                      stream_url as "stream_url!: String",
                      is_favorite as "fav!: i64",
                      is_in_watchlist as "wl!: i64"
               FROM series_episodes
               WHERE source_id = ? AND series_title IN (
                   SELECT series_title FROM series_episodes
                   WHERE source_id = ?
                   GROUP BY series_title
                   HAVING MAX(is_favorite) = 1 OR MAX(is_in_watchlist) = 1)
               ORDER BY series_title"#,
            source_id,
            source_id,
        )
        .fetch_all(&mut **tx)
        .await?;
        let mut out: Vec<SeriesFlagSnapshot> = Vec::new();
        for r in rows {
            match out.last_mut() {
                Some(last) if last.series_title == r.series_title => {
                    last.is_favorite |= r.fav != 0;
                    last.is_in_watchlist |= r.wl != 0;
                    last.stream_urls.push(r.stream_url);
                }
                _ => out.push(SeriesFlagSnapshot {
                    series_title: r.series_title,
                    is_favorite: r.fav != 0,
                    is_in_watchlist: r.wl != 0,
                    stream_urls: vec![r.stream_url],
                }),
            }
        }
        Ok(out)
    }

    /// Re-apply snapshotted flags to the refreshed rows: to every series that
    /// now owns one of the snapshot's stream URLs (even if its parsed title
    /// changed), or else still carries the old title. Returns the
    /// `(old_title, new_title)` renames detected by URL, so the catalog
    /// compiler can move the series' identity instead of copying its state.
    pub async fn apply_series_flags_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
        flags: &[SeriesFlagSnapshot],
    ) -> Result<Vec<(String, String)>, StorageError> {
        let mut renames = Vec::new();
        for snap in flags {
            let f = i64::from(snap.is_favorite);
            let w = i64::from(snap.is_in_watchlist);
            let mut titles = vec![snap.series_title.clone()];
            for url in &snap.stream_urls {
                if let Some(title) = sqlx::query_scalar!(
                    r#"SELECT series_title as "t!: String" FROM series_episodes
                       WHERE source_id = ? AND stream_url = ? LIMIT 1"#,
                    source_id,
                    url,
                )
                .fetch_optional(&mut **tx)
                .await?
                {
                    if !titles.contains(&title) {
                        renames.push((snap.series_title.clone(), title.clone()));
                        titles.push(title);
                    }
                }
            }
            for title in titles {
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
        }
        Ok(renames)
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
    /// `series_facts` / `series_tmdb_id` come from the same `get_series_info`
    /// response and refresh the series-level facts.
    pub async fn replace_series_episodes(
        &self,
        source_id: i64,
        series_title: &str,
        episodes: &[SeriesEpisode],
        series_facts: Option<&ProviderFacts>,
        series_tmdb_id: Option<i64>,
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
        // Series-level provenance (collection, genre and its source) is kept
        // across the replacement, like the user flags.
        let taxonomy = sqlx::query!(
            r#"SELECT MAX(collection) as "collection?: String", MAX(genre) as "genre?: String",
                      MAX(genre_source) as "genre_source?: String"
               FROM series_episodes WHERE source_id = ? AND series_title = ?"#,
            source_id,
            series_title,
        )
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query!(
            "DELETE FROM series_episodes WHERE source_id = ? AND series_title = ?",
            source_id,
            series_title,
        )
        .execute(&mut *tx)
        .await?;

        if series_facts.is_some() || series_tmdb_id.is_some() {
            Self::upsert_series_facts_tx(
                &mut tx,
                source_id,
                series_title,
                series_facts,
                series_tmdb_id,
            )
            .await?;
        }
        let added_at = now_ts();
        for e in episodes {
            let numbering = e.numbering.as_str();
            let facts = e.evidence.facts_json();
            let episode_key = episode_binding_key(&e.stream_url);
            sqlx::query!(
                "INSERT INTO series_episodes (source_id, series_title, category, season, episode, episode_title, logo_url, stream_url, added_at, is_favorite, is_in_watchlist, raw_name, numbering, collection, genre, genre_source, provider_facts, episode_key)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                e.evidence.raw_name,
                numbering,
                taxonomy.collection,
                taxonomy.genre,
                taxonomy.genre_source,
                facts,
                episode_key,
            )
            .execute(&mut *tx)
            .await?;
        }
        // Episode history parked by a refresh comes back with its episodes.
        crate::WatchHistoryRepository::reattach_pending_for_source_tx(&mut tx, source_id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Returns one summary per unique series title, with its card facts.
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
        let filters = SeriesFilters {
            category,
            query,
            favorites_only,
            watchlist_only,
            ..Default::default()
        };
        self.browse_series(source_id, &filters, sort, limit, offset)
            .await
    }

    pub async fn list_episodes(
        &self,
        source_id: i64,
        series_title: &str,
    ) -> Result<Vec<SeriesEpisode>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", source_id, series_title, category, season, episode, episode_title, logo_url, stream_url, added_at, numbering
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
                numbering: EpisodeNumberingKind::from_db(&r.numbering),
                evidence: SourceEvidence::default(),
            })
            .collect())
    }

    /// A single episode by id. Used by the player to resolve the series an
    /// episode belongs to (for the cinema-mode episode browser / autoplay).
    pub async fn get_episode(&self, id: i64) -> Result<Option<SeriesEpisode>, StorageError> {
        let row = sqlx::query!(
            r#"SELECT id as "id!: i64", source_id, series_title, category, season, episode, episode_title, logo_url, stream_url, added_at, numbering
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
            numbering: EpisodeNumberingKind::from_db(&r.numbering),
            evidence: SourceEvidence::default(),
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
        let filters = SeriesFilters {
            category,
            query,
            favorites_only,
            watchlist_only,
            ..Default::default()
        };
        self.count_browse_series(source_id, &filters).await
    }

    /// Toggle favourite for a series. The canonical series entity owns the
    /// flag (it survives refreshes and provider renames); every row of the
    /// series is updated as the compatibility mirror.
    pub async fn set_favorite(
        &self,
        source_id: i64,
        series_title: &str,
        is_favorite: bool,
    ) -> Result<(), StorageError> {
        let flag: i64 = if is_favorite { 1 } else { 0 };
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            r#"UPDATE canonical_entities SET is_favorite = ?
               WHERE kind = 'series' AND id IN (
                   SELECT entity_id FROM series_bindings
                   WHERE source_id = ? AND series_title = ?)"#,
            flag,
            source_id,
            series_title,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE series_episodes SET is_favorite = ? WHERE source_id = ? AND series_title = ?",
            flag,
            source_id,
            series_title,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Toggle watchlist for a series; entity-owned, mirrored like
    /// [`Self::set_favorite`].
    pub async fn set_in_watchlist(
        &self,
        source_id: i64,
        series_title: &str,
        in_watchlist: bool,
    ) -> Result<(), StorageError> {
        let flag: i64 = if in_watchlist { 1 } else { 0 };
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            r#"UPDATE canonical_entities SET is_in_watchlist = ?
               WHERE kind = 'series' AND id IN (
                   SELECT entity_id FROM series_bindings
                   WHERE source_id = ? AND series_title = ?)"#,
            flag,
            source_id,
            series_title,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE series_episodes SET is_in_watchlist = ? WHERE source_id = ? AND series_title = ?",
            flag,
            source_id,
            series_title,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
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

    /// Series of a source whose canonical entity needs a TMDB decision:
    /// never matched, decided by an older matcher or in another metadata
    /// language, left unresolved before `retry_unresolved_before`, or left
    /// unresolved by search while the provider now claims a TMDB id.
    pub async fn list_for_tmdb(
        &self,
        source_id: i64,
        matcher_version: u32,
        retry_unresolved_before: i64,
        language: &str,
    ) -> Result<Vec<SeriesTmdbQuery>, StorageError> {
        let version = i64::from(matcher_version);
        let rows = sqlx::query!(
            r#"SELECT b.series_title as "series_title!: String",
                      b.entity_id as "entity_id!: String",
                      spf.provider_tmdb_id as "trusted?: i64"
               FROM series_bindings b
               LEFT JOIN entity_external_matches m
                 ON m.entity_id = b.entity_id AND m.namespace = 'tmdb:tv'
               LEFT JOIN series_provider_facts spf
                 ON spf.source_id = b.source_id AND spf.series_title = b.series_title
               WHERE b.source_id = ? AND b.series_title IS NOT NULL
                 AND (m.entity_id IS NULL
                      -- a provider id is new evidence: retry an unresolved series now
                      OR (m.status = 'unresolved' AND spf.provider_tmdb_id IS NOT NULL
                          AND m.method = 'search')
                      OR m.matcher_version < ?
                      OR (m.status = 'unresolved' AND m.checked_at < ?)
                      OR (m.status = 'matched' AND COALESCE(m.title_language, 'en-US') <> ?))
               ORDER BY b.series_title"#,
            source_id,
            version,
            retry_unresolved_before,
            language,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| SeriesTmdbQuery {
                series_title: r.series_title,
                entity_id: Some(r.entity_id),
                trusted_tmdb_id: r.trusted.filter(|id| *id > 0),
            })
            .collect())
    }

    /// Record TMDB decisions for series. The series' canonical entity owns
    /// the decision (`entity_external_matches`, namespace `tmdb:tv`, with its
    /// provenance), so it survives refreshes and provider renames; the
    /// per-source `series_meta` row and the episodes' genre are its mirror.
    /// Decisions carry their series entity from the queue, so an import that
    /// renames the title while TMDB is queried can't redirect them; a write
    /// without one resolves the title's current binding, and is skipped when
    /// no entity holds that title.
    pub async fn apply_tmdb(
        &self,
        source_id: i64,
        updates: &[SeriesTmdbWrite],
    ) -> Result<(), StorageError> {
        // Take the write lock up front: a read-first deferred transaction
        // overlapping an import would fail with SQLITE_BUSY instead of
        // waiting out the busy timeout.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let now = nova_core::now_ts();
        for u in updates {
            let entity = match &u.entity_id {
                // The series the decision was made for, whatever its title now.
                Some(id) => {
                    sqlx::query_scalar!(
                        r#"SELECT id as "id!: String" FROM canonical_entities
                       WHERE id = ? AND kind = 'series' AND state = 'active'"#,
                        id,
                    )
                    .fetch_optional(&mut *tx)
                    .await?
                }
                None => {
                    sqlx::query_scalar!(
                        r#"SELECT entity_id as "entity_id!: String" FROM series_bindings
                           WHERE source_id = ? AND series_title = ?"#,
                        source_id,
                        u.series_title,
                    )
                    .fetch_optional(&mut *tx)
                    .await?
                }
            };
            let Some(entity) = entity else {
                continue;
            };
            // Matched means a TMDB id; anything else is unresolved.
            let external_id = u.tmdb_id.filter(|_| u.matched).map(|id| id.to_string());
            let matched = external_id.is_some();
            let status = if matched { "matched" } else { "unresolved" };
            let method = if u.method == "trusted_id" {
                "trusted_id"
            } else {
                "search"
            };
            let confidence = u.confidence_bp.map(i64::from);
            let version = i64::from(u.matcher_version);
            let title = u.canonical_title.as_deref().filter(|_| matched);
            let primary = u.primary_genre.as_deref().filter(|_| matched);
            sqlx::query!(
                r#"INSERT INTO entity_external_matches
                       (entity_id, namespace, status, external_id, method, confidence_bp,
                        matcher_version, evidence_json, canonical_title, poster_url,
                        backdrop_url, overview, genres, primary_genre, checked_at,
                        original_title, title_language)
                   VALUES (?, 'tmdb:tv', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(entity_id, namespace) DO UPDATE SET
                       status = excluded.status,
                       external_id = excluded.external_id,
                       method = excluded.method,
                       confidence_bp = excluded.confidence_bp,
                       matcher_version = excluded.matcher_version,
                       evidence_json = excluded.evidence_json,
                       canonical_title = excluded.canonical_title,
                       poster_url = excluded.poster_url,
                       backdrop_url = excluded.backdrop_url,
                       overview = excluded.overview,
                       genres = excluded.genres,
                       primary_genre = excluded.primary_genre,
                       checked_at = excluded.checked_at,
                       original_title = excluded.original_title,
                       title_language = excluded.title_language"#,
                entity,
                status,
                external_id,
                method,
                confidence,
                version,
                u.evidence_json,
                title,
                u.poster_url,
                u.backdrop_url,
                u.overview,
                u.genres_csv,
                primary,
                now,
                u.original_title,
                u.title_language,
            )
            .execute(&mut *tx)
            .await?;
            crate::ExternalMatchRepository::mirror_series_tx(
                &mut tx,
                crate::repositories::external_matches::SeriesMirrorScope::Entity(&entity),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Distinct series of a source not yet embedded for `model`, with their TMDB
    /// overview when available — the semantic-search embed worklist. Idempotent:
    /// re-running enrichment only embeds new titles.
    pub async fn list_for_embedding(
        &self,
        source_id: i64,
        model: &str,
    ) -> Result<Vec<(String, Option<String>)>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT se.series_title as "series_title!: String",
                      COALESCE(MAX(sm.overview),
                                 (SELECT json_extract(f.facts, '$.plot') FROM series_provider_facts f
                                  WHERE f.source_id = se.source_id AND f.series_title = se.series_title))
                            as "overview?: String"
               FROM series_episodes se
               LEFT JOIN series_meta sm
                 ON sm.source_id = se.source_id AND sm.series_title = se.series_title
               WHERE se.source_id = ?
                 AND NOT EXISTS (
                   SELECT 1 FROM series_embeddings emb
                   WHERE emb.source_id = se.source_id
                     AND emb.series_title = se.series_title
                     AND emb.model = ?
                 )
               GROUP BY se.series_title"#,
            source_id,
            model,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.series_title, r.overview))
            .collect())
    }

    /// Persist series embedding vectors for `model` (one transaction).
    pub async fn apply_embeddings(
        &self,
        source_id: i64,
        model: &str,
        rows: &[(String, Vec<f32>)],
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        for (title, vector) in rows {
            let blob = to_blob(vector);
            sqlx::query!(
                "INSERT OR REPLACE INTO series_embeddings (source_id, series_title, model, vector)
                 VALUES (?, ?, ?, ?)",
                source_id,
                title,
                model,
                blob,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// How many series vectors exist for a source + model (cache-invalidation
    /// signal for the in-memory index).
    pub async fn count_embeddings(&self, source_id: i64, model: &str) -> Result<i64, StorageError> {
        let row = sqlx::query!(
            r#"SELECT COUNT(*) as "n!: i64" FROM series_embeddings
               WHERE source_id = ? AND model = ?"#,
            source_id,
            model,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(row.n)
    }

    /// Load series vectors for a source + model as `(series_title, vector)`.
    pub async fn load_embeddings(
        &self,
        source_id: i64,
        model: &str,
    ) -> Result<Vec<(String, Vec<f32>)>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT series_title as "series_title!: String", vector as "vector!: Vec<u8>"
               FROM series_embeddings WHERE source_id = ? AND model = ?"#,
            source_id,
            model,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.series_title, from_blob(&r.vector)))
            .collect())
    }

    /// Hydrate series summaries by title (any order) — for semantic hits.
    pub async fn list_by_titles(
        &self,
        source_id: i64,
        titles: &[String],
    ) -> Result<Vec<SeriesSummary>, StorageError> {
        self.cards_by_titles(source_id, titles).await
    }
}

fn like_query(query: Option<&str>) -> Option<String> {
    query.map(|q| format!("%{q}%"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{init_profile_db, SeriesTmdbWrite, SourceRepository};

    fn episode(source_id: i64, title: &str) -> SeriesEpisode {
        SeriesEpisode {
            id: 0,
            source_id,
            series_title: title.into(),
            category: None,
            season: 1,
            episode: 1,
            episode_title: None,
            logo_url: None,
            stream_url: format!("http://host/series/{title}.mkv"),
            added_at: None,
            numbering: EpisodeNumberingKind::SeasonEpisode,
            evidence: SourceEvidence::default(),
        }
    }

    #[tokio::test]
    async fn series_display_titles_follow_the_naming_policy() {
        let dir = tempfile::tempdir().unwrap();
        let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
            .await
            .unwrap();
        let source = SourceRepository::new(&pool)
            .create("xtream", "t", "http://host/get.php")
            .await
            .unwrap()
            .id;
        let repo = SeriesRepository::new(&pool);
        repo.insert_batch(&[episode(source, "BREAKING BAD"), episode(source, "Dark")])
            .await
            .unwrap();
        // As an import does: bind the series to canonical entities.
        let mut tx = pool.begin().await.unwrap();
        crate::CanonicalRepository::compile_series_tx(&mut tx, source)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        repo.apply_tmdb(
            source,
            &[SeriesTmdbWrite {
                tmdb_id: Some(1396),
                series_title: "BREAKING BAD".into(),
                canonical_title: Some("Breaking Bad".into()),
                original_title: Some("Breaking Bad".into()),
                title_language: Some("en-US".into()),
                matched: true,
                method: "search",
                matcher_version: 2,
                evidence_json: "{}".into(),
                ..SeriesTmdbWrite::default()
            }],
        )
        .await
        .unwrap();
        assert_eq!(
            crate::TitleArtRepository::new(&pool)
                .series_tmdb_id(source, "BREAKING BAD")
                .await
                .unwrap(),
            Some(1396),
            "a series match keeps its TMDB id"
        );
        let list = repo
            .list_series(
                source,
                None,
                None,
                false,
                false,
                ContentSort::Default,
                10,
                0,
            )
            .await
            .unwrap();
        let titles: Vec<(&str, &str)> = list
            .iter()
            .map(|s| (s.series_title.as_str(), s.display_title.as_str()))
            .collect();
        assert!(
            titles.contains(&("BREAKING BAD", "Breaking Bad")),
            "confident TMDB title"
        );
        assert!(
            titles.contains(&("Dark", "Dark")),
            "unmatched: parsed title"
        );
    }
}
