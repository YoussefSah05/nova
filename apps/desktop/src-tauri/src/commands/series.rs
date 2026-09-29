//! Series browse API for the Shows section: filters shared by the list and
//! count commands, the genre facet and the week's schedule (series whose next
//! episode TMDB has scheduled within a date range). Cards carry facts and the
//! profile's progress from stored evidence only.

use nova_core::SeriesSummary;
use nova_storage::{FacetCount, SeriesFilters, SeriesRepository, SeriesStatusFilter};
use serde::Deserialize;
use tauri::State;

use crate::ipc::ApiError;
use crate::state::AppState;

use super::{normalize_filter, require_profile_pool, ContentFilters};

/// Series status filter as the frontend sends it.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SeriesStatus {
    Airing,
    Ended,
}

/// Owned, normalized series filters (borrowed by the repository).
pub(super) struct OwnedSeriesFilters {
    category: Option<String>,
    query: Option<String>,
    favorites_only: bool,
    watchlist_only: bool,
    genre: Option<String>,
    status: Option<SeriesStatus>,
}

impl From<ContentFilters> for OwnedSeriesFilters {
    fn from(f: ContentFilters) -> Self {
        Self {
            category: normalize_filter(f.category),
            query: normalize_filter(f.query),
            favorites_only: f.favorites_only,
            watchlist_only: f.watchlist_only,
            genre: normalize_filter(f.genre),
            status: f.status,
        }
    }
}

impl OwnedSeriesFilters {
    pub(super) fn borrow(&self) -> SeriesFilters<'_> {
        SeriesFilters {
            category: self.category.as_deref(),
            query: self.query.as_deref(),
            favorites_only: self.favorites_only,
            watchlist_only: self.watchlist_only,
            genre: self.genre.as_deref(),
            status: self.status.map(|s| match s {
                SeriesStatus::Airing => SeriesStatusFilter::Airing,
                SeriesStatus::Ended => SeriesStatusFilter::Ended,
            }),
            airs_between: None,
        }
    }
}

/// Nova's genres among the playlist's series, most common first.
#[tauri::command]
pub async fn list_series_genres(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<FacetCount>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    SeriesRepository::new(&pool)
        .series_genres(source_id)
        .await
        .map_err(Into::into)
}

/// Most titles one card lookup accepts.
const CARDS_LIMIT: usize = 50;

/// Fresh cards (facts and progress) for series titles of the playlist, in
/// any order: the Continue module and a series page read them.
#[tauri::command]
pub async fn list_series_cards(
    profile_id: String,
    source_id: i64,
    titles: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<SeriesSummary>, ApiError> {
    if titles.len() > CARDS_LIMIT {
        return Err(ApiError::InvalidInput(format!(
            "at most {CARDS_LIMIT} titles per lookup"
        )));
    }
    let pool = require_profile_pool(&profile_id, &state).await?;
    SeriesRepository::new(&pool)
        .list_by_titles(source_id, &titles)
        .await
        .map_err(Into::into)
}

/// A calendar date, `YYYY-MM-DD` (compared as text against TMDB air dates).
fn is_date(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Most series a schedule returns.
const SCHEDULE_LIMIT: i64 = 60;

/// Series of the playlist whose next episode airs between `from` and `to`
/// (inclusive, the viewer's local dates), soonest first. Only series whose
/// TMDB facts were fetched (their page opened or warmed) are known.
#[tauri::command]
pub async fn list_series_schedule(
    profile_id: String,
    source_id: i64,
    from: String,
    to: String,
    state: State<'_, AppState>,
) -> Result<Vec<SeriesSummary>, ApiError> {
    if !is_date(&from) || !is_date(&to) || from > to {
        return Err(ApiError::InvalidInput(
            "dates must be YYYY-MM-DD, from ≤ to".into(),
        ));
    }
    let pool = require_profile_pool(&profile_id, &state).await?;
    SeriesRepository::new(&pool)
        .series_schedule(source_id, &from, &to, SCHEDULE_LIMIT)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::is_date;

    /// The browse queries read stored TMDB facts by JSON path; these are the
    /// keys `TitleFacts` is stored under.
    #[test]
    fn the_browse_reads_the_keys_title_facts_are_stored_under() {
        use nova_enrich::tmdb_facts::{CollectionRef, TitleFacts, UpcomingEpisode};
        let facts = TitleFacts {
            rating: Some(9.0),
            vote_count: Some(10),
            runtime_min: Some(50),
            status: Some("Ended".into()),
            first_year: Some(2008),
            last_year: Some(2013),
            next_episode: Some(UpcomingEpisode {
                season: 3,
                episode: 1,
                name: Some("Return".into()),
                air_date: "2026-10-01".into(),
            }),
            collection: Some(CollectionRef {
                id: 1,
                name: "Saga".into(),
            }),
            ..Default::default()
        };
        let v: serde_json::Value = serde_json::from_str(&facts.to_json()).unwrap();
        for path in [
            "/rating",
            "/voteCount",
            "/runtimeMin",
            "/status",
            "/firstYear",
            "/lastYear",
            "/nextEpisode/season",
            "/nextEpisode/episode",
            "/nextEpisode/name",
            "/nextEpisode/airDate",
            "/collection/id",
        ] {
            assert!(v.pointer(path).is_some(), "{path} missing from {v}");
        }
    }

    #[test]
    fn schedule_dates_are_plain_calendar_dates() {
        assert!(is_date("2026-09-29"));
        for bad in ["2026-9-29", "2026/09/29", "20260929xx", "2026-09-2a", ""] {
            assert!(!is_date(bad), "{bad}");
        }
    }
}
