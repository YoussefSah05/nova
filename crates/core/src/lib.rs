//! Core domain types, traits, and errors shared across all Nova crates.

use serde::{Deserialize, Serialize};

pub mod classify;
pub mod clean;
pub mod region;
pub use classify::normalize_category;
pub use clean::polish_title;
pub use region::region_of;

// ── Profiles ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub avatar_color: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// At-a-glance, per-profile summary surfaced on the profile picker. Aggregated
/// from a profile's own DB (accent + sources + counts + continue-watching) so the
/// entry screen can show recency, stats and resume without N separate IPC calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileOverview {
    /// The profile's saved accent theme id (e.g. "amber"); defaults applied on read.
    pub accent_color: String,
    pub source_count: i64,
    pub item_count: i64,
    /// Number of in-progress items (capped at the query limit — for display only).
    pub continue_count: i64,
    /// Title of the most-recent in-progress item, if any (the resume hint).
    pub last_title: Option<String>,
}

// ── Media classification ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Live,
    Movie,
    Series,
}

// ── Parsed M3U entry (output of crates/m3u parser) ───────────────────────────

#[derive(Debug, Clone)]
pub struct ParsedEntry {
    pub kind: MediaKind,
    /// Raw `tvg-name` value from EXTINF.
    pub name: String,
    pub group_title: String,
    pub tvg_id: Option<String>,
    /// `None` when logo URL lacks an image extension (common TMDB base-URL edge case).
    pub logo_url: Option<String>,
    pub stream_url: String,
    // Derived by the classifier ------------------------------------------------
    /// Two-to-three letter country prefix extracted from name, e.g. `"DE"`.
    pub country_prefix: Option<String>,
    /// Name with country prefix and year/season-episode suffix stripped.
    pub clean_title: String,
    /// Release year parsed from movie name, e.g. `2024`.
    pub year: Option<u16>,
    /// Series title without season/episode suffix, e.g. `"Mon Comeback"`.
    pub series_title: Option<String>,
    pub season: Option<u8>,
    pub episode: Option<u8>,
    /// Human-readable category extracted from `group_title`, e.g. `"GERMANY MOVIES"`.
    pub category: Option<String>,
}

// ── Storage-mapped types (returned by repositories) ──────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub url: String,
    pub last_refreshed_at: Option<i64>,
    pub auto_refresh_hours: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: i64,
    pub source_id: i64,
    pub tvg_id: Option<String>,
    pub name: String,
    pub country: Option<String>,
    pub category: Option<String>,
    pub raw_group: Option<String>,
    pub logo_url: Option<String>,
    pub stream_url: String,
    pub is_favorite: bool,
    pub is_in_watchlist: bool,
    pub last_played_at: Option<i64>,
    pub added_at: Option<i64>,
    /// Enrichment de-dup cluster key (variants share one); `None` until enriched.
    pub dedup_group: Option<String>,
    /// Brand-family key (`nova_core::clean::channel_brand`) for grouping a brand's
    /// channels into one bundle (e.g. all `beIN Sports N`); set at import.
    pub channel_group: Option<String>,
    /// Coarse region bucket (`nova_core::region::region_of` of `country`).
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VodItem {
    pub id: i64,
    pub source_id: i64,
    pub name: String,
    pub clean_title: Option<String>,
    pub year: Option<i64>,
    pub logo_url: Option<String>,
    pub stream_url: String,
    pub category: Option<String>,
    pub is_favorite: bool,
    pub is_in_watchlist: bool,
    pub added_at: Option<i64>,
    /// Enrichment de-dup cluster key (variants share one); `None` until enriched.
    pub dedup_group: Option<String>,
    /// TMDB display layer (Phase C, opt-in); `None` until matched.
    pub canonical_title: Option<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub overview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesEpisode {
    pub id: i64,
    pub source_id: i64,
    pub series_title: String,
    pub category: Option<String>,
    pub season: i64,
    pub episode: i64,
    pub episode_title: Option<String>,
    pub logo_url: Option<String>,
    pub stream_url: String,
    pub added_at: Option<i64>,
}

/// One live brand-family bundle (e.g. all `beIN Sports N`) returned by
/// `list_channel_groups`. `representative_id` is the channel to play / show art
/// for; `member_count` drives the "N channels" badge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelGroup {
    pub group: String,
    pub member_count: i64,
    pub representative_id: i64,
    pub title: String,
    pub logo_url: Option<String>,
    pub category: Option<String>,
    pub region: Option<String>,
    pub is_favorite: bool,
    pub is_in_watchlist: bool,
}

/// Aggregated series row returned by `list_series`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesSummary {
    pub series_title: String,
    pub source_id: i64,
    pub category: Option<String>,
    pub episode_count: i64,
    pub season_count: i64,
    pub poster_url: Option<String>,
    pub is_favorite: bool,
    pub is_in_watchlist: bool,
    pub added_at: Option<i64>,
    /// TMDB display layer (Phase C, opt-in); `None` until matched.
    pub canonical_title: Option<String>,
    pub backdrop_url: Option<String>,
    pub overview: Option<String>,
}

/// Sort order for browse list commands.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentSort {
    /// Natural insertion order (by id ASC).
    #[default]
    Default,
    /// Most recently imported first.
    RecentlyAdded,
}

/// A row on the Continue Watching shelf — joins a `watch_history` entry with
/// its underlying media. Empty until Phase 5 wires up playback progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueWatchingItem {
    pub media_kind: String, // 'vod' | 'episode'
    pub media_id: i64,
    pub source_id: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub poster_url: Option<String>,
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    pub watched_at: i64,
}

// ── Import progress event payload ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportStage {
    Connecting,
    Receiving,
    Persisting,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    pub source_id: i64,
    pub stage: ImportStage,
    pub parsed: u64,
    pub inserted: u64,
    /// Subset of `parsed` that are live channels.
    pub live_count: u64,
    /// Subset of `parsed` that are movies.
    pub movie_count: u64,
    /// Subset of `parsed` that are series episodes.
    pub series_count: u64,
    pub failed_count: u64,
    pub skipped_count: u64,
    pub elapsed_ms: u64,
    pub done: bool,
    pub error: Option<String>,
    pub warning: Option<String>,
}

// ── Transcription (Phase 6) ───────────────────────────────────────────────────

/// One word with its timing, when Whisper emits word/token timestamps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptWord {
    pub start_sec: f64,
    pub end_sec: f64,
    pub word: String,
}

/// A single caption-sized unit of a transcript. `words` is `None` when the
/// engine did not provide word-level timing. Persisted as a row in
/// `transcript_segments` (with `words` flattened to the `words_json` column).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    /// Segment ordinal within the transcript, starting at 0.
    pub seq: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
    pub words: Option<Vec<TranscriptWord>>,
}

/// The output of a transcription run, before it is persisted. Maps onto a
/// `transcripts` row plus its `transcript_segments`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTranscript {
    /// BCP-47 source language (detected or supplied).
    pub language: String,
    /// Model identifier, e.g. `large-v3-turbo-q5_0`.
    pub model: String,
    pub segments: Vec<TranscriptSegment>,
    /// Seconds of audio actually processed, if known.
    pub duration_processed_sec: Option<f64>,
}

/// Lifecycle state of a transcript for a media item, surfaced to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptState {
    /// No transcript and no job.
    None,
    Queued,
    Running,
    Done,
    Failed,
}

/// Personalizable caption appearance, persisted per profile (Phase 6.2). Pure
/// presentation — applied by the React `CaptionOverlay`. String enums are kept
/// as `String` so the frontend can map them to its own class/style tables and
/// new presets don't require a schema change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionStyle {
    /// Multiplier on the base caption font size (e.g. 0.85 / 1.0 / 1.25 / 1.5).
    pub font_scale: f64,
    /// `sans` | `serif` | `mono` | `dyslexic`.
    pub font_family: String,
    /// `white` | `yellow` | `cyan`.
    pub text_color: String,
    /// Readability backdrop: `shadow` | `outline` | `box`.
    pub background: String,
    /// Box-background opacity in 0..=1, used when `background == "box"`.
    pub bg_opacity: f64,
}

impl Default for CaptionStyle {
    fn default() -> Self {
        // Matches the player's pre-6.2 look: medium sans, white, drop-shadow.
        Self {
            font_scale: 1.0,
            font_family: "sans".to_string(),
            text_color: "white".to_string(),
            background: "shadow".to_string(),
            bg_opacity: 0.6,
        }
    }
}

/// Per-profile viewing / behaviour settings (cleanup phase). Stored as a JSON
/// blob in the profile DB's `profile_settings` table. Defaults are applied when
/// no row exists yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileSettings {
    /// Auto-advance to the next episode when one finishes.
    pub autoplay_next: bool,
    /// Caption mode applied when a media item opens: `off` | `source` |
    /// `translation` | `dual`.
    pub default_caption_mode: String,
    /// Preferred audio-track language (BCP-47 / ISO code), if any.
    pub preferred_audio_lang: Option<String>,
    /// Preferred subtitle/caption language, if any.
    pub preferred_subtitle_lang: Option<String>,
    /// Default playback route: `auto` | `remux_hls` | `transcode_hls`.
    pub default_playback_route: String,
    /// Catalog enrichment (Phase C): use TMDB to fill canonical titles, posters,
    /// backdrops, overviews and genres. Offline-first — nothing calls TMDB unless
    /// this is on *and* a key is set.
    pub tmdb_enabled: bool,
    /// User-supplied TMDB API key (v3 auth). `None` keeps enrichment fully offline.
    pub tmdb_api_key: Option<String>,
    /// UI accent theme id: `amber` | `teal` | `rose` | `violet` | `blue`.
    /// `#[serde(default)]` on the container → old blobs without it load fine.
    pub accent_color: String,
}

impl Default for ProfileSettings {
    fn default() -> Self {
        Self {
            autoplay_next: true,
            default_caption_mode: "off".to_string(),
            preferred_audio_lang: None,
            preferred_subtitle_lang: None,
            default_playback_route: "auto".to_string(),
            tmdb_enabled: false,
            tmdb_api_key: None,
            accent_color: "amber".to_string(),
        }
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_secs() as i64
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_serializes_camel_case() {
        let p = Profile {
            id: "abc".into(),
            name: "Alex".into(),
            avatar_color: "#6366f1".into(),
            created_at: 0,
            last_used_at: Some(1),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert!(json.get("avatarColor").is_some(), "must use camelCase");
        assert!(
            json.get("avatar_color").is_none(),
            "must NOT use snake_case"
        );
    }

    #[test]
    fn import_progress_serializes() {
        let p = ImportProgress {
            source_id: 1,
            stage: ImportStage::Receiving,
            parsed: 500,
            inserted: 498,
            live_count: 200,
            movie_count: 150,
            series_count: 148,
            failed_count: 1,
            skipped_count: 2,
            elapsed_ms: 1234,
            done: false,
            error: None,
            warning: Some("one endpoint failed".into()),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert!(json.get("sourceId").is_some());
        assert_eq!(
            json.get("stage").and_then(|v| v.as_str()),
            Some("receiving")
        );
        assert!(json.get("liveCount").is_some());
        assert!(json.get("movieCount").is_some());
        assert!(json.get("seriesCount").is_some());
        assert!(json.get("failedCount").is_some());
        assert!(json.get("skippedCount").is_some());
        assert!(json.get("elapsedMs").is_some());
        assert!(json.get("warning").is_some());
    }
}
