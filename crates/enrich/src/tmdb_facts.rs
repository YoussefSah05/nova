//! Rich title facts from TMDB (master plan M4.6): what a detail page shows
//! beyond title, poster and overview. Movie details with credits, release
//! dates, videos and external ids; TV details with aggregate credits, content
//! ratings, videos and external ids; season details for episode lists.
//!
//! Pure parsing and choice rules only: fetched on demand when a title's page
//! opens, cached like every TMDB body, stored per TMDB work.

use serde::{Deserialize, Serialize};

use crate::tmdb_art::language_code;
use crate::EnrichError;

/// Bump when the facts below or their choice rules change; stored facts older
/// than this are fetched again when next shown.
pub const FACTS_VERSION: i64 = 1;

const IMG_BASE: &str = "https://image.tmdb.org/t/p";
const MAX_CAST: usize = 12;
const MAX_COMPANIES: usize = 3;

/// One billed cast member.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CastEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_url: Option<String>,
}

/// One season as listed by the series details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeasonSummary {
    pub number: i64,
    pub name: String,
    pub episode_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
}

/// The next episode TMDB knows is scheduled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingEpisode {
    pub season: i64,
    pub episode: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub air_date: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionRef {
    pub id: i64,
    pub name: String,
}

/// Facts shown on a movie or series page. Absent values are omitted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TitleFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tagline: Option<String>,
    /// Movie runtime, or the typical episode runtime of a series (minutes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_min: Option<i64>,
    /// TMDB user score out of 10 (one decimal), when enough people voted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vote_count: Option<i64>,
    /// Age rating for the metadata region (e.g. `12`, `PG-13`, `TV-MA`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certification: Option<String>,
    /// YouTube id of the chosen trailer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trailer_youtube: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cast: Vec<CastEntry>,
    /// Directors (movies) or creators (series).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub directors: Vec<String>,
    /// Production companies (movies) or networks (series).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub studios: Vec<String>,
    /// TMDB status: `Released`, `Returning Series`, `Ended`, `Canceled`…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_year: Option<i32>,
    /// Series only: year of the latest episode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_year: Option<i32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub seasons: Vec<SeasonSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_episodes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_episode: Option<UpcomingEpisode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<CollectionRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imdb_id: Option<String>,
}

impl TitleFacts {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
    pub fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }
}

// ── Raw response shapes (only what is read) ─────────────────────────────────

#[derive(Debug, Default, Deserialize)]
struct Named {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawCast {
    #[serde(default)]
    name: String,
    #[serde(default)]
    character: Option<String>,
    #[serde(default)]
    profile_path: Option<String>,
    #[serde(default)]
    order: i64,
    /// Aggregate credits (TV) list characters per role.
    #[serde(default)]
    roles: Vec<RawRole>,
}

#[derive(Debug, Deserialize)]
struct RawRole {
    #[serde(default)]
    character: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCrew {
    #[serde(default)]
    name: String,
    #[serde(default)]
    job: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawCredits {
    #[serde(default)]
    cast: Vec<RawCast>,
    #[serde(default)]
    crew: Vec<RawCrew>,
}

#[derive(Debug, Deserialize)]
struct RawVideo {
    #[serde(default)]
    key: String,
    #[serde(default)]
    site: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    official: bool,
    #[serde(default)]
    iso_639_1: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Results<T> {
    #[serde(default = "Vec::new")]
    results: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct RawReleaseCountry {
    #[serde(default)]
    iso_3166_1: String,
    #[serde(default)]
    release_dates: Vec<RawReleaseDate>,
}

#[derive(Debug, Deserialize)]
struct RawReleaseDate {
    #[serde(default)]
    certification: String,
    #[serde(default, rename = "type")]
    kind: i64,
}

#[derive(Debug, Deserialize)]
struct RawContentRating {
    #[serde(default)]
    iso_3166_1: String,
    #[serde(default)]
    rating: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawExternalIds {
    #[serde(default)]
    imdb_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCollection {
    id: i64,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawSeason {
    #[serde(default)]
    season_number: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    episode_count: i64,
    #[serde(default)]
    air_date: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawNextEpisode {
    #[serde(default)]
    season_number: i64,
    #[serde(default)]
    episode_number: i64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    air_date: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDetails {
    #[serde(default)]
    tagline: Option<String>,
    #[serde(default)]
    runtime: Option<i64>,
    #[serde(default)]
    episode_run_time: Vec<i64>,
    #[serde(default)]
    vote_average: Option<f64>,
    #[serde(default)]
    vote_count: Option<i64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    first_air_date: Option<String>,
    #[serde(default)]
    last_air_date: Option<String>,
    #[serde(default)]
    imdb_id: Option<String>,
    #[serde(default)]
    belongs_to_collection: Option<RawCollection>,
    #[serde(default)]
    production_companies: Vec<Named>,
    #[serde(default)]
    networks: Vec<Named>,
    #[serde(default)]
    created_by: Vec<Named>,
    #[serde(default)]
    seasons: Vec<RawSeason>,
    #[serde(default)]
    number_of_episodes: Option<i64>,
    #[serde(default)]
    next_episode_to_air: Option<RawNextEpisode>,
    #[serde(default)]
    credits: Option<RawCredits>,
    #[serde(default)]
    aggregate_credits: Option<RawCredits>,
    #[serde(default)]
    release_dates: Option<Results<RawReleaseCountry>>,
    #[serde(default)]
    content_ratings: Option<Results<RawContentRating>>,
    #[serde(default)]
    videos: Option<Results<RawVideo>>,
    #[serde(default)]
    external_ids: Option<RawExternalIds>,
}

// ── Choice rules ────────────────────────────────────────────────────────────

/// The metadata language's region (`fr-FR` → `FR`); `US` otherwise.
pub fn region_of(metadata_language: &str) -> String {
    metadata_language
        .split(['-', '_'])
        .nth(1)
        .filter(|r| r.len() == 2)
        .map(str::to_ascii_uppercase)
        .unwrap_or_else(|| "US".into())
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn year(date: Option<&str>) -> Option<i32> {
    date?.get(..4)?.parse().ok()
}

fn image(path: Option<&str>, size: &str) -> Option<String> {
    path.filter(|p| p.starts_with('/') && p.len() > 1)
        .map(|p| format!("{IMG_BASE}/{size}{p}"))
}

/// Movie certification for `region` (then US): theatrical releases first.
fn movie_certification(dates: &Results<RawReleaseCountry>, region: &str) -> Option<String> {
    let pick = |country: &str| {
        let entry = dates.results.iter().find(|c| c.iso_3166_1 == country)?;
        let mut certified: Vec<&RawReleaseDate> = entry
            .release_dates
            .iter()
            .filter(|d| !d.certification.trim().is_empty())
            .collect();
        // Theatrical (3), then limited (2), then anything else.
        certified.sort_by_key(|d| match d.kind {
            3 => 0,
            2 => 1,
            _ => 2,
        });
        certified
            .first()
            .map(|d| d.certification.trim().to_string())
    };
    pick(region).or_else(|| pick("US"))
}

fn tv_certification(ratings: &Results<RawContentRating>, region: &str) -> Option<String> {
    let pick = |country: &str| {
        ratings
            .results
            .iter()
            .find(|r| r.iso_3166_1 == country && !r.rating.trim().is_empty())
            .map(|r| r.rating.trim().to_string())
    };
    pick(region).or_else(|| pick("US"))
}

/// A YouTube trailer: official before fan uploads, the metadata language
/// before English before anything, trailers before teasers.
fn trailer(videos: &Results<RawVideo>, metadata_language: &str) -> Option<String> {
    let want = language_code(metadata_language);
    videos
        .results
        .iter()
        .filter(|v| v.site == "YouTube" && !v.key.is_empty())
        .filter(|v| v.kind == "Trailer" || v.kind == "Teaser")
        .min_by_key(|v| {
            let lang = match v.iso_639_1.as_deref() {
                Some(l) if l == want => 0,
                Some("en") => 1,
                _ => 2,
            };
            (v.kind != "Trailer", lang, !v.official)
        })
        .map(|v| v.key.clone())
}

fn cast(credits: &RawCredits) -> Vec<CastEntry> {
    let mut members: Vec<&RawCast> = credits
        .cast
        .iter()
        .filter(|c| !c.name.trim().is_empty())
        .collect();
    members.sort_by_key(|c| c.order);
    members
        .into_iter()
        .take(MAX_CAST)
        .map(|c| CastEntry {
            name: c.name.trim().to_string(),
            character: non_empty(
                c.character
                    .clone()
                    .or_else(|| c.roles.first().and_then(|r| r.character.clone())),
            ),
            profile_url: image(c.profile_path.as_deref(), "w185"),
        })
        .collect()
}

fn names(list: &[Named], max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in list {
        let name = n.name.trim();
        if !name.is_empty() && !out.iter().any(|o| o == name) && out.len() < max {
            out.push(name.to_string());
        }
    }
    out
}

fn score(vote_average: Option<f64>, vote_count: Option<i64>) -> Option<f32> {
    // A handful of votes is noise, not a score.
    let votes = vote_count.unwrap_or(0);
    let avg = vote_average?;
    (votes >= 10 && avg > 0.0).then(|| ((avg * 10.0).round() / 10.0) as f32)
}

fn parse_raw(body: &str) -> Result<RawDetails, EnrichError> {
    serde_json::from_str(body).map_err(|e| EnrichError::Parse(e.to_string()))
}

/// Facts from a `/movie/{id}` body with
/// `append_to_response=credits,release_dates,videos,external_ids`.
pub fn parse_movie_facts(body: &str, metadata_language: &str) -> Result<TitleFacts, EnrichError> {
    let d = parse_raw(body)?;
    let region = region_of(metadata_language);
    let credits = d.credits.unwrap_or_default();
    let mut directors: Vec<String> = Vec::new();
    for c in credits.crew.iter().filter(|c| c.job == "Director") {
        let name = c.name.trim();
        if !name.is_empty() && !directors.iter().any(|d| d == name) {
            directors.push(name.to_string());
        }
    }
    Ok(TitleFacts {
        tagline: non_empty(d.tagline),
        runtime_min: d.runtime.filter(|r| *r > 0),
        rating: score(d.vote_average, d.vote_count),
        vote_count: d.vote_count.filter(|v| *v > 0),
        certification: d
            .release_dates
            .as_ref()
            .and_then(|r| movie_certification(r, &region)),
        trailer_youtube: d
            .videos
            .as_ref()
            .and_then(|v| trailer(v, metadata_language)),
        cast: cast(&credits),
        directors,
        studios: names(&d.production_companies, MAX_COMPANIES),
        status: non_empty(d.status),
        first_year: year(d.release_date.as_deref()),
        collection: d
            .belongs_to_collection
            .filter(|c| c.id > 0 && !c.name.trim().is_empty())
            .map(|c| CollectionRef {
                id: c.id,
                name: c.name.trim().to_string(),
            }),
        imdb_id: non_empty(d.imdb_id.or(d.external_ids.and_then(|e| e.imdb_id))),
        ..TitleFacts::default()
    })
}

/// Facts from a `/tv/{id}` body with
/// `append_to_response=aggregate_credits,content_ratings,videos,external_ids`.
pub fn parse_tv_facts(body: &str, metadata_language: &str) -> Result<TitleFacts, EnrichError> {
    let d = parse_raw(body)?;
    let region = region_of(metadata_language);
    let credits = d.aggregate_credits.or(d.credits).unwrap_or_default();
    let mut seasons: Vec<SeasonSummary> = d
        .seasons
        .iter()
        .filter(|s| s.episode_count > 0)
        .map(|s| SeasonSummary {
            number: s.season_number,
            name: s.name.trim().to_string(),
            episode_count: s.episode_count,
            air_date: non_empty(s.air_date.clone()),
            poster_url: image(s.poster_path.as_deref(), "w342"),
        })
        .collect();
    seasons.sort_by_key(|s| s.number);
    Ok(TitleFacts {
        tagline: non_empty(d.tagline),
        runtime_min: d.episode_run_time.iter().copied().find(|r| *r > 0),
        rating: score(d.vote_average, d.vote_count),
        vote_count: d.vote_count.filter(|v| *v > 0),
        certification: d
            .content_ratings
            .as_ref()
            .and_then(|r| tv_certification(r, &region)),
        trailer_youtube: d
            .videos
            .as_ref()
            .and_then(|v| trailer(v, metadata_language)),
        cast: cast(&credits),
        directors: names(&d.created_by, 5),
        studios: names(&d.networks, MAX_COMPANIES),
        status: non_empty(d.status),
        first_year: year(d.first_air_date.as_deref()),
        last_year: year(d.last_air_date.as_deref()),
        seasons,
        number_of_episodes: d.number_of_episodes.filter(|n| *n > 0),
        next_episode: d.next_episode_to_air.and_then(|n| {
            Some(UpcomingEpisode {
                season: n.season_number,
                episode: n.episode_number,
                name: non_empty(n.name),
                air_date: non_empty(n.air_date)?,
            })
        }),
        imdb_id: non_empty(d.external_ids.and_then(|e| e.imdb_id)),
        ..TitleFacts::default()
    })
}

/// One episode of a TMDB season.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmdbEpisode {
    pub episode: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub still_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_min: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RawSeasonDetail {
    #[serde(default)]
    episodes: Vec<RawEpisode>,
}

#[derive(Debug, Deserialize)]
struct RawEpisode {
    #[serde(default)]
    episode_number: i64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    still_path: Option<String>,
    #[serde(default)]
    air_date: Option<String>,
    #[serde(default)]
    runtime: Option<i64>,
}

/// Episodes of a `/tv/{id}/season/{n}` body, by episode number.
pub fn parse_season(body: &str) -> Result<Vec<TmdbEpisode>, EnrichError> {
    let raw: RawSeasonDetail =
        serde_json::from_str(body).map_err(|e| EnrichError::Parse(e.to_string()))?;
    let mut episodes: Vec<TmdbEpisode> = raw
        .episodes
        .into_iter()
        .filter(|e| e.episode_number > 0)
        .map(|e| TmdbEpisode {
            episode: e.episode_number,
            // TMDB fills missing names with "Episode 7": that says nothing.
            name: non_empty(e.name).filter(|n| !is_placeholder_name(n, e.episode_number)),
            overview: non_empty(e.overview),
            still_url: image(e.still_path.as_deref(), "w300"),
            air_date: non_empty(e.air_date),
            runtime_min: e.runtime.filter(|r| *r > 0),
        })
        .collect();
    episodes.sort_by_key(|e| e.episode);
    Ok(episodes)
}

fn is_placeholder_name(name: &str, number: i64) -> bool {
    let lower = name.to_lowercase();
    let n = number.to_string();
    [
        "episode",
        "épisode",
        "episodio",
        "folge",
        "aflevering",
        "bölüm",
        "الحلقة",
    ]
    .iter()
    .any(|w| lower == format!("{w} {n}") || lower == format!("{n} {w}"))
}

/// TMDB ids listed by a results page (recommendations, similar, trending),
/// in TMDB's order, de-duplicated.
pub fn result_ids(body: &str) -> Result<Vec<i64>, EnrichError> {
    #[derive(Deserialize)]
    struct Item {
        id: i64,
    }
    let page: Results<Item> =
        serde_json::from_str(body).map_err(|e| EnrichError::Parse(e.to_string()))?;
    let mut out: Vec<i64> = Vec::new();
    for item in page.results {
        if item.id > 0 && !out.contains(&item.id) {
            out.push(item.id);
        }
    }
    Ok(out)
}

/// A collection's name and its films' TMDB ids in release order.
pub fn parse_collection(body: &str) -> Result<(String, Vec<i64>), EnrichError> {
    #[derive(Deserialize)]
    struct Part {
        id: i64,
        #[serde(default)]
        release_date: Option<String>,
    }
    #[derive(Deserialize)]
    struct Collection {
        #[serde(default)]
        name: String,
        #[serde(default)]
        parts: Vec<Part>,
    }
    let c: Collection =
        serde_json::from_str(body).map_err(|e| EnrichError::Parse(e.to_string()))?;
    let mut parts = c.parts;
    // Unreleased films (no date) go last.
    parts.sort_by(|a, b| {
        let key = |p: &Part| {
            p.release_date
                .clone()
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| "9999".into())
        };
        key(a).cmp(&key(b))
    });
    Ok((
        c.name.trim().to_string(),
        parts
            .into_iter()
            .map(|p| p.id)
            .filter(|id| *id > 0)
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOVIE: &str = r#"{
      "id": 27205, "title": "Inception", "tagline": " Your mind is the scene of the crime. ",
      "runtime": 148, "vote_average": 8.369, "vote_count": 36000, "status": "Released",
      "release_date": "2010-07-15", "imdb_id": "tt1375666",
      "belongs_to_collection": null,
      "production_companies": [{"name": "Legendary Pictures"}, {"name": "Syncopy"},
                               {"name": "Warner Bros. Pictures"}, {"name": "Extra"}],
      "credits": {
        "cast": [
          {"name": "Joseph Gordon-Levitt", "character": "Arthur", "order": 1, "profile_path": "/jgl.jpg"},
          {"name": "Leonardo DiCaprio", "character": "Cobb", "order": 0, "profile_path": "/leo.jpg"},
          {"name": "", "character": "Ghost", "order": 2}
        ],
        "crew": [{"name": "Christopher Nolan", "job": "Director"},
                 {"name": "Christopher Nolan", "job": "Writer"},
                 {"name": "Hans Zimmer", "job": "Original Music Composer"}]
      },
      "release_dates": {"results": [
        {"iso_3166_1": "US", "release_dates": [{"certification": "PG-13", "type": 3}]},
        {"iso_3166_1": "FR", "release_dates": [{"certification": "", "type": 3},
                                               {"certification": "TP", "type": 4},
                                               {"certification": "12", "type": 3}]}
      ]},
      "videos": {"results": [
        {"key": "fanmade1234", "site": "YouTube", "type": "Trailer", "official": false, "iso_639_1": "en"},
        {"key": "teaserFR123", "site": "YouTube", "type": "Teaser", "official": true, "iso_639_1": "fr"},
        {"key": "officialEN1", "site": "YouTube", "type": "Trailer", "official": true, "iso_639_1": "en"},
        {"key": "vimeo", "site": "Vimeo", "type": "Trailer", "official": true, "iso_639_1": "fr"}
      ]},
      "external_ids": {"imdb_id": "tt1375666"}
    }"#;

    #[test]
    fn movie_facts_pick_region_trailer_and_billing_order() {
        let fr = parse_movie_facts(MOVIE, "fr-FR").unwrap();
        assert_eq!(
            fr.tagline.as_deref(),
            Some("Your mind is the scene of the crime.")
        );
        assert_eq!((fr.runtime_min, fr.rating), (Some(148), Some(8.4)));
        assert_eq!(
            fr.certification.as_deref(),
            Some("12"),
            "theatrical before others"
        );
        assert_eq!(
            fr.trailer_youtube.as_deref(),
            Some("officialEN1"),
            "a trailer beats a teaser in the right language; official beats fan-made"
        );
        assert_eq!(fr.cast[0].name, "Leonardo DiCaprio");
        assert_eq!(fr.cast[0].character.as_deref(), Some("Cobb"));
        assert_eq!(
            fr.cast[0].profile_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w185/leo.jpg")
        );
        assert_eq!(fr.cast.len(), 2, "unnamed cast dropped");
        assert_eq!(fr.directors, vec!["Christopher Nolan"]);
        assert_eq!(fr.studios.len(), 3);
        assert_eq!(
            (fr.first_year, fr.imdb_id.as_deref()),
            (Some(2010), Some("tt1375666"))
        );

        let other = parse_movie_facts(MOVIE, "ar").unwrap();
        assert_eq!(other.certification.as_deref(), Some("PG-13"), "US fallback");
        let round = TitleFacts::from_json(&fr.to_json()).unwrap();
        assert_eq!(round, fr);
    }

    const TV: &str = r#"{
      "id": 1399, "name": "Game of Thrones", "tagline": "Winter is coming.",
      "episode_run_time": [0, 60], "vote_average": 8.4, "vote_count": 5,
      "status": "Ended", "first_air_date": "2011-04-17", "last_air_date": "2019-05-19",
      "number_of_episodes": 73,
      "networks": [{"name": "HBO"}], "created_by": [{"name": "David Benioff"}, {"name": "D. B. Weiss"}],
      "seasons": [
        {"season_number": 1, "name": "Season 1", "episode_count": 10, "air_date": "2011-04-17", "poster_path": "/s1.jpg"},
        {"season_number": 0, "name": "Specials", "episode_count": 3, "air_date": null},
        {"season_number": 9, "name": "Unannounced", "episode_count": 0}
      ],
      "next_episode_to_air": {"season_number": 9, "episode_number": 1, "name": "Soon", "air_date": "2027-01-01"},
      "aggregate_credits": {"cast": [
        {"name": "Emilia Clarke", "order": 1, "roles": [{"character": "Daenerys Targaryen"}]},
        {"name": "Kit Harington", "order": 0, "roles": [{"character": "Jon Snow"}], "profile_path": "/kit.jpg"}
      ]},
      "content_ratings": {"results": [{"iso_3166_1": "US", "rating": "TV-MA"}, {"iso_3166_1": "DE", "rating": "16"}]},
      "videos": {"results": []},
      "external_ids": {"imdb_id": "tt0944947"}
    }"#;

    #[test]
    fn tv_facts_list_seasons_creators_and_next_episode() {
        let de = parse_tv_facts(TV, "de-DE").unwrap();
        assert_eq!(de.runtime_min, Some(60));
        assert_eq!(de.rating, None, "five votes is not a score");
        assert_eq!(de.certification.as_deref(), Some("16"));
        assert_eq!(de.trailer_youtube, None);
        assert_eq!(de.cast[0].name, "Kit Harington");
        assert_eq!(de.cast[0].character.as_deref(), Some("Jon Snow"));
        assert_eq!(de.directors, vec!["David Benioff", "D. B. Weiss"]);
        assert_eq!(de.studios, vec!["HBO"]);
        assert_eq!((de.first_year, de.last_year), (Some(2011), Some(2019)));
        let numbers: Vec<i64> = de.seasons.iter().map(|s| s.number).collect();
        assert_eq!(numbers, vec![0, 1], "empty seasons dropped, sorted");
        assert_eq!(
            de.next_episode.as_ref().map(|n| n.air_date.as_str()),
            Some("2027-01-01")
        );
        assert_eq!(
            parse_tv_facts(TV, "en").unwrap().certification.as_deref(),
            Some("TV-MA")
        );
    }

    #[test]
    fn season_episodes_drop_placeholder_names() {
        let body = r#"{"episodes": [
          {"episode_number": 2, "name": "Episode 2", "overview": "", "still_path": "/e2.jpg", "runtime": 55},
          {"episode_number": 1, "name": "Winter Is Coming", "overview": "Lord Stark...", "air_date": "2011-04-17", "runtime": 62},
          {"episode_number": 0, "name": "Bogus"}
        ]}"#;
        let eps = parse_season(body).unwrap();
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].name.as_deref(), Some("Winter Is Coming"));
        assert_eq!(eps[0].runtime_min, Some(62));
        assert_eq!(eps[1].name, None, "\"Episode 2\" is no title");
        assert_eq!(eps[1].overview, None);
        assert_eq!(
            eps[1].still_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w300/e2.jpg")
        );
        assert!(parse_season("not json").is_err());
    }

    #[test]
    fn lists_ids_and_orders_collections() {
        let page = r#"{"page": 1, "results": [{"id": 3}, {"id": 1}, {"id": 3}, {"id": 0}]}"#;
        assert_eq!(result_ids(page).unwrap(), vec![3, 1]);
        let collection = r#"{"id": 230, "name": " The Godfather Collection ", "parts": [
            {"id": 242, "release_date": "1990-12-25"}, {"id": 999, "release_date": ""},
            {"id": 238, "release_date": "1972-03-14"}, {"id": 240, "release_date": "1974-12-20"}]}"#;
        assert_eq!(
            parse_collection(collection).unwrap(),
            (
                "The Godfather Collection".to_string(),
                vec![238, 240, 242, 999]
            )
        );
    }

    #[test]
    fn regions_follow_the_metadata_language() {
        assert_eq!(region_of("fr-FR"), "FR");
        assert_eq!(region_of("pt-BR"), "BR");
        assert_eq!(region_of("ar"), "US");
    }
}
