//! TMDB metadata matching. A thin async client over TMDB's v3 API (still the
//! current API for movie/TV data), plus pure, unit-tested matching logic.
//!
//! **A TMDB ID is external evidence, never a Nova identity.** A match attaches
//! display metadata (title, art, overview, genres) to a Nova entity and records
//! how it was chosen. It never creates, merges or renames Nova entities.
//!
//! Resolution order per title:
//! 1. **Trusted ID:** a provider-supplied TMDB ID is looked up directly
//!    ([`TmdbClient::movie_details`]) and accepted only when the details
//!    corroborate the catalog evidence ([`verify_trusted`]).
//! 2. **Search:** otherwise candidates are searched, ranked ([`rank`]) and
//!    decided ([`decide`]). Below [`MIN_CONFIDENCE`], or when two different
//!    titles score within [`AMBIGUITY_MARGIN`], the title stays **unresolved**:
//!    no metadata is attached.
//!
//! The client returns raw JSON bodies so callers can cache them verbatim (TMDB's
//! terms permit caching). Auth prefers the *API Read Access Token* (a Bearer
//! header, TMDB's default) and still accepts a legacy v3 API key. Error messages
//! never contain request URLs, which would carry a legacy key.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::EnrichError;

const API_BASE: &str = "https://api.themoviedb.org/3";
const IMG_BASE: &str = "https://image.tmdb.org/t/p";
/// Sequential calls spaced ~25ms apart stay under TMDB's ~40 req/s ceiling.
const REQUEST_SPACING: Duration = Duration::from_millis(25);

/// Version of the matching rules. Bump when scoring, thresholds or query
/// building change: every stored result with an older version is re-matched.
pub const MATCHER_VERSION: u32 = 2;
/// A search candidate must score at least this to be accepted.
pub const MIN_CONFIDENCE: f32 = 0.6;
/// Two *different* titles scoring this close are ambiguous: neither is taken.
pub const AMBIGUITY_MARGIN: f32 = 0.05;
/// Candidates kept as evidence per decision.
pub const MAX_EVIDENCE_CANDIDATES: usize = 5;

// ── API response shapes ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<TmdbItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct GenreResponse {
    #[serde(default)]
    genres: Vec<GenreEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GenreEntry {
    pub id: i64,
    pub name: String,
}

/// A search result or a details body (the shared fields).
#[derive(Debug, Clone, Deserialize)]
pub struct TmdbItem {
    pub id: i64,
    #[serde(default)]
    title: Option<String>, // movies
    #[serde(default)]
    name: Option<String>, // tv
    #[serde(default)]
    original_title: Option<String>,
    #[serde(default)]
    original_name: Option<String>,
    #[serde(default)]
    release_date: Option<String>, // movies, "YYYY-MM-DD"
    #[serde(default)]
    first_air_date: Option<String>, // tv
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    /// Search results carry genre IDs...
    #[serde(default)]
    genre_ids: Vec<i64>,
    /// ...details bodies carry genre objects.
    #[serde(default)]
    genres: Vec<GenreEntry>,
    /// Present on `/search/multi` results (`movie` | `tv` | `person`).
    #[serde(default)]
    media_type: Option<String>,
}

impl TmdbItem {
    pub fn display_title(&self) -> Option<&str> {
        self.title.as_deref().or(self.name.as_deref())
    }
    fn original(&self) -> Option<&str> {
        self.original_title
            .as_deref()
            .or(self.original_name.as_deref())
    }
    /// Skip non-title results (people) that `/search/multi` mixes in.
    fn is_title(&self) -> bool {
        self.media_type.as_deref() != Some("person")
    }
    pub fn year(&self) -> Option<i32> {
        let d = self
            .release_date
            .as_deref()
            .or(self.first_air_date.as_deref())?;
        d.get(0..4)?.parse().ok()
    }
}

/// Display metadata chosen for one Nova title.
#[derive(Debug, Clone, PartialEq)]
pub struct TmdbMatch {
    pub tmdb_id: i64,
    pub canonical_title: String,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub overview: Option<String>,
    /// Genre names (TMDB's), in TMDB's order.
    pub genres: Vec<String>,
}

impl TmdbMatch {
    pub fn from_item(item: &TmdbItem, fallback_title: &str, genres: &HashMap<i64, String>) -> Self {
        let names: Vec<String> = if item.genres.is_empty() {
            item.genre_ids
                .iter()
                .filter_map(|id| genres.get(id).cloned())
                .collect()
        } else {
            item.genres.iter().map(|g| g.name.clone()).collect()
        };
        Self {
            tmdb_id: item.id,
            canonical_title: item.display_title().unwrap_or(fallback_title).to_string(),
            poster_url: image_url(&item.poster_path, "w500"),
            backdrop_url: image_url(&item.backdrop_path, "w1280"),
            overview: item.overview.clone().filter(|o| !o.trim().is_empty()),
            genres: names,
        }
    }
}

// ── Client ──────────────────────────────────────────────────────────────────

/// How requests authenticate. Both work on the v3 endpoints used here.
#[derive(Clone)]
enum TmdbAuth {
    /// "API Read Access Token": a JWT sent as `Authorization: Bearer`.
    ReadAccessToken(String),
    /// Legacy v3 "API Key", sent as the `api_key` query parameter.
    ApiKey(String),
}

impl TmdbAuth {
    /// A read access token is a JWT (`header.payload.signature`, base64url);
    /// a legacy key is a 32-character hex string.
    fn detect(credential: &str) -> Self {
        let c = credential.trim().to_string();
        if c.starts_with("eyJ") && c.split('.').count() == 3 {
            Self::ReadAccessToken(c)
        } else {
            Self::ApiKey(c)
        }
    }
}

impl std::fmt::Debug for TmdbAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ReadAccessToken(_) => "ReadAccessToken([redacted])",
            Self::ApiKey(_) => "ApiKey([redacted])",
        })
    }
}

/// Async TMDB v3 client. Methods return raw JSON bodies for caching.
pub struct TmdbClient {
    http: reqwest::Client,
    auth: TmdbAuth,
}

impl TmdbClient {
    /// `credential` is a Read Access Token (preferred) or a legacy API key;
    /// the kind is detected from its shape.
    pub fn new(credential: String) -> Result<Self, EnrichError> {
        let http = reqwest::Client::builder()
            .user_agent("Nova/0.1")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| EnrichError::Http(e.without_url().to_string()))?;
        Ok(Self {
            http,
            auth: TmdbAuth::detect(&credential),
        })
    }

    /// `Ok(None)` for 404 (unknown ID); other non-success statuses are errors.
    async fn get_optional(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<Option<String>, EnrichError> {
        tokio::time::sleep(REQUEST_SPACING).await;
        let mut request = self.http.get(format!("{API_BASE}{path}")).query(params);
        request = match &self.auth {
            TmdbAuth::ReadAccessToken(token) => request.bearer_auth(token),
            TmdbAuth::ApiKey(key) => request.query(&[("api_key", key.as_str())]),
        };
        // `without_url`: a legacy key lives in the query string.
        let resp = request
            .send()
            .await
            .map_err(|e| EnrichError::Http(e.without_url().to_string()))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(EnrichError::Http(format!("TMDB {status}")));
        }
        resp.text()
            .await
            .map(Some)
            .map_err(|e| EnrichError::Http(e.without_url().to_string()))
    }

    async fn get(&self, path: &str, params: &[(&str, String)]) -> Result<String, EnrichError> {
        self.get_optional(path, params)
            .await?
            .ok_or_else(|| EnrichError::Http("TMDB 404 Not Found".into()))
    }

    /// `/movie/{id}` details; `None` when TMDB doesn't know the ID.
    pub async fn movie_details(&self, id: i64) -> Result<Option<String>, EnrichError> {
        self.get_optional(&format!("/movie/{id}"), &[]).await
    }

    /// `/search/movie`.
    pub async fn search_movie(
        &self,
        title: &str,
        year: Option<i32>,
    ) -> Result<String, EnrichError> {
        let mut params = vec![("query", title.to_string())];
        if let Some(y) = year {
            params.push(("year", y.to_string()));
        }
        self.get("/search/movie", &params).await
    }

    /// `/search/tv`.
    pub async fn search_tv(&self, title: &str) -> Result<String, EnrichError> {
        self.get("/search/tv", &[("query", title.to_string())])
            .await
    }

    /// `/search/multi`: last-resort fallback across movies and TV.
    pub async fn search_multi(&self, query: &str) -> Result<String, EnrichError> {
        self.get("/search/multi", &[("query", query.to_string())])
            .await
    }

    /// `/genre/{movie,tv}/list`. `kind` is `movie` or `tv`.
    pub async fn genre_list(&self, kind: &str) -> Result<String, EnrichError> {
        self.get(&format!("/genre/{kind}/list"), &[]).await
    }
}

// ── Pure parsing ────────────────────────────────────────────────────────────

pub fn parse_search(body: &str) -> Result<Vec<TmdbItem>, EnrichError> {
    let r: SearchResponse =
        serde_json::from_str(body).map_err(|e| EnrichError::Parse(e.to_string()))?;
    Ok(r.results)
}

/// Parse a `/movie/{id}` (or `/tv/{id}`) details body.
pub fn parse_details(body: &str) -> Result<TmdbItem, EnrichError> {
    serde_json::from_str(body).map_err(|e| EnrichError::Parse(e.to_string()))
}

pub fn parse_genres(body: &str) -> Result<HashMap<i64, String>, EnrichError> {
    let r: GenreResponse =
        serde_json::from_str(body).map_err(|e| EnrichError::Parse(e.to_string()))?;
    Ok(r.genres.into_iter().map(|g| (g.id, g.name)).collect())
}

fn image_url(path: &Option<String>, size: &str) -> Option<String> {
    path.as_ref().map(|p| format!("{IMG_BASE}/{size}{p}"))
}

/// Lowercase, keep alphanumerics + spaces, collapse runs of space.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    out.trim().to_string()
}

/// Title similarity in [0,1]: exact normalized match = 1.0; otherwise token
/// Jaccard (shared / union of word sets).
fn title_similarity(a: &str, b: &str) -> f32 {
    let (na, nb) = (normalize(a), normalize(b));
    if na.is_empty() || nb.is_empty() {
        return 0.0;
    }
    if na == nb {
        return 1.0;
    }
    let ta: std::collections::HashSet<&str> = na.split(' ').collect();
    let tb: std::collections::HashSet<&str> = nb.split(' ').collect();
    let inter = ta.intersection(&tb).count() as f32;
    let union = ta.union(&tb).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Best similarity against the localized and the original title.
fn item_similarity(query: &str, item: &TmdbItem) -> f32 {
    [item.display_title(), item.original()]
        .into_iter()
        .flatten()
        .map(|t| title_similarity(query, t))
        .fold(0.0, f32::max)
}

// ── Search ranking and decision ─────────────────────────────────────────────

/// Media kind a result must have to match an entity of that kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Movie,
    Tv,
}

/// Drop results of another media kind. `/search/multi` mixes movies and TV, and
/// a TV ID must never be recorded as a movie match (IDs are per-kind
/// namespaces). Results without a `media_type` come from kind-specific
/// endpoints and are kept.
pub fn retain_kind(items: Vec<TmdbItem>, kind: MediaKind) -> Vec<TmdbItem> {
    let want = match kind {
        MediaKind::Movie => "movie",
        MediaKind::Tv => "tv",
    };
    items
        .into_iter()
        .filter(|i| i.media_type.as_deref().is_none_or(|t| t == want))
        .collect()
}

/// One scored search candidate.
#[derive(Debug, Clone)]
pub struct Scored {
    pub score: f32,
    pub item: TmdbItem,
}

/// Score title results (people are skipped), keep the best score per TMDB ID,
/// best first, ties broken by ID for determinism.
pub fn rank(query_title: &str, query_year: Option<i32>, items: Vec<TmdbItem>) -> Vec<Scored> {
    let mut best: HashMap<i64, Scored> = HashMap::new();
    for item in items {
        if !item.is_title() || item.display_title().is_none() {
            continue;
        }
        let mut score = item_similarity(query_title, &item);
        match (query_year, item.year()) {
            (Some(qy), Some(iy)) if qy == iy => score = (score + 0.15).min(1.0),
            (Some(qy), Some(iy)) if (qy - iy).abs() > 1 => score *= 0.7,
            _ => {}
        }
        match best.get(&item.id) {
            Some(cur) if cur.score >= score => {}
            _ => {
                best.insert(item.id, Scored { score, item });
            }
        }
    }
    let mut ranked: Vec<Scored> = best.into_values().collect();
    ranked.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.item.id.cmp(&b.item.id))
    });
    ranked
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedReason {
    NoCandidates,
    LowConfidence,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Index into the ranked candidates.
    Matched(usize),
    Unresolved(UnresolvedReason),
}

/// Accept the top candidate only when it clears [`MIN_CONFIDENCE`] and no
/// different title is within [`AMBIGUITY_MARGIN`] of it.
pub fn decide(ranked: &[Scored]) -> Decision {
    let Some(top) = ranked.first() else {
        return Decision::Unresolved(UnresolvedReason::NoCandidates);
    };
    if top.score < MIN_CONFIDENCE {
        return Decision::Unresolved(UnresolvedReason::LowConfidence);
    }
    if ranked
        .get(1)
        .is_some_and(|second| top.score - second.score < AMBIGUITY_MARGIN)
    {
        return Decision::Unresolved(UnresolvedReason::Ambiguous);
    }
    Decision::Matched(0)
}

// ── Trusted IDs ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustedCheck {
    /// The details agree with the catalog evidence: accept.
    Consistent,
    /// A known year is more than one year off: the provider ID is wrong.
    Contradicted,
    /// Neither a year nor a similar title confirms it: don't rely on it alone.
    Uncorroborated,
}

/// Check a provider-supplied ID's details against Nova's evidence for the
/// title. A localized title ("Le Parrain") is confirmed by the year alone; an
/// undated one needs a similar title.
pub fn verify_trusted(details: &TmdbItem, titles: &[String], year: Option<i32>) -> TrustedCheck {
    if let (Some(y), Some(dy)) = (year, details.year()) {
        if (y - dy).abs() > 1 {
            return TrustedCheck::Contradicted;
        }
        return TrustedCheck::Consistent;
    }
    let similar = titles
        .iter()
        .any(|t| item_similarity(&nova_core::clean::search_title(t), details) >= 0.5);
    if similar {
        TrustedCheck::Consistent
    } else {
        TrustedCheck::Uncorroborated
    }
}

// ── Provenance ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateEvidence {
    pub tmdb_id: i64,
    pub title: String,
    pub year: Option<i32>,
    /// Score in basis points (0..=10000).
    pub score_bp: u16,
}

impl CandidateEvidence {
    pub fn of(scored: &Scored) -> Self {
        Self {
            tmdb_id: scored.item.id,
            title: scored.item.display_title().unwrap_or_default().to_string(),
            year: scored.item.year(),
            score_bp: score_bp(scored.score),
        }
    }
}

pub fn score_bp(score: f32) -> u16 {
    (score.clamp(0.0, 1.0) * 10_000.0).round() as u16
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedIdEvidence {
    pub tmdb_id: i64,
    /// `None` when TMDB didn't know the ID (404).
    pub check: Option<TrustedCheck>,
}

/// How a result was reached: persisted verbatim with the match so it can be
/// explained, audited and recomputed when [`MATCHER_VERSION`] changes. Holds
/// no credentials and no provider URLs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchEvidence {
    pub matcher_version: u32,
    /// Cleaned query sent to TMDB and the year evidence it was scored with.
    pub query: String,
    pub year: Option<i32>,
    /// Response-cache keys consulted, in order.
    pub requests: Vec<String>,
    pub trusted_id: Option<TrustedIdEvidence>,
    /// Top candidates across every search step, best first.
    pub candidates: Vec<CandidateEvidence>,
    pub min_confidence_bp: u16,
    pub ambiguity_margin_bp: u16,
    /// `matched`, or why the title stayed unresolved.
    pub outcome: String,
}

impl MatchEvidence {
    pub fn new(query: String, year: Option<i32>) -> Self {
        Self {
            matcher_version: MATCHER_VERSION,
            query,
            year,
            requests: Vec::new(),
            trusted_id: None,
            candidates: Vec::new(),
            min_confidence_bp: score_bp(MIN_CONFIDENCE),
            ambiguity_margin_bp: score_bp(AMBIGUITY_MARGIN),
            outcome: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOVIE_BODY: &str = r#"{
        "results": [
            {"id": 603, "title": "The Matrix", "original_title": "The Matrix",
             "release_date": "1999-03-30", "poster_path": "/p.jpg", "backdrop_path": "/b.jpg",
             "overview": "A hacker learns the truth.", "genre_ids": [28, 878]},
            {"id": 1, "title": "The Matrix Revisited", "release_date": "2001-11-18", "genre_ids": [99]},
            {"id": 9, "name": "Keanu Reeves", "media_type": "person"}
        ]
    }"#;
    const GENRE_BODY: &str = r#"{"genres":[{"id":28,"name":"Action"},{"id":878,"name":"Science Fiction"},{"id":99,"name":"Documentary"}]}"#;
    const DETAILS_BODY: &str = r#"{"id": 238, "title": "The Godfather", "original_title": "The Godfather",
        "release_date": "1972-03-14", "poster_path": "/g.jpg", "overview": "Spanning the years...",
        "genres": [{"id": 18, "name": "Drama"}, {"id": 80, "name": "Crime"}], "imdb_id": "tt0068646"}"#;

    fn item(json: &str) -> TmdbItem {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_clear_winner_is_matched_with_its_metadata() {
        let genres = parse_genres(GENRE_BODY).unwrap();
        let ranked = rank("The Matrix", Some(1999), parse_search(MOVIE_BODY).unwrap());
        assert_eq!(ranked.len(), 2, "people are skipped");
        assert_eq!(decide(&ranked), Decision::Matched(0));
        let m = TmdbMatch::from_item(&ranked[0].item, "x", &genres);
        assert_eq!(m.tmdb_id, 603);
        assert_eq!(m.canonical_title, "The Matrix");
        assert_eq!(
            m.poster_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w500/p.jpg")
        );
        assert_eq!(m.genres, vec!["Action", "Science Fiction"]);
    }

    #[test]
    fn low_confidence_and_empty_results_stay_unresolved() {
        let ranked = rank(
            "Breaking Bad",
            Some(2008),
            parse_search(MOVIE_BODY).unwrap(),
        );
        assert_eq!(
            decide(&ranked),
            Decision::Unresolved(UnresolvedReason::LowConfidence)
        );
        assert_eq!(
            decide(&rank("x", None, Vec::new())),
            Decision::Unresolved(UnresolvedReason::NoCandidates)
        );
        // The former "weak, art only" band (0.3..0.6) is no longer accepted.
        let weak = rank(
            "Matrix Reloaded",
            None,
            vec![item(
                r#"{"id": 5, "title": "The Matrix", "poster_path": "/p.jpg"}"#,
            )],
        );
        assert!(weak[0].score >= 0.3 && weak[0].score < MIN_CONFIDENCE);
        assert_eq!(
            decide(&weak),
            Decision::Unresolved(UnresolvedReason::LowConfidence)
        );
    }

    #[test]
    fn two_distinct_titles_scoring_alike_are_ambiguous() {
        // Remakes with no year evidence: both match the title exactly.
        let ranked = rank(
            "Scarface",
            None,
            vec![
                item(r#"{"id": 111, "title": "Scarface", "release_date": "1983-12-09"}"#),
                item(r#"{"id": 112, "title": "Scarface", "release_date": "1932-04-09"}"#),
            ],
        );
        assert_eq!(
            decide(&ranked),
            Decision::Unresolved(UnresolvedReason::Ambiguous)
        );
        // A year disambiguates them.
        let ranked = rank(
            "Scarface",
            Some(1983),
            vec![
                item(r#"{"id": 111, "title": "Scarface", "release_date": "1983-12-09"}"#),
                item(r#"{"id": 112, "title": "Scarface", "release_date": "1932-04-09"}"#),
            ],
        );
        assert_eq!(decide(&ranked), Decision::Matched(0));
        assert_eq!(ranked[0].item.id, 111);
    }

    #[test]
    fn duplicate_ids_across_searches_are_not_ambiguous() {
        let a = item(r#"{"id": 603, "title": "The Matrix", "release_date": "1999-03-30"}"#);
        let ranked = rank("The Matrix", Some(1999), vec![a.clone(), a]);
        assert_eq!(ranked.len(), 1);
        assert_eq!(decide(&ranked), Decision::Matched(0));
    }

    #[test]
    fn multi_search_results_are_filtered_to_the_entity_kind() {
        let items = parse_search(
            r#"{"results": [
                {"id": 1, "title": "Fargo", "media_type": "movie"},
                {"id": 2, "name": "Fargo", "media_type": "tv"},
                {"id": 3, "name": "Someone", "media_type": "person"},
                {"id": 4, "title": "Fargo"}
            ]}"#,
        )
        .unwrap();
        let movies: Vec<i64> = retain_kind(items.clone(), MediaKind::Movie)
            .iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(movies, vec![1, 4]);
        let tv: Vec<i64> = retain_kind(items, MediaKind::Tv)
            .iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(tv, vec![2, 4]);
    }

    #[test]
    fn ranking_is_deterministic() {
        let items = || {
            vec![
                item(r#"{"id": 2, "title": "Heat"}"#),
                item(r#"{"id": 1, "title": "Heat"}"#),
            ]
        };
        let a: Vec<i64> = rank("Heat", None, items())
            .iter()
            .map(|s| s.item.id)
            .collect();
        let mut rev = items();
        rev.reverse();
        let b: Vec<i64> = rank("Heat", None, rev).iter().map(|s| s.item.id).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn original_titles_count_for_similarity() {
        let ranked = rank(
            "Le fabuleux destin d'Amélie Poulain",
            Some(2001),
            vec![item(
                r#"{"id": 194, "title": "Amélie", "original_title": "Le Fabuleux Destin d'Amélie Poulain", "release_date": "2001-04-25"}"#,
            )],
        );
        assert_eq!(decide(&ranked), Decision::Matched(0));
    }

    #[test]
    fn trusted_ids_need_corroboration() {
        let details = parse_details(DETAILS_BODY).unwrap();
        let fr = vec!["Le Parrain".to_string()];
        assert_eq!(
            verify_trusted(&details, &fr, Some(1972)),
            TrustedCheck::Consistent
        );
        assert_eq!(
            verify_trusted(&details, &fr, Some(1973)),
            TrustedCheck::Consistent
        );
        assert_eq!(
            verify_trusted(&details, &fr, Some(1990)),
            TrustedCheck::Contradicted
        );
        assert_eq!(
            verify_trusted(&details, &fr, None),
            TrustedCheck::Uncorroborated
        );
        let en = vec!["EN - The Godfather FHD".to_string()];
        assert_eq!(
            verify_trusted(&details, &en, None),
            TrustedCheck::Consistent
        );
        let m = TmdbMatch::from_item(&details, "x", &HashMap::new());
        assert_eq!(
            m.genres,
            vec!["Drama", "Crime"],
            "details carry genre objects"
        );
    }

    #[test]
    fn credentials_are_detected_and_never_printed() {
        let token = "eyJhbGciOiJIUzI1NiJ9.eyJhdWQiOiJ4In0.c2lnbmF0dXJl";
        assert!(matches!(
            TmdbAuth::detect(token),
            TmdbAuth::ReadAccessToken(_)
        ));
        let key = "0123456789abcdef0123456789abcdef";
        assert!(matches!(TmdbAuth::detect(key), TmdbAuth::ApiKey(_)));
        for auth in [TmdbAuth::detect(token), TmdbAuth::detect(key)] {
            let debug = format!("{auth:?}");
            assert!(!debug.contains("eyJ") && !debug.contains("0123"), "{debug}");
        }
    }

    #[test]
    fn evidence_serializes_without_secrets() {
        let ranked = rank("The Matrix", Some(1999), parse_search(MOVIE_BODY).unwrap());
        let mut evidence = MatchEvidence::new("The Matrix".into(), Some(1999));
        evidence.requests.push("movie::the matrix::1999".into());
        evidence.candidates = ranked.iter().map(CandidateEvidence::of).collect();
        evidence.outcome = "matched".into();
        let json = serde_json::to_string(&evidence).unwrap();
        assert!(json.contains("\"matcherVersion\":2"));
        assert!(json.contains("\"scoreBp\":10000"));
        assert!(!json.contains("api_key") && !json.contains("http"));
    }
}
