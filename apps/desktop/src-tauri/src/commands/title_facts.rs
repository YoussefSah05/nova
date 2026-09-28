//! Rich TMDB facts on demand (master plan M4.6): a title's facts
//! (runtime, rating, certification, cast, trailer, seasons…) and its season
//! episode lists, resolved when its page opens.
//!
//! Same contract as title art: stored facts are served while current (same
//! metadata language and `FACTS_VERSION`, younger than a TTL); otherwise one
//! cached TMDB request refreshes them when the user enabled TMDB. Offline, or
//! on any error, the stored facts (possibly stale) are returned, so a page
//! never fails because TMDB is unreachable.

use std::collections::HashMap;

use nova_core::catalog::EntityId;
use nova_core::{ContinueWatchingItem, EpisodeNumberingKind, ProviderFacts, SeriesEpisode};
use nova_enrich::tmdb_facts::{
    parse_movie_facts, parse_season, parse_tv_facts, CastEntry, CollectionRef, SeasonSummary,
    TitleFacts, TmdbEpisode, UpcomingEpisode, FACTS_VERSION,
};
use nova_enrich::TmdbMediaKind;
use nova_storage::{
    CanonicalRepository, SeriesRepository, SourceRepository, StoredFacts, TitleArtMedia,
    TitleFactsRepository, TmdbCacheRepository, VodRepository, WatchHistoryRepository,
};
use serde::Serialize;
use tauri::State;

use super::enrich::{CachedTmdb, TmdbFetch, TmdbRequest};
use super::require_profile_pool;
use super::title_art::{tmdb_access, tmdb_work, TitleArtSubject};
use crate::ipc::ApiError;
use crate::state::AppState;

const DAY: i64 = 24 * 3600;

/// How long stored facts stay current: a week for a show still airing
/// (ratings, next episode), a month otherwise.
pub(crate) fn facts_ttl(facts: &TitleFacts) -> i64 {
    if airing(facts) {
        7 * DAY
    } else {
        30 * DAY
    }
}

/// How long a season's episode list stays current: a week while the show
/// airs, three months once it has ended.
pub(crate) fn season_ttl(series: Option<&TitleFacts>) -> i64 {
    if series.is_some_and(|f| !airing(f)) {
        90 * DAY
    } else {
        7 * DAY
    }
}

fn airing(facts: &TitleFacts) -> bool {
    facts.next_episode.is_some()
        || matches!(
            facts.status.as_deref(),
            Some("Returning Series" | "In Production" | "Planned" | "Pilot")
        )
}

fn current(stored: &StoredFacts, language: &str, ttl: i64, now: i64) -> bool {
    stored.facts_version >= FACTS_VERSION
        && stored.language == language
        && now - stored.fetched_at < ttl
}

fn media_kind(media: TitleArtMedia) -> TmdbMediaKind {
    match media {
        TitleArtMedia::Movie => TmdbMediaKind::Movie,
        TitleArtMedia::Tv => TmdbMediaKind::Tv,
    }
}

/// The facts of one TMDB work, refreshed when stale and a fetcher is given.
pub(crate) async fn resolve_title_facts(
    repo: &TitleFactsRepository<'_>,
    tmdb: Option<&impl TmdbFetch>,
    media: TitleArtMedia,
    tmdb_id: i64,
    language: &str,
) -> Option<TitleFacts> {
    let stored = repo.get(media, tmdb_id).await.ok().flatten();
    let stored_facts = stored.as_ref().and_then(|s| TitleFacts::from_json(&s.json));
    let now = nova_core::now_ts();
    let is_current = match (&stored, &stored_facts) {
        (Some(s), Some(f)) => current(s, language, facts_ttl(f), now),
        _ => false,
    };
    let Some(tmdb) = tmdb.filter(|_| !is_current) else {
        return stored_facts;
    };
    let ttl = stored_facts.as_ref().map_or(30 * DAY, facts_ttl);
    let request = TmdbRequest::Facts(media_kind(media), tmdb_id);
    let body = match tmdb.fetch_max_age(&request, ttl).await {
        Ok(Some(body)) => body,
        Ok(None) => return stored_facts, // TMDB no longer knows the id
        Err(e) => {
            tracing::warn!(error = ?e, tmdb_id, "title facts: TMDB unavailable");
            return stored_facts;
        }
    };
    let parsed = match media {
        TitleArtMedia::Movie => parse_movie_facts(&body, language),
        TitleArtMedia::Tv => parse_tv_facts(&body, language),
    };
    let facts = match parsed {
        Ok(facts) => facts,
        Err(e) => {
            tracing::warn!(error = %e, tmdb_id, "title facts: unreadable TMDB body");
            return stored_facts;
        }
    };
    if let Err(e) = repo
        .put(media, tmdb_id, language, &facts.to_json(), FACTS_VERSION)
        .await
    {
        tracing::warn!(error = %e, tmdb_id, "title facts: could not store");
    }
    Some(facts)
}

/// The TMDB episodes of one season of a TV work (empty when unknown).
pub(crate) async fn resolve_season(
    repo: &TitleFactsRepository<'_>,
    tmdb: Option<&impl TmdbFetch>,
    tmdb_id: i64,
    season: i64,
    language: &str,
    series: Option<&TitleFacts>,
) -> Vec<TmdbEpisode> {
    let stored = repo.season(tmdb_id, season).await.ok().flatten();
    let stored_episodes: Option<Vec<TmdbEpisode>> = stored
        .as_ref()
        .and_then(|s| serde_json::from_str(&s.json).ok());
    let ttl = season_ttl(series);
    let is_current = stored.as_ref().is_some_and(|s| {
        stored_episodes.is_some() && current(s, language, ttl, nova_core::now_ts())
    });
    let Some(tmdb) = tmdb.filter(|_| !is_current) else {
        return stored_episodes.unwrap_or_default();
    };
    let body = match tmdb
        .fetch_max_age(&TmdbRequest::Season(tmdb_id, season), ttl)
        .await
    {
        Ok(Some(body)) => body,
        Ok(None) => return stored_episodes.unwrap_or_default(),
        Err(e) => {
            tracing::warn!(error = ?e, tmdb_id, season, "season: TMDB unavailable");
            return stored_episodes.unwrap_or_default();
        }
    };
    let Ok(episodes) = parse_season(&body) else {
        return stored_episodes.unwrap_or_default();
    };
    let json = serde_json::to_string(&episodes).unwrap_or_else(|_| "[]".into());
    if let Err(e) = repo
        .put_season(tmdb_id, season, language, &json, FACTS_VERSION)
        .await
    {
        tracing::warn!(error = %e, tmdb_id, season, "season: could not store");
    }
    episodes
}

/// What a detail page shows about a title: confident TMDB facts first, the
/// provider's facts filling the gaps (and standing alone offline).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleFactsView {
    pub tagline: Option<String>,
    pub runtime_min: Option<i64>,
    /// Out of 10.
    pub rating: Option<f32>,
    /// `tmdb` | `provider`.
    pub rating_source: Option<&'static str>,
    pub certification: Option<String>,
    /// A YouTube watch URL (opened in the browser).
    pub trailer_url: Option<String>,
    pub cast: Vec<CastEntry>,
    pub directors: Vec<String>,
    pub studios: Vec<String>,
    pub status: Option<String>,
    pub first_year: Option<i32>,
    pub last_year: Option<i32>,
    pub seasons: Vec<SeasonSummary>,
    pub number_of_episodes: Option<i64>,
    pub next_episode: Option<UpcomingEpisode>,
    pub collection: Option<CollectionRef>,
    /// The provider's plot, for titles TMDB has no overview for.
    pub plot: Option<String>,
    /// The provider's genre label.
    pub genre: Option<String>,
}

fn split_names(list: Option<&str>, max: usize) -> Vec<String> {
    list.map(|l| {
        l.split([',', '/', '|'])
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .take(max)
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

/// Display precedence per field: confident TMDB, then provider.
pub(crate) fn merge_facts(
    tmdb: Option<TitleFacts>,
    provider: Option<ProviderFacts>,
) -> TitleFactsView {
    let t = tmdb.unwrap_or_default();
    let p = provider.unwrap_or_default();
    let (rating, rating_source) = match (t.rating, p.rating) {
        (Some(r), _) => (Some(r), Some("tmdb")),
        (None, Some(r)) => (Some(r), Some("provider")),
        _ => (None, None),
    };
    let cast = if t.cast.is_empty() {
        split_names(p.cast.as_deref(), 12)
            .into_iter()
            .map(|name| CastEntry {
                name,
                character: None,
                profile_url: None,
            })
            .collect()
    } else {
        t.cast
    };
    let directors = if t.directors.is_empty() {
        split_names(p.director.as_deref(), 3)
    } else {
        t.directors
    };
    TitleFactsView {
        tagline: t.tagline,
        runtime_min: t
            .runtime_min
            .or_else(|| p.duration_sec.map(|s| (s + 30) / 60).filter(|m| *m > 0)),
        rating,
        rating_source,
        certification: t.certification,
        trailer_url: t
            .trailer_youtube
            .or(p.trailer_youtube)
            .map(|key| format!("https://www.youtube.com/watch?v={key}")),
        cast,
        directors,
        studios: t.studios,
        status: t.status,
        first_year: t.first_year.or_else(|| {
            p.release_date
                .as_deref()
                .and_then(|d| d.get(..4)?.parse().ok())
        }),
        last_year: t.last_year,
        seasons: t.seasons,
        number_of_episodes: t.number_of_episodes,
        next_episode: t.next_episode,
        collection: t.collection,
        plot: p.plot,
        genre: p.genre,
    }
}

/// The detail-page facts of a movie or series.
#[tauri::command]
pub async fn get_title_facts(
    profile_id: String,
    subject: TitleArtSubject,
    state: State<'_, AppState>,
) -> Result<TitleFactsView, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let provider = match &subject {
        TitleArtSubject::Movie { entity_id } => movie_provider_facts(&pool, entity_id).await,
        TitleArtSubject::Series {
            source_id,
            series_title,
        } => {
            SeriesRepository::new(&pool)
                .series_provider_facts(*source_id, series_title)
                .await?
                .0
        }
    };
    let tmdb = match tmdb_work(&pool, &subject).await? {
        Some((media, tmdb_id)) => {
            let (client, language) = tmdb_access(&pool).await?;
            let cache = TmdbCacheRepository::new(&pool);
            let fetcher = client.as_ref().map(|client| CachedTmdb {
                client,
                cache: &cache,
            });
            resolve_title_facts(
                &TitleFactsRepository::new(&pool),
                fetcher.as_ref(),
                media,
                tmdb_id,
                &language,
            )
            .await
        }
        None => None,
    };
    Ok(merge_facts(tmdb, provider))
}

/// One episode as a series page lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeView {
    /// `series_episodes.id` (what plays).
    pub id: i64,
    pub season: i64,
    pub episode: i64,
    pub numbering: EpisodeNumberingKind,
    /// TMDB name in the metadata language, else the provider's title.
    pub title: Option<String>,
    pub overview: Option<String>,
    pub still_url: Option<String>,
    pub air_date: Option<String>,
    pub runtime_min: Option<i64>,
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    pub completed: bool,
}

/// Merge one season: provider episodes are what exists (and plays); TMDB adds
/// names, overviews, stills, air dates and runtimes, matched by episode
/// number only where the provider's numbers are real.
pub(crate) fn merge_season(
    provider: &[SeriesEpisode],
    facts: &HashMap<i64, ProviderFacts>,
    tmdb: &[TmdbEpisode],
    progress: &HashMap<i64, (f64, Option<f64>, bool)>,
) -> Vec<EpisodeView> {
    provider
        .iter()
        .map(|ep| {
            let t = (ep.numbering == EpisodeNumberingKind::SeasonEpisode)
                .then(|| tmdb.iter().find(|t| t.episode == ep.episode))
                .flatten();
            let f = facts.get(&ep.id);
            let (position_sec, duration_sec, completed) =
                progress.get(&ep.id).copied().unwrap_or((0.0, None, false));
            EpisodeView {
                id: ep.id,
                season: ep.season,
                episode: ep.episode,
                numbering: ep.numbering,
                title: t
                    .and_then(|t| t.name.clone())
                    .or_else(|| ep.episode_title.clone()),
                overview: t
                    .and_then(|t| t.overview.clone())
                    .or_else(|| f.and_then(|f| f.plot.clone())),
                still_url: t
                    .and_then(|t| t.still_url.clone())
                    .or_else(|| ep.logo_url.clone()),
                air_date: t
                    .and_then(|t| t.air_date.clone())
                    .or_else(|| f.and_then(|f| f.release_date.clone())),
                runtime_min: t.and_then(|t| t.runtime_min).or_else(|| {
                    f.and_then(|f| f.duration_sec)
                        .map(|s| (s + 30) / 60)
                        .filter(|m| *m > 0)
                }),
                position_sec,
                duration_sec,
                completed,
            }
        })
        .collect()
}

/// One season of a series, as its page lists it. Xtream episodes are fetched
/// on first open (like `list_series_episodes`).
#[tauri::command]
pub async fn get_season_episodes(
    profile_id: String,
    source_id: i64,
    series_title: String,
    season: i64,
    state: State<'_, AppState>,
) -> Result<Vec<EpisodeView>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let all = super::fetch_series_episodes(&pool, source_id, &series_title).await?;
    let provider: Vec<SeriesEpisode> = all.into_iter().filter(|e| e.season == season).collect();
    if provider.is_empty() {
        return Ok(Vec::new());
    }
    let series = SeriesRepository::new(&pool);
    let facts = series
        .episode_provider_facts(source_id, &series_title)
        .await?;
    let progress = WatchHistoryRepository::new(&pool)
        .episode_progress(source_id, &series_title)
        .await?;
    let subject = TitleArtSubject::Series {
        source_id,
        series_title: series_title.clone(),
    };
    let tmdb = match tmdb_work(&pool, &subject).await? {
        Some((_, tv_id)) => {
            let (client, language) = tmdb_access(&pool).await?;
            let cache = TmdbCacheRepository::new(&pool);
            let fetcher = client.as_ref().map(|client| CachedTmdb {
                client,
                cache: &cache,
            });
            let repo = TitleFactsRepository::new(&pool);
            let series_facts =
                resolve_title_facts(&repo, fetcher.as_ref(), TitleArtMedia::Tv, tv_id, &language)
                    .await;
            resolve_season(
                &repo,
                fetcher.as_ref(),
                tv_id,
                season,
                &language,
                series_facts.as_ref(),
            )
            .await
        }
        None => Vec::new(),
    };
    Ok(merge_season(&provider, &facts, &tmdb, &progress))
}

/// Where a series continues (see `WatchHistoryRepository::series_resume`).
#[tauri::command]
pub async fn get_series_resume(
    profile_id: String,
    source_id: i64,
    series_title: String,
    state: State<'_, AppState>,
) -> Result<Option<ContinueWatchingItem>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    WatchHistoryRepository::new(&pool)
        .series_resume(source_id, &series_title)
        .await
        .map_err(Into::into)
}

/// The canonical watch URL of a YouTube trailer link, or `None` for anything
/// else (the only external pages Nova opens).
pub(crate) fn trailer_watch_url(url: &str) -> Option<String> {
    let id = nova_core::provider_facts::youtube_id(url)?;
    Some(format!("https://www.youtube.com/watch?v={id}"))
}

/// Open a trailer in the default browser. Only YouTube trailers are opened.
#[tauri::command]
pub async fn open_trailer(url: String, app: tauri::AppHandle) -> Result<(), ApiError> {
    use tauri_plugin_shell::ShellExt;
    let url = trailer_watch_url(&url)
        .ok_or_else(|| ApiError::InvalidInput("not a trailer link".into()))?;
    #[allow(deprecated)] // `open` is the shell plugin's URL opener in this version
    app.shell()
        .open(url, None)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

/// Longest wait for a provider's movie details; the page renders without them.
const VOD_INFO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6);

/// A movie's provider facts, asking an Xtream panel for its details
/// (`get_vod_info`) the first time the bulk list gave no plot.
async fn movie_provider_facts(pool: &sqlx::SqlitePool, entity_id: &str) -> Option<ProviderFacts> {
    let entity = EntityId::new(entity_id.to_string()).ok()?;
    let target = CanonicalRepository::new(pool)
        .resolve_playback(&entity, None)
        .await
        .ok()
        .flatten()?;
    let vods = VodRepository::new(pool);
    let stored = vods.provider_facts(target.media_id).await.ok().flatten();
    if stored.as_ref().is_some_and(|f| f.plot.is_some()) {
        return stored;
    }
    let (source_id, stream_url) = vods.locator(target.media_id).await.ok().flatten()?;
    let source = SourceRepository::new(pool)
        .get(source_id)
        .await
        .ok()
        .flatten()?;
    let (Some(creds), Some(vod_id)) = (
        nova_xtream::XtreamCredentials::from_url(&source.url),
        nova_xtream::vod_id_from_url(&stream_url),
    ) else {
        return stored;
    };
    match tokio::time::timeout(
        VOD_INFO_TIMEOUT,
        nova_xtream::fetch_vod_info(&creds, vod_id),
    )
    .await
    {
        Ok(Ok(info)) => {
            let Some(facts) = info.facts else {
                return stored;
            };
            if let Err(e) = vods.merge_provider_facts(target.media_id, &facts).await {
                tracing::warn!(error = %e, "title facts: could not store provider details");
            }
            Some(match stored {
                Some(old) => facts.or(old),
                None => facts,
            })
        }
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "title facts: provider details unavailable");
            stored
        }
        Err(_) => stored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::ApiError;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct Fixture {
        language: String,
        bodies: HashMap<String, Option<String>>,
        requested: RefCell<Vec<(String, i64)>>,
    }

    impl Fixture {
        fn new(language: &str) -> Self {
            Self {
                language: language.into(),
                bodies: HashMap::new(),
                requested: RefCell::new(Vec::new()),
            }
        }
        fn with(mut self, key: &str, body: Option<&str>) -> Self {
            self.bodies.insert(key.into(), body.map(str::to_owned));
            self
        }
    }

    impl TmdbFetch for Fixture {
        fn language(&self) -> &str {
            &self.language
        }
        async fn fetch(&self, request: &TmdbRequest) -> Result<Option<String>, ApiError> {
            self.fetch_max_age(request, i64::MAX).await
        }
        async fn fetch_max_age(
            &self,
            request: &TmdbRequest,
            max_age_secs: i64,
        ) -> Result<Option<String>, ApiError> {
            let key = request.cache_key(&self.language);
            self.requested
                .borrow_mut()
                .push((key.clone(), max_age_secs));
            self.bodies
                .get(&key)
                .cloned()
                .ok_or_else(|| ApiError::Internal(format!("no fixture for {key}")))
        }
    }

    const MOVIE: &str = r#"{"id": 238, "runtime": 175, "vote_average": 8.7, "vote_count": 20000,
        "status": "Released", "release_date": "1972-03-14",
        "credits": {"cast": [{"name": "Marlon Brando", "character": "Vito", "order": 0}], "crew": []}}"#;
    const TV: &str = r#"{"id": 9, "status": "Returning Series", "episode_run_time": [45],
        "seasons": [{"season_number": 1, "name": "Season 1", "episode_count": 2}]}"#;
    const SEASON: &str = r#"{"episodes": [{"episode_number": 1, "name": "Pilot"}]}"#;

    async fn pool() -> (tempfile::TempDir, sqlx::SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = nova_storage::init_profile_db(dir.path().join("p.db").to_str().unwrap())
            .await
            .unwrap();
        (dir, pool)
    }

    fn ep(id: i64, season: i64, episode: i64, numbering: EpisodeNumberingKind) -> SeriesEpisode {
        SeriesEpisode {
            id,
            source_id: 1,
            series_title: "Dark".into(),
            category: None,
            season,
            episode,
            episode_title: Some(format!("Provider {episode}")),
            logo_url: Some(format!("https://still/{episode}.jpg")),
            stream_url: format!("https://x/{id}.mkv"),
            added_at: None,
            numbering,
            evidence: nova_core::SourceEvidence::default(),
        }
    }

    #[test]
    fn seasons_merge_tmdb_by_number_only_when_numbers_are_real() {
        let provider = vec![
            ep(10, 1, 1, EpisodeNumberingKind::SeasonEpisode),
            ep(11, 1, 2, EpisodeNumberingKind::SeasonEpisode),
            ep(12, 1, 3, EpisodeNumberingKind::Unknown),
        ];
        let tmdb = vec![
            TmdbEpisode {
                episode: 1,
                name: Some("Secrets".into()),
                overview: Some("TMDB overview".into()),
                still_url: Some("https://tmdb/1.jpg".into()),
                air_date: Some("2017-12-01".into()),
                runtime_min: Some(51),
            },
            TmdbEpisode {
                episode: 3,
                name: Some("Past and Present".into()),
                overview: None,
                still_url: None,
                air_date: None,
                runtime_min: None,
            },
        ];
        let facts = HashMap::from([(
            11,
            ProviderFacts {
                plot: Some("Provider plot".into()),
                duration_sec: Some(2_700),
                ..ProviderFacts::default()
            },
        )]);
        let progress = HashMap::from([
            (10, (3_000.0, Some(3_060.0), true)),
            (11, (600.0, Some(2_700.0), false)),
        ]);
        let season = merge_season(&provider, &facts, &tmdb, &progress);

        assert_eq!(season[0].title.as_deref(), Some("Secrets"));
        assert_eq!(season[0].still_url.as_deref(), Some("https://tmdb/1.jpg"));
        assert_eq!(
            (season[0].runtime_min, season[0].completed),
            (Some(51), true)
        );
        // No TMDB episode 2: provider title, plot and runtime.
        assert_eq!(season[1].title.as_deref(), Some("Provider 2"));
        assert_eq!(season[1].overview.as_deref(), Some("Provider plot"));
        assert_eq!(season[1].runtime_min, Some(45));
        assert_eq!(season[1].position_sec, 600.0);
        // Placeholder numbering never borrows TMDB episode 3's name.
        assert_eq!(season[2].title.as_deref(), Some("Provider 3"));
        assert_eq!(season[2].still_url.as_deref(), Some("https://still/3.jpg"));
    }

    #[test]
    fn only_youtube_trailers_are_opened() {
        assert_eq!(
            trailer_watch_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=x").as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
        );
        assert_eq!(
            trailer_watch_url("https://evil.example/watch?v=dQw4w9WgXcQ"),
            None
        );
        assert_eq!(trailer_watch_url("file:///etc/passwd"), None);
        assert_eq!(trailer_watch_url("javascript:alert(1)"), None);
    }

    #[test]
    fn tmdb_facts_win_and_provider_facts_fill_gaps() {
        let provider = ProviderFacts {
            plot: Some("Provider plot".into()),
            cast: Some("Actor A, Actor B / Actor C".into()),
            director: Some("Someone".into()),
            rating: Some(6.0),
            duration_sec: Some(5_400),
            trailer_youtube: Some("provider123".into()),
            release_date: Some("1999-03-31".into()),
            genre: Some("Sci-Fi".into()),
            ..ProviderFacts::default()
        };
        let offline = merge_facts(None, Some(provider.clone()));
        assert_eq!(
            (offline.rating, offline.rating_source),
            (Some(6.0), Some("provider"))
        );
        assert_eq!(offline.runtime_min, Some(90));
        assert_eq!(offline.first_year, Some(1999));
        let names: Vec<&str> = offline.cast.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Actor A", "Actor B", "Actor C"]);
        assert_eq!(
            offline.trailer_url.as_deref(),
            Some("https://www.youtube.com/watch?v=provider123")
        );
        assert_eq!(offline.plot.as_deref(), Some("Provider plot"));

        let tmdb = TitleFacts {
            rating: Some(8.7),
            runtime_min: Some(136),
            cast: vec![CastEntry {
                name: "Keanu Reeves".into(),
                character: Some("Neo".into()),
                profile_url: None,
            }],
            trailer_youtube: Some("tmdbTrailer".into()),
            ..TitleFacts::default()
        };
        let merged = merge_facts(Some(tmdb), Some(provider));
        assert_eq!(
            (merged.rating, merged.rating_source),
            (Some(8.7), Some("tmdb"))
        );
        assert_eq!(merged.runtime_min, Some(136));
        assert_eq!(merged.cast[0].name, "Keanu Reeves");
        assert_eq!(
            merged.directors,
            vec!["Someone"],
            "gaps filled from the provider"
        );
        assert_eq!(
            merged.trailer_url.as_deref(),
            Some("https://www.youtube.com/watch?v=tmdbTrailer")
        );
        assert_eq!(merge_facts(None, None), TitleFactsView::default());
    }

    #[tokio::test]
    async fn facts_are_fetched_once_then_served_and_survive_outages() {
        let (_dir, pool) = pool().await;
        let repo = TitleFactsRepository::new(&pool);
        let tmdb = Fixture::new("en-US").with("movie-facts-v1::238", Some(MOVIE));

        let facts = resolve_title_facts(&repo, Some(&tmdb), TitleArtMedia::Movie, 238, "en-US")
            .await
            .unwrap();
        assert_eq!((facts.runtime_min, facts.rating), (Some(175), Some(8.7)));
        assert_eq!(facts.cast[0].name, "Marlon Brando");
        let again =
            resolve_title_facts(&repo, Some(&tmdb), TitleArtMedia::Movie, 238, "en-US").await;
        assert_eq!(again.as_ref(), Some(&facts));
        assert_eq!(
            tmdb.requested.borrow().len(),
            1,
            "stored facts are not refetched"
        );

        // Offline: stored facts still show.
        let offline =
            resolve_title_facts(&repo, None::<&Fixture>, TitleArtMedia::Movie, 238, "en-US").await;
        assert_eq!(offline.as_ref(), Some(&facts));
        // Another language, TMDB failing: the stored facts beat nothing.
        let failing = Fixture::new("fr-FR");
        let fallback =
            resolve_title_facts(&repo, Some(&failing), TitleArtMedia::Movie, 238, "fr-FR").await;
        assert_eq!(fallback.as_ref(), Some(&facts));
        assert_eq!(
            repo.get(TitleArtMedia::Movie, 238)
                .await
                .unwrap()
                .unwrap()
                .language,
            "en-US"
        );
    }

    #[tokio::test]
    async fn airing_shows_refresh_weekly_and_seasons_are_stored() {
        let (_dir, pool) = pool().await;
        let repo = TitleFactsRepository::new(&pool);
        let tmdb = Fixture::new("en-US")
            .with("tv-facts-v1::9", Some(TV))
            .with("tv-season-v1::9::1", Some(SEASON));
        let facts = resolve_title_facts(&repo, Some(&tmdb), TitleArtMedia::Tv, 9, "en-US")
            .await
            .unwrap();
        assert_eq!(facts_ttl(&facts), 7 * DAY);
        assert_eq!(season_ttl(Some(&facts)), 7 * DAY);

        let episodes = resolve_season(&repo, Some(&tmdb), 9, 1, "en-US", Some(&facts)).await;
        assert_eq!(episodes[0].name.as_deref(), Some("Pilot"));
        let again = resolve_season(&repo, Some(&tmdb), 9, 1, "en-US", Some(&facts)).await;
        assert_eq!(again, episodes);
        {
            let requested = tmdb.requested.borrow();
            assert_eq!(requested.len(), 2, "one facts and one season request");
            assert_eq!(requested[1], ("tv-season-v1::9::1".to_string(), 7 * DAY));
        }

        // A stale row (older than its TTL) is refreshed.
        sqlx::query("UPDATE tmdb_season_episodes SET fetched_at = fetched_at - ?")
            .bind(8 * DAY)
            .execute(&pool)
            .await
            .unwrap();
        resolve_season(&repo, Some(&tmdb), 9, 1, "en-US", Some(&facts)).await;
        assert_eq!(tmdb.requested.borrow().len(), 3);

        let ended = TitleFacts {
            status: Some("Ended".into()),
            ..TitleFacts::default()
        };
        assert_eq!(season_ttl(Some(&ended)), 90 * DAY);
        assert_eq!(facts_ttl(&ended), 30 * DAY);
    }
}
