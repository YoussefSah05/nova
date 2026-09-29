//! Series browse reads: one card per series title in the playlist, with the
//! card facts (genres, rating, status, years, next episode) derived from
//! stored evidence — the provider's facts, the TMDB match and TMDB facts
//! fetched earlier — and the profile's progress (watched episodes, last
//! watched), never from the network. Every list, count, title lookup and the
//! week's schedule go through the same filters.
//!
//! The query groups the playlist's episodes per title (cheap columns only),
//! filters and cuts the page, then computes the per-series aggregates and
//! progress for the page's titles alone. With a category, a card describes
//! the episodes in that category. Progress is the series entity's episode
//! state (by durable episode key), so it survives provider URL changes.
//! Status and TMDB years exist only for series whose facts were fetched.

use nova_core::{ContentSort, NextEpisode, SeriesFacts, SeriesSummary};

use super::SeriesRepository;
use crate::error::StorageError;
use crate::repositories::FacetCount;

/// Where a series stands, by TMDB's status and schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesStatusFilter {
    /// Still making episodes: a scheduled episode, or a returning /
    /// in-production status.
    Airing,
    /// Ended or cancelled.
    Ended,
}

impl SeriesStatusFilter {
    fn code(self) -> &'static str {
        match self {
            Self::Airing => "airing",
            Self::Ended => "ended",
        }
    }
}

/// Filters of the series lists. `genre` is Nova's primary genre of the
/// series; `airs_between` keeps series whose next episode airs within the
/// inclusive `YYYY-MM-DD` dates.
#[derive(Debug, Clone, Copy, Default)]
pub struct SeriesFilters<'a> {
    pub category: Option<&'a str>,
    pub query: Option<&'a str>,
    pub favorites_only: bool,
    pub watchlist_only: bool,
    pub genre: Option<&'a str>,
    pub status: Option<SeriesStatusFilter>,
    pub airs_between: Option<(&'a str, &'a str)>,
}

/// Order codes shared by the page query (`5` is internal: next air date).
fn sort_code(sort: ContentSort) -> i64 {
    match sort {
        ContentSort::Default => 0,
        ContentSort::RecentlyAdded => 1,
        ContentSort::Title => 2,
        ContentSort::Year => 3,
        ContentSort::Rating => 4,
    }
}
const BY_NEXT_AIR_DATE: i64 = 5;

fn genres(tmdb_genres: Option<String>, genre: Option<String>) -> Vec<String> {
    tmdb_genres
        .filter(|g| !g.trim().is_empty())
        .map(|g| {
            g.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .or_else(|| genre.filter(|g| !g.is_empty()).map(|g| vec![g]))
        .unwrap_or_default()
}

impl SeriesRepository<'_> {
    /// One card per series title matching `filters`, in `sort` order.
    pub async fn browse_series(
        &self,
        source_id: i64,
        filters: &SeriesFilters<'_>,
        sort: ContentSort,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SeriesSummary>, StorageError> {
        self.page(source_id, filters, None, sort_code(sort), limit, offset)
            .await
    }

    /// How many series match `filters`.
    pub async fn count_browse_series(
        &self,
        source_id: i64,
        filters: &SeriesFilters<'_>,
    ) -> Result<i64, StorageError> {
        let search = super::like_query(filters.query);
        let fav: i64 = filters.favorites_only.into();
        let wl: i64 = filters.watchlist_only.into();
        let status = filters.status.map(SeriesStatusFilter::code);
        let (airs_from, airs_to) = filters.airs_between.unzip();
        let n = sqlx::query_scalar!(
            r#"WITH g AS (
                 SELECT se.series_title AS series_title, MAX(se.genre) AS genre,
                        MAX(se.is_favorite) AS fav, MAX(se.is_in_watchlist) AS wl
                 FROM series_episodes se
                 WHERE se.source_id = ?1 AND (?2 IS NULL OR se.category = ?2)
                 GROUP BY se.series_title
               )
               SELECT COUNT(*) as "n!: i64"
               FROM g
               LEFT JOIN series_meta sm
                      ON sm.source_id = ?1 AND sm.series_title = g.series_title
               LEFT JOIN tmdb_title_facts f
                      ON f.media_type = 'tv' AND f.tmdb_id = sm.tmdb_id
               WHERE (?3 IS NULL OR g.series_title LIKE ?3 OR sm.canonical_title LIKE ?3
                      OR sm.original_title LIKE ?3)
                 AND (?4 = 0 OR g.fav = 1)
                 AND (?5 = 0 OR g.wl = 1)
                 AND (?6 IS NULL OR g.genre = ?6)
                 AND (?7 IS NULL
                      OR (?7 = 'airing' AND (json_extract(f.facts, '$.nextEpisode.airDate') >= date('now', '-1 day')
                          OR json_extract(f.facts, '$.status') IN
                             ('Returning Series', 'In Production', 'Planned', 'Pilot')))
                      OR (?7 = 'ended' AND COALESCE(json_extract(f.facts, '$.nextEpisode.airDate') < date('now', '-1 day'), 1)
                          AND json_extract(f.facts, '$.status') IN ('Ended', 'Canceled')))
                 AND (?8 IS NULL
                      OR json_extract(f.facts, '$.nextEpisode.airDate') BETWEEN ?8 AND ?9)"#,
            source_id,
            filters.category,
            search,
            fav,
            wl,
            filters.genre,
            status,
            airs_from,
            airs_to,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(n)
    }

    /// Series whose next episode airs between the inclusive `YYYY-MM-DD`
    /// dates, soonest first.
    pub async fn series_schedule(
        &self,
        source_id: i64,
        from: &str,
        to: &str,
        limit: i64,
    ) -> Result<Vec<SeriesSummary>, StorageError> {
        let filters = SeriesFilters {
            airs_between: Some((from, to)),
            ..Default::default()
        };
        self.page(source_id, &filters, None, BY_NEXT_AIR_DATE, limit, 0)
            .await
    }

    /// Cards for the given titles (any order).
    pub(super) async fn cards_by_titles(
        &self,
        source_id: i64,
        titles: &[String],
    ) -> Result<Vec<SeriesSummary>, StorageError> {
        if titles.is_empty() {
            return Ok(Vec::new());
        }
        let json = serde_json::to_string(titles)?;
        let limit = titles.len() as i64;
        self.page(
            source_id,
            &SeriesFilters::default(),
            Some(json),
            0,
            limit,
            0,
        )
        .await
    }

    /// Nova's primary genres among the playlist's series, most common first.
    pub async fn series_genres(&self, source_id: i64) -> Result<Vec<FacetCount>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT genre as "name!: String", COUNT(*) as "count!: i64"
               FROM (SELECT MAX(genre) AS genre FROM series_episodes
                     WHERE source_id = ? GROUP BY series_title)
               WHERE genre IS NOT NULL AND genre <> ''
               GROUP BY genre
               ORDER BY COUNT(*) DESC, genre ASC"#,
            source_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| FacetCount {
                name: r.name,
                count: r.count,
            })
            .collect())
    }

    async fn page(
        &self,
        source_id: i64,
        filters: &SeriesFilters<'_>,
        titles: Option<String>,
        sort: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SeriesSummary>, StorageError> {
        let search = super::like_query(filters.query);
        let fav: i64 = filters.favorites_only.into();
        let wl: i64 = filters.watchlist_only.into();
        let status = filters.status.map(SeriesStatusFilter::code);
        let (airs_from, airs_to) = filters.airs_between.unzip();
        let rows = sqlx::query!(
            r#"WITH g AS (
                 SELECT se.series_title AS series_title, MAX(se.added_at) AS added_at,
                        MAX(se.genre) AS genre,
                        MAX(se.is_favorite) AS fav, MAX(se.is_in_watchlist) AS wl
                 FROM series_episodes se
                 WHERE se.source_id = ?1 AND (?2 IS NULL OR se.category = ?2)
                 GROUP BY se.series_title
               ),
               page AS MATERIALIZED (
                 SELECT g.series_title AS series_title, g.genre AS genre,
                        sm.canonical_title AS canonical_title, sm.poster_url AS poster_url,
                        sm.backdrop_url AS backdrop_url, sm.overview AS overview,
                        sm.tmdb_genres AS tmdb_genres, f.facts AS facts, pf.facts AS provider_facts,
                        COALESCE(json_extract(f.facts, '$.firstYear'),
                                 CAST(NULLIF(substr(json_extract(pf.facts, '$.releaseDate'), 1, 4), '') AS INTEGER))
                            AS first_year,
                        COALESCE(NULLIF(json_extract(f.facts, '$.rating'), 0),
                                 NULLIF(json_extract(pf.facts, '$.rating'), 0)) AS rating,
                        CASE WHEN NULLIF(json_extract(f.facts, '$.rating'), 0) IS NOT NULL THEN 'tmdb'
                             WHEN NULLIF(json_extract(pf.facts, '$.rating'), 0) IS NOT NULL THEN 'provider'
                        END AS rating_source,
                        ROW_NUMBER() OVER (
                          ORDER BY CASE WHEN ?11 = 1 THEN g.added_at END DESC,
                                   CASE WHEN ?11 = 2 THEN COALESCE(sm.canonical_title, g.series_title) END
                                        COLLATE NOCASE ASC,
                                   CASE WHEN ?11 = 3 THEN COALESCE(json_extract(f.facts, '$.firstYear'),
                                        CAST(NULLIF(substr(json_extract(pf.facts, '$.releaseDate'), 1, 4), '') AS INTEGER)) END DESC,
                                   CASE WHEN ?11 = 4 THEN COALESCE(NULLIF(json_extract(f.facts, '$.rating'), 0),
                                                                   NULLIF(json_extract(pf.facts, '$.rating'), 0)) END DESC,
                                   CASE WHEN ?11 = 5 THEN json_extract(f.facts, '$.nextEpisode.airDate') END ASC,
                                   g.series_title ASC
                        ) AS position
                 FROM g
                 LEFT JOIN series_meta sm
                        ON sm.source_id = ?1 AND sm.series_title = g.series_title
                 LEFT JOIN tmdb_title_facts f
                        ON f.media_type = 'tv' AND f.tmdb_id = sm.tmdb_id
                 LEFT JOIN series_provider_facts pf
                        ON pf.source_id = ?1 AND pf.series_title = g.series_title
                 WHERE (?3 IS NULL OR g.series_title LIKE ?3 OR sm.canonical_title LIKE ?3
                        OR sm.original_title LIKE ?3)
                   AND (?4 = 0 OR g.fav = 1)
                   AND (?5 = 0 OR g.wl = 1)
                   AND (?6 IS NULL OR g.genre = ?6)
                   AND (?7 IS NULL
                        OR (?7 = 'airing' AND (json_extract(f.facts, '$.nextEpisode.airDate') >= date('now', '-1 day')
                            OR json_extract(f.facts, '$.status') IN
                               ('Returning Series', 'In Production', 'Planned', 'Pilot')))
                        OR (?7 = 'ended' AND COALESCE(json_extract(f.facts, '$.nextEpisode.airDate') < date('now', '-1 day'), 1)
                            AND json_extract(f.facts, '$.status') IN ('Ended', 'Canceled')))
                   AND (?8 IS NULL
                        OR json_extract(f.facts, '$.nextEpisode.airDate') BETWEEN ?8 AND ?9)
                   AND (?10 IS NULL OR g.series_title IN (SELECT value FROM json_each(?10)))
                 ORDER BY position
                 LIMIT ?12 OFFSET ?13
               )
               SELECT p.series_title as "series_title!: String",
                      MIN(se.category) as "category?: String",
                      COUNT(DISTINCT CASE WHEN se.season > 0
                                          THEN COALESCE(se.episode_key, 'row:' || se.id) END)
                          as "episode_count!: i64",
                      COUNT(DISTINCT CASE WHEN se.season > 0 THEN se.season END) as "season_count!: i64",
                      COALESCE(p.poster_url, MIN(se.logo_url)) as "poster_url?: String",
                      MAX(se.is_favorite) as "is_favorite!: i64",
                      MAX(se.is_in_watchlist) as "is_in_watchlist!: i64",
                      MAX(se.added_at) as "added_at?: i64",
                      p.canonical_title as "canonical_title?: String",
                      MAX(b.entity_id) as "entity_id?: String",
                      COALESCE(p.backdrop_url, json_extract(p.provider_facts, '$.backdropUrl'))
                          as "backdrop_url?: String",
                      COALESCE(p.overview, json_extract(p.provider_facts, '$.plot'))
                          as "overview?: String",
                      p.tmdb_genres as "tmdb_genres?: String",
                      p.genre as "genre?: String",
                      p.rating as "rating?: f64",
                      p.rating_source as "rating_source?: String",
                      json_extract(p.facts, '$.status') as "status?: String",
                      p.first_year as "first_year?: i64",
                      json_extract(p.facts, '$.lastYear') as "last_year?: i64",
                      json_extract(p.facts, '$.nextEpisode.season') as "next_season?: i64",
                      json_extract(p.facts, '$.nextEpisode.episode') as "next_episode?: i64",
                      json_extract(p.facts, '$.nextEpisode.name') as "next_name?: String",
                      json_extract(p.facts, '$.nextEpisode.airDate') as "next_air_date?: String",
                      COUNT(DISTINCT CASE WHEN se.season > 0 AND w.completed = 1
                                          THEN se.episode_key END) as "watched_count!: i64",
                      MAX(w.watched_at) as "last_watched_at?: i64"
               FROM page p
               JOIN series_episodes se
                 ON se.source_id = ?1 AND se.series_title = p.series_title
                AND (?2 IS NULL OR se.category = ?2)
               LEFT JOIN series_bindings b
                      ON b.source_id = ?1 AND b.series_title = p.series_title
               LEFT JOIN episode_watch_state w
                      ON w.series_entity_id = b.entity_id AND w.episode_key = se.episode_key
               GROUP BY p.series_title
               ORDER BY MIN(p.position)"#,
            source_id,
            filters.category,
            search,
            fav,
            wl,
            filters.genre,
            status,
            airs_from,
            airs_to,
            titles,
            sort,
            limit,
            offset,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let rating = r.rating.filter(|x| *x > 0.0).map(|x| x as f32);
                let next_episode = match (r.next_season, r.next_episode, r.next_air_date) {
                    (Some(season), Some(episode), Some(air_date)) => Some(NextEpisode {
                        season,
                        episode,
                        name: r.next_name,
                        air_date,
                    }),
                    _ => None,
                };
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
                    entity_id: r.entity_id,
                    facts: SeriesFacts {
                        genres: genres(r.tmdb_genres, r.genre),
                        rating,
                        rating_source: rating.and(r.rating_source),
                        status: r.status,
                        first_year: r.first_year.and_then(|y| i32::try_from(y).ok()),
                        last_year: r.last_year.and_then(|y| i32::try_from(y).ok()),
                        next_episode,
                        watched_count: r.watched_count,
                        last_watched_at: r.last_watched_at,
                    },
                }
                .with_display_title()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{init_profile_db, SeriesTmdbWrite, SourceRepository, TitleArtMedia};
    use nova_core::{EpisodeNumberingKind, ProviderFacts, SeriesEpisode, SourceEvidence};
    use sqlx::SqlitePool;

    fn episode(source_id: i64, title: &str, season: i64, number: i64) -> SeriesEpisode {
        SeriesEpisode {
            id: 0,
            source_id,
            series_title: title.into(),
            category: None,
            season,
            episode: number,
            episode_title: None,
            logo_url: None,
            stream_url: format!("http://host/files/{title}/s{season}e{number}.mkv"),
            added_at: None,
            numbering: EpisodeNumberingKind::SeasonEpisode,
            evidence: SourceEvidence::default(),
        }
    }

    /// Three shows: Breaking Bad (TMDB-matched, airing, one episode
    /// watched), Dark (matched, ended, rated only by the provider) and
    /// Fleabag (unmatched, no facts).
    async fn fixture() -> (tempfile::TempDir, SqlitePool, i64) {
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
        repo.insert_batch(&[
            episode(source, "BREAKING BAD", 1, 1),
            episode(source, "BREAKING BAD", 1, 2),
            episode(source, "BREAKING BAD", 2, 1),
            episode(source, "Dark", 1, 1),
            episode(source, "Dark", 1, 2),
            episode(source, "Fleabag", 1, 1),
        ])
        .await
        .unwrap();
        let mut tx = pool.begin().await.unwrap();
        crate::CanonicalRepository::compile_series_tx(&mut tx, source)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let matched = |title: &str, tmdb_id: i64, name: &str, genres: &str| SeriesTmdbWrite {
            tmdb_id: Some(tmdb_id),
            series_title: title.into(),
            canonical_title: Some(name.into()),
            genres_csv: Some(genres.into()),
            matched: true,
            method: "search",
            matcher_version: 2,
            evidence_json: "{}".into(),
            ..SeriesTmdbWrite::default()
        };
        repo.apply_tmdb(
            source,
            &[
                matched("BREAKING BAD", 1396, "Breaking Bad", "Drama, Crime"),
                matched("Dark", 70523, "Dark", "Mystery"),
            ],
        )
        .await
        .unwrap();
        for (title, genre) in [
            ("BREAKING BAD", "Crime"),
            ("Dark", "Mystery"),
            ("Fleabag", "Comedy"),
        ] {
            sqlx::query("UPDATE series_episodes SET genre = ? WHERE series_title = ?")
                .bind(genre)
                .bind(title)
                .execute(&pool)
                .await
                .unwrap();
        }
        // TMDB facts as `TitleFacts` serializes them (camelCase; the key
        // names are pinned by the desktop crate's facts tests).
        let facts = crate::TitleFactsRepository::new(&pool);
        facts
            .put(
                TitleArtMedia::Tv,
                1396,
                "en-US",
                r#"{"rating":9.0,"status":"Returning Series","firstYear":2008,"lastYear":2013,
                    "nextEpisode":{"season":3,"episode":1,"name":"Return","airDate":"2099-10-01"}}"#,
                1,
            )
            .await
            .unwrap();
        facts
            .put(
                TitleArtMedia::Tv,
                70523,
                "en-US",
                r#"{"rating":0,"status":"Ended","firstYear":2017,"lastYear":2020}"#,
                1,
            )
            .await
            .unwrap();
        let mut tx = pool.begin().await.unwrap();
        SeriesRepository::upsert_series_facts_tx(
            &mut tx,
            source,
            "Dark",
            Some(&ProviderFacts {
                rating: Some(8.0),
                ..Default::default()
            }),
            None,
        )
        .await
        .unwrap();
        // Fleabag has no TMDB facts: its year comes from the provider.
        SeriesRepository::upsert_series_facts_tx(
            &mut tx,
            source,
            "Fleabag",
            Some(&ProviderFacts {
                release_date: Some("2016-07-21".into()),
                ..Default::default()
            }),
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        // The profile finished Breaking Bad's first episode.
        let first: i64 = sqlx::query_scalar(
            "SELECT id FROM series_episodes
             WHERE series_title = 'BREAKING BAD' AND season = 1 AND episode = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        crate::WatchHistoryRepository::new(&pool)
            .record_progress("episode", first, 2900.0, Some(2900.0), true, None)
            .await
            .unwrap();
        (dir, pool, source)
    }

    fn titles(list: &[SeriesSummary]) -> Vec<&str> {
        list.iter().map(|s| s.display_title.as_str()).collect()
    }

    #[tokio::test]
    async fn cards_carry_facts_and_progress_from_stored_evidence() {
        let (_dir, pool, source) = fixture().await;
        let repo = SeriesRepository::new(&pool);
        let all = repo
            .browse_series(
                source,
                &SeriesFilters::default(),
                ContentSort::Default,
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(titles(&all), ["Breaking Bad", "Dark", "Fleabag"]);
        let bb = &all[0];
        assert_eq!(bb.episode_count, 3);
        let f = &bb.facts;
        assert_eq!(f.genres, ["Drama", "Crime"], "TMDB genres win");
        assert_eq!(
            (f.rating, f.rating_source.as_deref()),
            (Some(9.0), Some("tmdb"))
        );
        assert_eq!(f.status.as_deref(), Some("Returning Series"));
        assert_eq!((f.first_year, f.last_year), (Some(2008), Some(2013)));
        assert_eq!(
            f.next_episode,
            Some(NextEpisode {
                season: 3,
                episode: 1,
                name: Some("Return".into()),
                air_date: "2099-10-01".into(),
            })
        );
        assert_eq!(f.watched_count, 1);
        assert!(f.last_watched_at.is_some());
        let dark = &all[1].facts;
        assert_eq!(
            (dark.rating, dark.rating_source.as_deref()),
            (Some(8.0), Some("provider")),
            "a zero TMDB rating is unrated: the provider's rating shows"
        );
        let fleabag = &all[2].facts;
        assert_eq!(fleabag.genres, ["Comedy"], "Nova's genre without a match");
        assert_eq!((fleabag.rating, fleabag.watched_count), (None, 0));
        assert_eq!(
            fleabag.first_year,
            Some(2016),
            "the provider's release year"
        );
        // Title lookups (semantic hits) carry the same facts.
        let hit = repo.list_by_titles(source, &["Dark".into()]).await.unwrap();
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].facts.rating, Some(8.0));
    }

    #[tokio::test]
    async fn series_filter_by_genre_and_status_and_sort() {
        let (_dir, pool, source) = fixture().await;
        let repo = SeriesRepository::new(&pool);
        let list = |filters: SeriesFilters<'static>, sort: ContentSort| {
            let repo = SeriesRepository::new(&pool);
            async move {
                let cards = repo
                    .browse_series(source, &filters, sort, 10, 0)
                    .await
                    .unwrap();
                let count = repo.count_browse_series(source, &filters).await.unwrap();
                assert_eq!(count, cards.len() as i64, "count agrees with the list");
                cards
                    .into_iter()
                    .map(|s| s.display_title)
                    .collect::<Vec<_>>()
            }
        };
        let genre = SeriesFilters {
            genre: Some("Comedy"),
            ..Default::default()
        };
        assert_eq!(list(genre, ContentSort::Default).await, ["Fleabag"]);
        let airing = SeriesFilters {
            status: Some(SeriesStatusFilter::Airing),
            ..Default::default()
        };
        assert_eq!(list(airing, ContentSort::Default).await, ["Breaking Bad"]);
        let ended = SeriesFilters {
            status: Some(SeriesStatusFilter::Ended),
            ..Default::default()
        };
        assert_eq!(list(ended, ContentSort::Default).await, ["Dark"]);
        assert_eq!(
            list(SeriesFilters::default(), ContentSort::Rating).await,
            ["Breaking Bad", "Dark", "Fleabag"]
        );
        assert_eq!(
            list(SeriesFilters::default(), ContentSort::Year).await,
            ["Dark", "Fleabag", "Breaking Bad"],
            "the provider's release year when TMDB has none"
        );
        let search = SeriesFilters {
            query: Some("breaking"),
            ..Default::default()
        };
        assert_eq!(list(search, ContentSort::Default).await, ["Breaking Bad"]);
        let genres = repo.series_genres(source).await.unwrap();
        assert_eq!(
            genres
                .iter()
                .map(|g| (g.name.as_str(), g.count))
                .collect::<Vec<_>>(),
            [("Comedy", 1), ("Crime", 1), ("Mystery", 1)]
        );
    }

    #[tokio::test]
    async fn the_schedule_lists_next_episodes_in_the_window() {
        let (_dir, pool, source) = fixture().await;
        let repo = SeriesRepository::new(&pool);
        let week = repo
            .series_schedule(source, "2099-09-29", "2099-10-05", 60)
            .await
            .unwrap();
        assert_eq!(titles(&week), ["Breaking Bad"]);
        let later = repo
            .series_schedule(source, "2099-10-02", "2099-10-08", 60)
            .await
            .unwrap();
        assert!(
            later.is_empty(),
            "an episode before the window is not listed"
        );
    }

    #[tokio::test]
    async fn progress_follows_the_episode_not_its_stream_url() {
        let (_dir, pool, source) = fixture().await;
        // The provider moves the episode to a new URL; its durable key stays
        // and the row-keyed history mirror is gone.
        sqlx::query(
            "UPDATE series_episodes SET stream_url = 'http://new-host/bb/1/1.mp4'
             WHERE series_title = 'BREAKING BAD' AND season = 1 AND episode = 1",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM watch_history")
            .execute(&pool)
            .await
            .unwrap();
        let cards = SeriesRepository::new(&pool)
            .list_by_titles(source, &["BREAKING BAD".into()])
            .await
            .unwrap();
        assert_eq!(cards[0].facts.watched_count, 1);
    }

    #[tokio::test]
    async fn a_category_card_describes_the_episodes_in_it() {
        let (_dir, pool, source) = fixture().await;
        sqlx::query(
            "UPDATE series_episodes SET category = 'Classics'
             WHERE series_title = 'BREAKING BAD' AND season = 1",
        )
        .execute(&pool)
        .await
        .unwrap();
        let filters = SeriesFilters {
            category: Some("Classics"),
            ..Default::default()
        };
        let cards = SeriesRepository::new(&pool)
            .browse_series(source, &filters, ContentSort::Default, 10, 0)
            .await
            .unwrap();
        assert_eq!(titles(&cards), ["Breaking Bad"]);
        assert_eq!(
            (
                cards[0].category.as_deref(),
                cards[0].episode_count,
                cards[0].season_count
            ),
            (Some("Classics"), 2, 1)
        );
    }
}
