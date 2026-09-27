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
            sqlx::query!(
                "INSERT INTO series_episodes (source_id, series_title, category, season, episode, episode_title, logo_url, stream_url, added_at, raw_name, raw_group, parser_version, quality_hints, language_hints, numbering, collection, genre, genre_source)
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
                ev.raw_name,
                ev.raw_group,
                parser_version,
                quality,
                language,
                numbering,
                ev.collection,
                ev.provider_genre,
                genre_source,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
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
    /// changed), or else still carries the old title.
    pub async fn apply_series_flags_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
        flags: &[SeriesFlagSnapshot],
    ) -> Result<(), StorageError> {
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

        let added_at = now_ts();
        for e in episodes {
            let numbering = e.numbering.as_str();
            sqlx::query!(
                "INSERT INTO series_episodes (source_id, series_title, category, season, episode, episode_title, logo_url, stream_url, added_at, is_favorite, is_in_watchlist, raw_name, numbering, collection, genre, genre_source)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            )
            .execute(&mut *tx)
            .await?;
        }
        // Episode history parked by a refresh comes back with its episodes.
        crate::WatchHistoryRepository::reattach_pending_for_source_tx(&mut tx, source_id).await?;
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
                r#"SELECT se.series_title as "series_title!: String",
                        MIN(se.category) as "category?: String",
                        SUM(CASE WHEN se.season > 0 THEN 1 ELSE 0 END) as "episode_count!: i64",
                        COUNT(DISTINCT CASE WHEN se.season > 0 THEN se.season END) as "season_count!: i64",
                        COALESCE(MAX(sm.poster_url), MIN(se.logo_url)) as "poster_url?: String",
                        MAX(se.is_favorite) as "is_favorite!: i64",
                        MAX(se.is_in_watchlist) as "is_in_watchlist!: i64",
                        MAX(se.added_at) as "added_at?: i64",
                        MAX(sm.canonical_title) as "canonical_title?: String",
                        MAX(sm.backdrop_url) as "backdrop_url?: String",
                        MAX(sm.overview) as "overview?: String"
                 FROM series_episodes se
                 LEFT JOIN series_meta sm
                   ON sm.source_id = se.source_id AND sm.series_title = se.series_title
                 WHERE se.source_id = ?
                   AND (? IS NULL OR se.category = ?)
                   AND (? IS NULL OR se.series_title LIKE ? OR sm.canonical_title LIKE ?
                        OR sm.original_title LIKE ?)
                 GROUP BY se.series_title
                 HAVING (? = 0 OR MAX(se.is_favorite) = 1)
                    AND (? = 0 OR MAX(se.is_in_watchlist) = 1)
                 ORDER BY MAX(se.added_at) DESC, se.series_title ASC
                 LIMIT ? OFFSET ?"#,
                source_id,
                category,
                category,
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
            .await?
            .into_iter()
            .map(|r| {
                SeriesSummary {
                    series_title: r.series_title,
                    source_id,
                    category: r.category,
                    episode_count: r.episode_count,
                    season_count: r.season_count,
                    poster_url: r.poster_url,
                    is_favorite: r.is_favorite != 0,
                    is_in_watchlist: r.is_in_watchlist != 0,
                    added_at: r.added_at,
                    canonical_title: r.canonical_title,
                    backdrop_url: r.backdrop_url,
                    overview: r.overview,
                    display_title: String::new(),
                }
                .with_display_title()
            })
            .collect(),
            ContentSort::Default => sqlx::query!(
                r#"SELECT se.series_title as "series_title!: String",
                        MIN(se.category) as "category?: String",
                        SUM(CASE WHEN se.season > 0 THEN 1 ELSE 0 END) as "episode_count!: i64",
                        COUNT(DISTINCT CASE WHEN se.season > 0 THEN se.season END) as "season_count!: i64",
                        COALESCE(MAX(sm.poster_url), MIN(se.logo_url)) as "poster_url?: String",
                        MAX(se.is_favorite) as "is_favorite!: i64",
                        MAX(se.is_in_watchlist) as "is_in_watchlist!: i64",
                        MAX(se.added_at) as "added_at?: i64",
                        MAX(sm.canonical_title) as "canonical_title?: String",
                        MAX(sm.backdrop_url) as "backdrop_url?: String",
                        MAX(sm.overview) as "overview?: String"
                 FROM series_episodes se
                 LEFT JOIN series_meta sm
                   ON sm.source_id = se.source_id AND sm.series_title = se.series_title
                 WHERE se.source_id = ?
                   AND (? IS NULL OR se.category = ?)
                   AND (? IS NULL OR se.series_title LIKE ? OR sm.canonical_title LIKE ?
                        OR sm.original_title LIKE ?)
                 GROUP BY se.series_title
                 HAVING (? = 0 OR MAX(se.is_favorite) = 1)
                    AND (? = 0 OR MAX(se.is_in_watchlist) = 1)
                 ORDER BY se.series_title ASC
                 LIMIT ? OFFSET ?"#,
                source_id,
                category,
                category,
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
            .await?
            .into_iter()
            .map(|r| {
                SeriesSummary {
                    series_title: r.series_title,
                    source_id,
                    category: r.category,
                    episode_count: r.episode_count,
                    season_count: r.season_count,
                    poster_url: r.poster_url,
                    is_favorite: r.is_favorite != 0,
                    is_in_watchlist: r.is_in_watchlist != 0,
                    added_at: r.added_at,
                    canonical_title: r.canonical_title,
                    backdrop_url: r.backdrop_url,
                    overview: r.overview,
                    display_title: String::new(),
                }
                .with_display_title()
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
                  AND (? IS NULL OR series_title LIKE ? OR EXISTS (
                      SELECT 1 FROM series_meta sm
                      WHERE sm.source_id = series_episodes.source_id
                        AND sm.series_title = series_episodes.series_title
                        AND (sm.canonical_title LIKE ? OR sm.original_title LIKE ?)))
                GROUP BY series_title
                HAVING (? = 0 OR MAX(is_favorite) = 1)
                   AND (? = 0 OR MAX(is_in_watchlist) = 1)
              )"#,
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

    /// Distinct series of a source needing a TMDB decision: never matched,
    /// decided by an older matcher, or left unresolved before
    /// `retry_unresolved_before`.
    pub async fn list_for_tmdb(
        &self,
        source_id: i64,
        matcher_version: u32,
        retry_unresolved_before: i64,
        language: &str,
    ) -> Result<Vec<SeriesTmdbQuery>, StorageError> {
        let version = i64::from(matcher_version);
        let rows = sqlx::query!(
            r#"SELECT DISTINCT se.series_title as "series_title!: String"
               FROM series_episodes se
               LEFT JOIN series_meta sm
                 ON sm.source_id = se.source_id AND sm.series_title = se.series_title
               WHERE se.source_id = ?
                 AND (sm.tmdb_status IS NULL
                      OR sm.matcher_version IS NULL
                      OR sm.matcher_version < ?
                      OR (sm.tmdb_status = 'unmatched' AND sm.checked_at < ?)
                      OR (sm.tmdb_status = 'matched'
                          AND (sm.tmdb_id IS NULL
                               OR COALESCE(sm.title_language, 'en-US') <> ?)))"#,
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
            })
            .collect())
    }

    /// Upsert TMDB metadata for series into `series_meta` (matched or unmatched).
    pub async fn apply_tmdb(
        &self,
        source_id: i64,
        updates: &[SeriesTmdbWrite],
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        for u in updates {
            let status = if u.matched { "matched" } else { "unmatched" };
            let confidence = u.confidence_bp.map(i64::from);
            let version = i64::from(u.matcher_version);
            let now = nova_core::now_ts();
            sqlx::query!(
                "INSERT INTO series_meta
                   (source_id, series_title, canonical_title, poster_url, backdrop_url, overview,
                    tmdb_genres, tmdb_status, match_method, confidence_bp, matcher_version,
                    evidence_json, checked_at, original_title, title_language, tmdb_id)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(source_id, series_title) DO UPDATE SET
                   tmdb_id = excluded.tmdb_id,
                   canonical_title = excluded.canonical_title,
                   poster_url = excluded.poster_url,
                   backdrop_url = excluded.backdrop_url,
                   overview = excluded.overview,
                   tmdb_genres = excluded.tmdb_genres,
                   tmdb_status = excluded.tmdb_status,
                   match_method = excluded.match_method,
                   confidence_bp = excluded.confidence_bp,
                   matcher_version = excluded.matcher_version,
                   evidence_json = excluded.evidence_json,
                   checked_at = excluded.checked_at,
                   original_title = excluded.original_title,
                   title_language = excluded.title_language",
                source_id,
                u.series_title,
                u.canonical_title,
                u.poster_url,
                u.backdrop_url,
                u.overview,
                u.genres_csv,
                status,
                u.method,
                confidence,
                version,
                u.evidence_json,
                now,
                u.original_title,
                u.title_language,
                u.tmdb_id,
            )
            .execute(&mut *tx)
            .await?;

            // Same genre precedence as movies: a confident TMDB genre wins, an
            // unresolved decision removes only a TMDB genre, and the browse
            // facet `category` is recomputed as COALESCE(genre, collection).
            let genre = u.primary_genre.as_deref();
            let matched_flag: i64 = u.matched.into();
            sqlx::query!(
                "UPDATE series_episodes
                 SET genre = CASE WHEN ? IS NOT NULL THEN ?
                                  WHEN ? = 0 AND genre_source = 'tmdb' THEN NULL
                                  ELSE genre END,
                     genre_source = CASE WHEN ? IS NOT NULL THEN 'tmdb'
                                         WHEN ? = 0 AND genre_source = 'tmdb' THEN NULL
                                         ELSE genre_source END,
                     category = COALESCE(CASE WHEN ? IS NOT NULL THEN ?
                                              WHEN ? = 0 AND genre_source = 'tmdb' THEN NULL
                                              ELSE genre END,
                                         collection, category)
                 WHERE source_id = ? AND series_title = ?",
                genre,
                genre,
                matched_flag,
                genre,
                matched_flag,
                genre,
                genre,
                matched_flag,
                source_id,
                u.series_title,
            )
            .execute(&mut *tx)
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
                      MAX(sm.overview) as "overview?: String"
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

    /// Hydrate series summaries by title (any order) — for semantic hits. Titles
    /// passed as a JSON array so one static query handles a variable-length set.
    pub async fn list_by_titles(
        &self,
        source_id: i64,
        titles: &[String],
    ) -> Result<Vec<SeriesSummary>, StorageError> {
        if titles.is_empty() {
            return Ok(Vec::new());
        }
        let titles_json = serde_json::to_string(titles)?;
        let rows = sqlx::query!(
            r#"SELECT se.series_title as "series_title!: String",
                    MIN(se.category) as "category?: String",
                    SUM(CASE WHEN se.season > 0 THEN 1 ELSE 0 END) as "episode_count!: i64",
                    COUNT(DISTINCT CASE WHEN se.season > 0 THEN se.season END) as "season_count!: i64",
                    COALESCE(MAX(sm.poster_url), MIN(se.logo_url)) as "poster_url?: String",
                    MAX(se.is_favorite) as "is_favorite!: i64",
                    MAX(se.is_in_watchlist) as "is_in_watchlist!: i64",
                    MAX(se.added_at) as "added_at?: i64",
                    MAX(sm.canonical_title) as "canonical_title?: String",
                    MAX(sm.backdrop_url) as "backdrop_url?: String",
                    MAX(sm.overview) as "overview?: String"
             FROM series_episodes se
             LEFT JOIN series_meta sm
               ON sm.source_id = se.source_id AND sm.series_title = se.series_title
             WHERE se.source_id = ?
               AND se.series_title IN (SELECT value FROM json_each(?))
             GROUP BY se.series_title"#,
            source_id,
            titles_json,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                SeriesSummary {
                    series_title: r.series_title,
                    source_id,
                    category: r.category,
                    episode_count: r.episode_count,
                    season_count: r.season_count,
                    poster_url: r.poster_url,
                    is_favorite: r.is_favorite != 0,
                    is_in_watchlist: r.is_in_watchlist != 0,
                    added_at: r.added_at,
                    canonical_title: r.canonical_title,
                    backdrop_url: r.backdrop_url,
                    overview: r.overview,
                    display_title: String::new(),
                }
                .with_display_title()
            })
            .collect())
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
