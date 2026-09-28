//! Descriptive facts a provider ships with its catalog (Xtream panels send
//! plots, cast, ratings, durations, trailers and backdrops). They make detail
//! pages useful offline, without TMDB, and rank below a confident TMDB match
//! (see `master-plan.md` M4.6).
//!
//! Panels are inconsistent: ratings arrive as `7.5`, `"7,5"`, `"0"` or out of
//! 5; durations as seconds or `01:42:10`; trailers as a YouTube id or a full
//! URL; backdrops as a string or a list; empty values as `""`, `"N/A"` or
//! `"null"`. The normalizers here turn all of that into clean values or
//! nothing.

use serde::{Deserialize, Serialize};

/// Normalized provider facts for a movie, series or episode. Every field is
/// optional; an all-empty value is not stored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plot: Option<String>,
    /// Comma-separated cast, as the provider lists it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cast: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub director: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    /// `YYYY-MM-DD` or `YYYY`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    /// Out of 10.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<f32>,
    /// Runtime (movie/episode) or typical episode runtime (series), seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_sec: Option<i64>,
    /// YouTube video id of a trailer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trailer_youtube: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backdrop_url: Option<String>,
}

impl ProviderFacts {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// `None` when empty, so empty facts are never stored.
    pub fn non_empty(self) -> Option<Self> {
        (!self.is_empty()).then_some(self)
    }

    /// Serialized form for storage.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Stored form back; unreadable or empty values are `None`.
    pub fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str::<Self>(json).ok()?.non_empty()
    }

    /// Fill this value's gaps from `other` (this value wins).
    pub fn or(mut self, other: Self) -> Self {
        self.plot = self.plot.or(other.plot);
        self.cast = self.cast.or(other.cast);
        self.director = self.director.or(other.director);
        self.genre = self.genre.or(other.genre);
        self.release_date = self.release_date.or(other.release_date);
        self.rating = self.rating.or(other.rating);
        self.duration_sec = self.duration_sec.or(other.duration_sec);
        self.trailer_youtube = self.trailer_youtube.or(other.trailer_youtube);
        self.backdrop_url = self.backdrop_url.or(other.backdrop_url);
        self
    }
}

/// Longest plot kept, in characters.
const MAX_TEXT: usize = 4000;

/// Trimmed, whitespace-collapsed text; placeholders are `None`.
pub fn clean_text(raw: &str) -> Option<String> {
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = text.to_ascii_lowercase();
    if text.is_empty()
        || matches!(
            lower.as_str(),
            "n/a" | "na" | "null" | "none" | "-" | "0" | "unknown"
        )
    {
        return None;
    }
    Some(text.chars().take(MAX_TEXT).collect())
}

/// A rating out of 10 from a number or numeric string; `out_of_five` doubles
/// it. Zero (the usual "no rating") and out-of-range values are `None`.
pub fn rating(raw: &str, out_of_five: bool) -> Option<f32> {
    let value: f32 = raw.trim().replace(',', ".").parse().ok()?;
    let value = if out_of_five { value * 2.0 } else { value };
    (value > 0.0 && value <= 10.0 && value.is_finite()).then(|| (value * 10.0).round() / 10.0)
}

/// Seconds from `5400`, `"5400"`, `"01:30:00"` or `"90:00"`, within 1 s–10 h.
pub fn duration_sec(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let secs = if raw.contains(':') {
        let parts: Vec<i64> = raw
            .split(':')
            .map(|p| p.trim().parse::<i64>())
            .collect::<Result<_, _>>()
            .ok()?;
        match parts.as_slice() {
            [h, m, s] => h * 3600 + m * 60 + s,
            [m, s] => m * 60 + s,
            _ => return None,
        }
    } else {
        raw.parse::<f64>().ok()? as i64
    };
    (1..=36_000).contains(&secs).then_some(secs)
}

/// `YYYY-MM-DD` (or `YYYY`) from a provider date; anything else is `None`.
pub fn release_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let date = raw.get(..10).unwrap_or(raw);
    let bytes = date.as_bytes();
    let digits = |r: std::ops::Range<usize>| bytes[r].iter().all(u8::is_ascii_digit);
    let year_ok = |y: &str| y.parse::<u16>().is_ok_and(|y| (1880..=2100).contains(&y));
    if date.len() == 10
        && digits(0..4)
        && bytes[4] == b'-'
        && digits(5..7)
        && bytes[7] == b'-'
        && digits(8..10)
    {
        return (year_ok(&date[..4]) && &date[5..] != "00-00").then(|| date.to_string());
    }
    let year = raw.get(..4)?;
    (raw.len() == 4 && year_ok(year)).then(|| year.to_string())
}

fn is_youtube_id(s: &str) -> bool {
    s.len() == 11
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The YouTube video id in a bare id or a watch / short / embed URL.
pub fn youtube_id(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if is_youtube_id(raw) {
        return Some(raw.to_string());
    }
    let lower = raw.to_ascii_lowercase();
    if !lower.contains("youtube.com") && !lower.contains("youtu.be") {
        return None;
    }
    let candidate = if let Some(i) = raw.find("v=") {
        &raw[i + 2..]
    } else if let Some(i) = raw.find("youtu.be/") {
        &raw[i + 9..]
    } else if let Some(i) = raw.find("/embed/") {
        &raw[i + 7..]
    } else if let Some(i) = raw.find("/shorts/") {
        &raw[i + 8..]
    } else {
        return None;
    };
    let id: String = candidate
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    is_youtube_id(&id).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_text_and_drops_placeholders() {
        assert_eq!(
            clean_text("  A   story\n about  things "),
            Some("A story about things".into())
        );
        for p in ["", "  ", "N/A", "null", "0", "-"] {
            assert_eq!(clean_text(p), None, "{p:?}");
        }
    }

    #[test]
    fn normalizes_ratings() {
        assert_eq!(rating("7.5", false), Some(7.5));
        assert_eq!(rating("7,54", false), Some(7.5));
        assert_eq!(rating("3.5", true), Some(7.0));
        assert_eq!(rating("0", false), None);
        assert_eq!(rating("11", false), None);
        assert_eq!(rating("abc", false), None);
    }

    #[test]
    fn parses_durations() {
        assert_eq!(duration_sec("5400"), Some(5400));
        assert_eq!(duration_sec("01:30:00"), Some(5400));
        assert_eq!(duration_sec("45:10"), Some(2710));
        assert_eq!(duration_sec("0"), None);
        assert_eq!(duration_sec("99:00:00"), None);
        assert_eq!(duration_sec("soon"), None);
    }

    #[test]
    fn keeps_only_real_dates() {
        assert_eq!(release_date("2019-10-04"), Some("2019-10-04".into()));
        assert_eq!(
            release_date("2019-10-04 00:00:00"),
            Some("2019-10-04".into())
        );
        assert_eq!(release_date("2019"), Some("2019".into()));
        assert_eq!(release_date("0000-00-00"), None);
        assert_eq!(release_date("04/10/2019"), None);
        assert_eq!(release_date(""), None);
    }

    #[test]
    fn extracts_youtube_ids() {
        assert_eq!(youtube_id("dQw4w9WgXcQ"), Some("dQw4w9WgXcQ".into()));
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ?rel=0",
        ] {
            assert_eq!(youtube_id(url), Some("dQw4w9WgXcQ".into()), "{url}");
        }
        assert_eq!(youtube_id("https://vimeo.com/123"), None);
        assert_eq!(youtube_id("short"), None);
    }

    #[test]
    fn empty_facts_are_not_stored_and_round_trip() {
        assert!(ProviderFacts::default().non_empty().is_none());
        let facts = ProviderFacts {
            plot: Some("P".into()),
            rating: Some(7.5),
            ..ProviderFacts::default()
        };
        assert_eq!(facts.to_json(), r#"{"plot":"P","rating":7.5}"#);
        assert_eq!(
            ProviderFacts::from_json(&facts.to_json()),
            Some(facts.clone())
        );
        assert_eq!(ProviderFacts::from_json("{}"), None);
        let merged = ProviderFacts {
            rating: None,
            ..facts.clone()
        }
        .or(ProviderFacts {
            rating: Some(6.0),
            plot: Some("other".into()),
            ..ProviderFacts::default()
        });
        assert_eq!(
            (merged.plot.as_deref(), merged.rating),
            (Some("P"), Some(6.0))
        );
    }
}
