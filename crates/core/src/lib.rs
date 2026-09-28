//! Core domain types, traits, and errors shared across all Nova crates.

use serde::{Deserialize, Serialize};

pub mod artwork;
pub mod catalog;
pub mod classify;
pub mod clean;
pub mod models;
pub mod naming;
pub mod provider_facts;
pub mod region;
pub use classify::normalize_category;
pub use clean::polish_title;
pub use provider_facts::ProviderFacts;
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
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// Human-readable category extracted from `group_title`, e.g. `"GERMANY MOVIES"`.
    pub category: Option<String>,
    /// Provider-scoped stable item ID (e.g. `xtream:vod:123`) when the
    /// provider has one. Never a URL; consumed only by the catalog compiler.
    pub upstream_id: Option<String>,
    /// Provider-claimed TMDB ID of the entry's kind (a movie id for movies, a
    /// TV id for series; Xtream `tmdb`). Identity evidence, never displayed;
    /// `None` when absent or invalid.
    pub provider_tmdb_id: Option<i64>,
    /// Tags the parser removed from the title (see `clean::ParsedTitle`).
    pub quality_hints: Vec<String>,
    pub language_hints: Vec<String>,
    /// `clean::PARSER_VERSION` that produced the parsed fields; `None` when
    /// the fields were not parsed from a provider title.
    pub parser_version: Option<u32>,
    /// Descriptive facts the provider shipped with the entry (plot, cast,
    /// rating, trailer…); `None` when it sent nothing usable.
    pub provider_facts: Option<ProviderFacts>,
}

/// Provider evidence captured at import and persisted next to a catalog row:
/// the untouched title and group label, the parser that produced the display
/// values, and the tags it removed. Insert-side only: never serialized over IPC
/// and not loaded on read paths.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceEvidence {
    pub raw_name: Option<String>,
    pub raw_group: Option<String>,
    pub parser_version: Option<u32>,
    pub quality_hints: Vec<String>,
    pub language_hints: Vec<String>,
    /// Provider collection: the group label normalized for browsing.
    pub collection: Option<String>,
    /// Nova genre evidenced by the provider label (specific genres only).
    pub provider_genre: Option<String>,
    /// Provider-shipped descriptive facts (see `provider_facts`).
    pub provider_facts: Option<ProviderFacts>,
    /// Provider-claimed TMDB id of the entry's kind (series rows use it; VOD
    /// rows also keep it on `VodItem::provider_tmdb_id`).
    pub provider_tmdb_id: Option<i64>,
}

impl SourceEvidence {
    /// Evidence for a row parsed from a provider entry.
    pub fn from_entry(e: &ParsedEntry) -> Self {
        let non_empty = |s: &str| (!s.trim().is_empty()).then(|| s.to_string());
        Self {
            raw_name: non_empty(&e.name),
            raw_group: non_empty(&e.group_title),
            parser_version: e.parser_version,
            quality_hints: e.quality_hints.clone(),
            language_hints: e.language_hints.clone(),
            collection: e.category.as_deref().and_then(non_empty),
            provider_genre: classify::provider_genre(&e.group_title).map(str::to_string),
            provider_facts: e.provider_facts.clone(),
            provider_tmdb_id: e.provider_tmdb_id,
        }
    }

    /// Stored form of the provider facts (`None` when there are none).
    pub fn facts_json(&self) -> Option<String> {
        self.provider_facts.as_ref().map(ProviderFacts::to_json)
    }

    /// The browse facet stored in `category`: genre when known, else the
    /// provider collection.
    pub fn facet(&self) -> Option<String> {
        self.provider_genre
            .clone()
            .or_else(|| self.collection.clone())
    }

    /// `genre_source` for the stored provider genre.
    pub fn genre_source(&self) -> Option<&'static str> {
        self.provider_genre.as_ref().map(|_| "provider")
    }

    /// Stored form of a hint list: comma-separated, `None` when empty.
    pub fn csv(hints: &[String]) -> Option<String> {
        (!hints.is_empty()).then(|| hints.join(","))
    }
}

/// How trustworthy an episode's stored `season`/`episode` numbers are.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EpisodeNumberingKind {
    /// Both numbers are real (provider data or parsed from the title).
    #[default]
    SeasonEpisode,
    /// Only the episode number is real (`Folge 1234`); season is a placeholder.
    Absolute,
    /// Neither is known; both numbers are placeholders and must not be shown.
    Unknown,
}

impl EpisodeNumberingKind {
    /// From parsed numbering, for rows that store placeholders when unknown.
    pub fn from_parsed(season: Option<u32>, episode: Option<u32>) -> Self {
        match (season, episode) {
            (Some(_), Some(_)) => Self::SeasonEpisode,
            (None, Some(_)) => Self::Absolute,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SeasonEpisode => "season_episode",
            Self::Absolute => "absolute",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "absolute" => Self::Absolute,
            "unknown" => Self::Unknown,
            _ => Self::SeasonEpisode,
        }
    }
}

/// A secret-free label for a source URL: `scheme://host[:port]` for remote
/// URLs (user info, path and query dropped, since Xtream puts credentials in
/// both), or the file name for a local playlist path.
pub fn display_source_url(url: &str) -> String {
    let url = url.trim();
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            if host.is_empty() {
                // `file:///…/list.m3u` — local, so the file name is safe to show.
                last_path_segment(rest)
            } else {
                format!("{}://{host}", scheme.to_ascii_lowercase())
            }
        }
        // A local path, or a malformed URL (`http:host/get.php?password=…`,
        // `user:pass@host`) that the importer will reject but that is stored.
        None => last_path_segment(url),
    }
}

/// The final path segment with any query, fragment or user info removed, so a
/// malformed provider URL can't surface its credentials.
fn last_path_segment(path: &str) -> String {
    let path = path.split(['?', '#']).next().unwrap_or("");
    let segment = path.rsplit(['/', '\\']).next().unwrap_or("");
    segment.rsplit('@').next().unwrap_or("").to_string()
}

fn serialize_display_url<S: serde::Serializer>(url: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&display_source_url(url))
}

// ── Storage-mapped types (returned by repositories) ──────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: i64,
    pub kind: String,
    pub name: String,
    /// The provider URL, which for Xtream carries the account's username and
    /// password. Stays Rust-side: IPC receives only `displayUrl`
    /// (scheme + host, see [`display_source_url`]).
    #[serde(
        rename(serialize = "displayUrl"),
        serialize_with = "serialize_display_url"
    )]
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
    /// Authenticated provider locator. Rust-side only: never serialized over IPC.
    #[serde(skip_serializing)]
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
    /// Import-time provider evidence (insert only; not sent over IPC).
    #[serde(skip)]
    pub evidence: SourceEvidence,
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
    /// Authenticated provider locator. Rust-side only: never serialized over IPC.
    #[serde(skip_serializing)]
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
    /// Provider-scoped stable ID from ingestion. Compiler input only: never
    /// serialized across IPC and not persisted on read paths.
    #[serde(skip)]
    pub upstream_id: Option<String>,
    /// Provider-claimed TMDB movie ID, entity-resolution evidence only.
    #[serde(skip)]
    pub provider_tmdb_id: Option<i64>,
    /// Import-time provider evidence (insert only; not sent over IPC).
    #[serde(skip)]
    pub evidence: SourceEvidence,
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
    /// Authenticated provider locator. Rust-side only: never serialized over IPC.
    #[serde(skip_serializing)]
    pub stream_url: String,
    pub added_at: Option<i64>,
    /// Whether `season`/`episode` are real or placeholders.
    #[serde(default)]
    pub numbering: EpisodeNumberingKind,
    /// Import-time provider evidence (insert only; not sent over IPC).
    #[serde(skip)]
    pub evidence: SourceEvidence,
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
    /// The name to show, by `naming::display_title` (confident TMDB title,
    /// else the parsed provider title).
    pub display_title: String,
    /// Opaque canonical series entity (`ser-…`); `None` only for a series the
    /// catalog compiler hasn't bound yet.
    #[serde(default)]
    pub entity_id: Option<String>,
}

impl SeriesSummary {
    /// Apply the display-name policy to this row's names.
    pub fn with_display_title(mut self) -> Self {
        self.display_title = naming::display_title(naming::DisplayNameSources {
            trusted_metadata: self.canonical_title.as_deref(),
            parsed: Some(&self.series_title),
            raw: &self.series_title,
            ..Default::default()
        });
        self
    }
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

/// Which browse tab a Continue Watching rail belongs to. Each tab owns its
/// rail: movies resume, series continue (or move on to the next episode), live
/// lists recently watched channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContinueSection {
    Movies,
    Series,
    Live,
}

/// A row on a Continue Watching rail. `media_kind`/`media_id` name what plays
/// on click (`vod` / `episode` / `live`); the optional fields add context for
/// the card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueWatchingItem {
    pub media_kind: String, // 'vod' | 'episode' | 'live'
    pub media_id: i64,
    pub source_id: i64,
    /// Display title: the movie, the series (for an episode) or the channel.
    pub title: String,
    pub subtitle: Option<String>,
    /// Card art (16:9 preferred: a backdrop or episode still; a logo for live).
    pub poster_url: Option<String>,
    pub position_sec: f64,
    pub duration_sec: Option<f64>,
    pub watched_at: i64,
    /// Canonical movie entity (movies only).
    #[serde(default)]
    pub entity_id: Option<String>,
    /// Provider series title (episodes only), to open the series.
    #[serde(default)]
    pub series_title: Option<String>,
    #[serde(default)]
    pub season: Option<i64>,
    #[serde(default)]
    pub episode: Option<i64>,
    /// How far the episode numbers are known (episodes only).
    #[serde(default)]
    pub numbering: Option<EpisodeNumberingKind>,
    #[serde(default)]
    pub episode_title: Option<String>,
    /// The episode is the next one after a finished episode, not a resume.
    #[serde(default)]
    pub up_next: bool,
}

// ── Import progress event payload ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportStage {
    Queued,
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
    /// User-supplied TMDB credential: an API Read Access Token or a legacy v3
    /// key. `None` keeps enrichment fully offline. Persisted, but write-only
    /// towards the webview (see desktop `ProfileSettingsView`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tmdb_api_key: Option<String>,
    /// Language of TMDB titles and overviews (BCP-47, e.g. `en-US`, `fr-FR`).
    /// Presentation only: identity never depends on it.
    pub metadata_language: String,
    /// Artwork v1: fill channel logos the provider and EPG did not supply from
    /// the public iptv-org channel directory (downloaded whole about once a
    /// week and matched on device). Off by default: offline-first.
    pub logo_directory_enabled: bool,
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
            metadata_language: DEFAULT_METADATA_LANGUAGE.to_string(),
            logo_directory_enabled: false,
            accent_color: "amber".to_string(),
        }
    }
}

/// TMDB metadata language used when none (or an invalid one) is configured.
pub const DEFAULT_METADATA_LANGUAGE: &str = "en-US";

/// A metadata language tag Nova sends to TMDB: `ll` or `ll-CC` (ISO 639-1 +
/// ISO 3166-1). Anything else falls back to [`DEFAULT_METADATA_LANGUAGE`].
pub fn metadata_language(tag: &str) -> &str {
    let b = tag.as_bytes();
    let lang_ok = |s: &[u8]| s.len() == 2 && s.iter().all(u8::is_ascii_lowercase);
    let valid = match b.len() {
        2 => lang_ok(b),
        5 => lang_ok(&b[..2]) && b[2] == b'-' && b[3..].iter().all(u8::is_ascii_uppercase),
        _ => false,
    };
    if valid {
        tag
    } else {
        DEFAULT_METADATA_LANGUAGE
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

#[cfg(test)]
mod metadata_language_tests {
    use super::*;

    #[test]
    fn only_well_formed_language_tags_reach_tmdb() {
        for ok in ["en-US", "fr-FR", "ar", "pt-BR", "de"] {
            assert_eq!(metadata_language(ok), ok);
        }
        for bad in [
            "",
            "EN-us",
            "english",
            "fr_FR",
            "fr-FRA",
            "x",
            "en-US; DROP",
        ] {
            assert_eq!(metadata_language(bad), DEFAULT_METADATA_LANGUAGE, "{bad:?}");
        }
        assert_eq!(ProfileSettings::default().metadata_language, "en-US");
        // Older settings blobs without the field load with the default.
        let old: ProfileSettings = serde_json::from_str(r#"{"tmdbEnabled":true}"#).unwrap();
        assert_eq!(old.metadata_language, "en-US");
    }
}
