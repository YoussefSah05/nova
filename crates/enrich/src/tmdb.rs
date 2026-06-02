//! TMDB enrichment (Phase C). A thin async client over the TMDB v3 API plus the
//! pure matching/parsing logic used to turn a dirty catalog title into a canonical
//! title, poster, backdrop, overview and real genres.
//!
//! The network surface ([`TmdbClient`]) returns raw JSON bodies so the caller can
//! cache them verbatim (TMDB ToS permits caching); all selection logic
//! ([`parse_search`], [`best_match`], …) is pure over those bodies and unit-tested
//! without a key or network (CI hits nothing).

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::EnrichError;

const API_BASE: &str = "https://api.themoviedb.org/3";
const IMG_BASE: &str = "https://image.tmdb.org/t/p";
/// ~40 req/s — comfortably under TMDB's limit; calls are sequential so a fixed
/// inter-request delay is enough (no token bucket needed).
const REQUEST_SPACING: Duration = Duration::from_millis(25);
/// Below this combined title+year confidence we leave the row unmatched rather
/// than guess (a Phase D / LLM residual later).
pub const MIN_CONFIDENCE: f32 = 0.6;

// ── API response shapes (deserialize only what we use) ──────────────────────────

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

#[derive(Debug, Clone, Deserialize)]
struct GenreEntry {
    id: i64,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TmdbItem {
    pub id: i64,
    #[serde(default)]
    title: Option<String>, // movies
    #[serde(default)]
    name: Option<String>, // tv
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
    #[serde(default)]
    genre_ids: Vec<i64>,
}

impl TmdbItem {
    fn display_title(&self) -> Option<&str> {
        self.title.as_deref().or(self.name.as_deref())
    }
    fn year(&self) -> Option<i32> {
        let d = self
            .release_date
            .as_deref()
            .or(self.first_air_date.as_deref())?;
        d.get(0..4)?.parse().ok()
    }
}

/// The enriched metadata chosen for one catalog entry.
#[derive(Debug, Clone, PartialEq)]
pub struct TmdbMatch {
    pub tmdb_id: i64,
    pub canonical_title: String,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub overview: Option<String>,
    /// Genre names, mapped from `genre_ids` via the TMDB genre list.
    pub genres: Vec<String>,
}

// ── Client ──────────────────────────────────────────────────────────────────

/// Async TMDB v3 client. Methods return raw JSON bodies for caching; parse them
/// with [`parse_search`] / [`parse_genres`].
pub struct TmdbClient {
    http: reqwest::Client,
    api_key: String,
}

impl TmdbClient {
    pub fn new(api_key: String) -> Result<Self, EnrichError> {
        let http = reqwest::Client::builder()
            .user_agent("Nova/0.1")
            .build()
            .map_err(|e| EnrichError::Http(e.to_string()))?;
        Ok(Self { http, api_key })
    }

    async fn get(&self, path: &str, params: &[(&str, String)]) -> Result<String, EnrichError> {
        // Sequential calls; a fixed spacing keeps us well under the rate limit.
        tokio::time::sleep(REQUEST_SPACING).await;
        let resp = self
            .http
            .get(format!("{API_BASE}{path}"))
            .query(&[("api_key", self.api_key.as_str())])
            .query(params)
            .send()
            .await
            .map_err(|e| EnrichError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(EnrichError::Http(format!(
                "TMDB {} on {path}",
                resp.status()
            )));
        }
        resp.text()
            .await
            .map_err(|e| EnrichError::Http(e.to_string()))
    }

    /// `/search/movie` — raw JSON body.
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

    /// `/search/tv` — raw JSON body.
    pub async fn search_tv(&self, title: &str) -> Result<String, EnrichError> {
        self.get("/search/tv", &[("query", title.to_string())])
            .await
    }

    /// `/genre/{movie,tv}/list` — raw JSON body. `kind` is `movie` or `tv`.
    pub async fn genre_list(&self, kind: &str) -> Result<String, EnrichError> {
        self.get(&format!("/genre/{kind}/list"), &[]).await
    }
}

// ── Pure parsing + matching ─────────────────────────────────────────────────

/// Parse a `/search/*` body into its result list.
pub fn parse_search(body: &str) -> Result<Vec<TmdbItem>, EnrichError> {
    let r: SearchResponse =
        serde_json::from_str(body).map_err(|e| EnrichError::Parse(e.to_string()))?;
    Ok(r.results)
}

/// Parse a `/genre/*/list` body into an id → name map.
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

/// Pick the best candidate for `query_title`/`query_year`. Confidence combines
/// title similarity (TMDB already popularity-ranks results) with a year bonus.
/// Returns `None` if nothing clears `min_confidence`.
pub fn best_match(
    query_title: &str,
    query_year: Option<i32>,
    items: &[TmdbItem],
    genres: &HashMap<i64, String>,
    min_confidence: f32,
) -> Option<TmdbMatch> {
    let mut best: Option<(f32, &TmdbItem)> = None;
    for item in items {
        let Some(cand_title) = item.display_title() else {
            continue;
        };
        let mut score = title_similarity(query_title, cand_title);
        match (query_year, item.year()) {
            (Some(qy), Some(iy)) if qy == iy => score = (score + 0.15).min(1.0),
            (Some(qy), Some(iy)) if (qy - iy).abs() > 1 => score *= 0.7,
            _ => {}
        }
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, item));
        }
    }
    let (score, item) = best?;
    if score < min_confidence {
        return None;
    }
    Some(TmdbMatch {
        tmdb_id: item.id,
        canonical_title: item.display_title().unwrap_or(query_title).to_string(),
        poster_url: image_url(&item.poster_path, "w500"),
        backdrop_url: image_url(&item.backdrop_path, "w1280"),
        overview: item.overview.clone().filter(|o| !o.trim().is_empty()),
        genres: item
            .genre_ids
            .iter()
            .filter_map(|id| genres.get(id).cloned())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOVIE_BODY: &str = r#"{
        "results": [
            {"id": 603, "title": "The Matrix", "release_date": "1999-03-30",
             "poster_path": "/p.jpg", "backdrop_path": "/b.jpg",
             "overview": "A hacker learns the truth.", "genre_ids": [28, 878]},
            {"id": 1, "title": "The Matrix Revisited", "release_date": "2001-11-18", "genre_ids": [99]}
        ]
    }"#;
    const GENRE_BODY: &str = r#"{"genres":[{"id":28,"name":"Action"},{"id":878,"name":"Science Fiction"},{"id":99,"name":"Documentary"}]}"#;

    #[test]
    fn parses_and_maps_genres() {
        let g = parse_genres(GENRE_BODY).unwrap();
        assert_eq!(g.get(&878).map(String::as_str), Some("Science Fiction"));
        let items = parse_search(MOVIE_BODY).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn best_match_prefers_exact_title_and_year() {
        let items = parse_search(MOVIE_BODY).unwrap();
        let genres = parse_genres(GENRE_BODY).unwrap();
        let m = best_match("The Matrix", Some(1999), &items, &genres, MIN_CONFIDENCE).unwrap();
        assert_eq!(m.tmdb_id, 603);
        assert_eq!(m.canonical_title, "The Matrix");
        assert_eq!(
            m.poster_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w500/p.jpg")
        );
        assert_eq!(
            m.backdrop_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w1280/b.jpg")
        );
        assert_eq!(
            m.genres,
            vec!["Action".to_string(), "Science Fiction".to_string()]
        );
    }

    #[test]
    fn best_match_rejects_low_confidence() {
        let items = parse_search(MOVIE_BODY).unwrap();
        let genres = parse_genres(GENRE_BODY).unwrap();
        // Nothing resembling either candidate → below floor → no match.
        assert!(best_match("Breaking Bad", Some(2008), &items, &genres, MIN_CONFIDENCE).is_none());
    }

    #[test]
    fn title_similarity_handles_decorated_names() {
        assert!((title_similarity("The Matrix", "the matrix!!!") - 1.0).abs() < 1e-6);
        assert!(title_similarity("The Matrix", "Inception") < 0.5);
    }
}
