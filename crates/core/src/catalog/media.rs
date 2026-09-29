//! Canonical media read/action contracts and compiler binding keys.
//!
//! `SourceRecord != CanonicalEntity != StreamVariant`: these DTOs are what the
//! application layer exposes for canonical media. They are secret-free by
//! construction — no stream URLs, source URLs, credentials or provider row
//! locators except the transient playback target resolved by Rust.

use serde::{Deserialize, Serialize};

use super::fingerprint::binding_fingerprint;
use super::{ContentKind, EntityId, VariantId};

/// Binding key from a provider-scoped stable ID (e.g. `xtream:vod:123`).
/// Survives credential rotation and URL changes of the same upstream item.
pub fn upstream_binding_key(upstream_id: &str) -> String {
    format!(
        "u1-{}",
        binding_fingerprint("upstream", upstream_id.as_bytes()).to_hex()
    )
}

/// Binding key from a stream locator. Used when no stable ID is known, and to
/// re-bind variants compiled before one was available.
pub fn locator_binding_key(locator: &str) -> String {
    format!(
        "l1-{}",
        binding_fingerprint("locator", locator.as_bytes()).to_hex()
    )
}

/// Preferred binding key for a source record.
pub fn binding_key(upstream_id: Option<&str>, locator: &str) -> String {
    match upstream_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => upstream_binding_key(id),
        None => locator_binding_key(locator),
    }
}

/// Durable, secret-free key for an episode stream, stable across refreshes.
/// Xtream episode URLs (`…/series/<user>/<pass>/<episode_id>.<ext>`) key on
/// the provider's episode ID, so a credential rotation keeps the key; any
/// other URL keys on its locator digest.
pub fn episode_binding_key(stream_url: &str) -> String {
    let path = stream_url.split(['?', '#']).next().unwrap_or(stream_url);
    let segments: Vec<&str> = path.rsplit('/').take(4).collect();
    if let [file, _pass, _user, "series"] = segments[..] {
        let id = file.split('.').next().unwrap_or("");
        if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
            return upstream_binding_key(&format!("xtream:episode:{id}"));
        }
    }
    locator_binding_key(stream_url)
}

/// The provider series ID carried by an Xtream catalog sentinel
/// (`xtream://series/<id>`), the only row a series has until it is opened.
pub fn xtream_series_id(stream_url: &str) -> Option<&str> {
    stream_url
        .strip_prefix("xtream://series/")
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

/// Durable, secret-free key binding a source's series to its canonical
/// entity: the provider's series ID when known (so a provider rename keeps
/// identity), else the exact parsed title (M3U has no series ID; a renamed
/// M3U series becomes a new entity, as a changed M3U movie URL does).
pub fn series_binding_key(provider_series_id: Option<&str>, series_title: &str) -> String {
    match provider_series_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(id) => upstream_binding_key(&format!("xtream:series:{id}")),
        None => format!(
            "t1-{}",
            binding_fingerprint("series-title", series_title.as_bytes()).to_hex()
        ),
    }
}

/// A canonical media card. Artwork URLs are public display assets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalMediaSummary {
    pub entity_id: EntityId,
    pub kind: ContentKind,
    pub title: String,
    /// Raw provider title of the representative variant (quality/version
    /// labels such as "FHD" or "MULTI"); display only, never identity.
    pub source_title: String,
    pub year: Option<i64>,
    pub category: Option<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub overview: Option<String>,
    pub is_favorite: bool,
    pub is_in_watchlist: bool,
    pub added_at: Option<i64>,
    /// Enrichment similarity cluster of the representative variant: a
    /// presentation grouping hint only, never identity (not entity resolution).
    pub dedup_group: Option<String>,
    /// What a card shows beyond the title (genres, runtime, rating, badges).
    #[serde(flatten, default)]
    pub facts: CardFacts,
}

/// Card-level facts, all derived from stored evidence (no network).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CardFacts {
    /// TMDB genres when matched, else Nova's genre.
    pub genres: Vec<String>,
    pub runtime_min: Option<i64>,
    /// Out of 10.
    pub rating: Option<f32>,
    /// Where the rating comes from: `tmdb` or `provider`.
    pub rating_source: Option<String>,
    /// The best picture quality and the audio versions across the title's
    /// sources in this playlist (`4K`, `FHD`, `MULTI`, `VF`).
    pub badges: Vec<String>,
    /// How many sources of the title this playlist has.
    pub version_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantSummary {
    pub variant_id: VariantId,
    pub source_id: i64,
    pub source_name: String,
    pub label: String,
    /// False while the provider no longer lists this variant.
    pub available: bool,
    /// Picture quality and audio version tags (`FHD`, `MULTI`).
    #[serde(default)]
    pub badges: Vec<String>,
    /// Failed more recently than it last played (ranked last).
    #[serde(default)]
    pub recently_failed: bool,
    /// The source Nova plays by default (the variant selector's first pick).
    #[serde(default)]
    pub picked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeState {
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalMediaDetail {
    #[serde(flatten)]
    pub summary: CanonicalMediaSummary,
    pub variants: Vec<VariantSummary>,
    pub resume: Option<ResumeState>,
    /// TMDB's original-language title, when it differs from the display one.
    #[serde(default)]
    pub original_title: Option<String>,
    /// `YYYY-MM-DD` (or a year) from the provider.
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub vote_count: Option<i64>,
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// Rust-selected playback target. `media_kind`/`media_id` are a transient
/// locator for the current catalog generation, used by the existing player;
/// durable state is keyed by the entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoviePlaybackTarget {
    pub entity_id: EntityId,
    pub variant_id: VariantId,
    pub media_kind: String,
    pub media_id: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_keys_prefer_the_provider_id_and_never_collide_across_namespaces() {
        assert_eq!(xtream_series_id("xtream://series/42"), Some("42"));
        for not_a_sentinel in [
            "xtream://series/",
            "xtream://series/4a",
            "http://h/series/u/p/42.mkv",
        ] {
            assert_eq!(xtream_series_id(not_a_sentinel), None, "{not_a_sentinel}");
        }
        // A rename keeps the ID key; the title key follows the exact title.
        assert_eq!(
            series_binding_key(Some("42"), "FR Show"),
            series_binding_key(Some("42"), "Show")
        );
        assert_ne!(
            series_binding_key(None, "FR Show"),
            series_binding_key(None, "Show")
        );
        // Distinct namespaces: a title can't impersonate an ID, nor a movie.
        assert_ne!(
            series_binding_key(Some("42"), "x"),
            series_binding_key(None, "42")
        );
        assert_ne!(
            series_binding_key(Some("42"), "x"),
            upstream_binding_key("xtream:vod:42")
        );
        assert!(series_binding_key(Some("42"), "x").starts_with("u1-"));
        assert!(series_binding_key(None, "Show").starts_with("t1-"));
    }

    #[test]
    fn binding_keys_are_stable_namespaced_and_secret_free() {
        let url = "http://host/movie/alice/s3cret/42.mkv";
        let locator = locator_binding_key(url);
        assert_eq!(locator, locator_binding_key(url));
        assert!(locator.starts_with("l1-") && locator.len() == 67);
        assert!(!locator.contains("s3cret") && !locator.contains("alice"));
        // A stable ID is independent of the (credential-bearing) locator.
        let rotated = "http://host/movie/alice/n3w/42.mkv";
        assert_eq!(
            binding_key(Some("xtream:vod:42"), url),
            binding_key(Some("xtream:vod:42"), rotated)
        );
        assert_ne!(binding_key(None, url), binding_key(None, rotated));
        // Namespaces never collide even for identical input bytes.
        assert_ne!(upstream_binding_key("x"), locator_binding_key("x"));
        assert_eq!(binding_key(Some("  "), url), locator);
    }

    #[test]
    fn episode_keys_survive_credential_rotation_and_hide_secrets() {
        let old = episode_binding_key("http://h:80/series/alice/s3cret/293257.mkv");
        let rotated = episode_binding_key("http://h:80/series/alice/n3w/293257.mkv");
        assert_eq!(old, rotated);
        assert_ne!(
            old,
            episode_binding_key("http://h:80/series/alice/s3cret/293258.mkv")
        );
        assert!(!old.contains("s3cret") && old.starts_with("u1-"));
        // Not an Xtream episode URL: keyed on the whole locator.
        let m3u = episode_binding_key("https://cdn.example/show/s01e02.mp4");
        assert!(m3u.starts_with("l1-"));
        assert_eq!(
            m3u,
            episode_binding_key("https://cdn.example/show/s01e02.mp4")
        );
    }

    #[test]
    fn dto_serialization_contains_no_locators() {
        let summary = CanonicalMediaSummary {
            entity_id: EntityId::new("mov-1").unwrap(),
            kind: ContentKind::Movie,
            title: "Heat".into(),
            source_title: "EN - Heat (1995) FHD".into(),
            year: Some(1995),
            category: None,
            poster_url: None,
            backdrop_url: None,
            overview: None,
            is_favorite: true,
            is_in_watchlist: false,
            added_at: None,
            dedup_group: None,
            facts: CardFacts {
                badges: vec!["FHD".into()],
                ..CardFacts::default()
            },
        };
        let detail = CanonicalMediaDetail {
            summary,
            variants: vec![VariantSummary {
                variant_id: VariantId::new("var-1").unwrap(),
                source_id: 1,
                source_name: "Provider".into(),
                label: "EN - Heat (1995) FHD".into(),
                available: true,
                badges: vec!["FHD".into()],
                recently_failed: false,
                picked: true,
            }],
            resume: None,
            original_title: None,
            release_date: None,
            vote_count: None,
            keywords: vec![],
        };
        let json = serde_json::to_string(&detail).unwrap();
        assert!(json.contains("\"entityId\":\"mov-1\""));
        assert!(json.contains("\"kind\":\"movie\""));
        assert!(json.contains("\"badges\":[\"FHD\"]") && json.contains("\"versionCount\":0"));
        for forbidden in ["streamUrl", "url\"", "http"] {
            assert!(!json.contains(forbidden), "{forbidden} in {json}");
        }
    }
}
