export type { AudioLanguages } from "./generated/AudioLanguages";

// ── Profiles ──────────────────────────────────────────────────────────────────

export interface Profile {
  id: string;
  name: string;
  avatarColor: string;
  createdAt: number;
  lastUsedAt: number | null;
}

/// At-a-glance per-profile summary for the picker (from `get_profile_overview`).
export interface ProfileOverview {
  accentColor: string;
  sourceCount: number;
  itemCount: number;
  continueCount: number;
  lastTitle: string | null;
}

// ── Sources ───────────────────────────────────────────────────────────────────

export interface Source {
  id: number;
  kind: string;
  name: string;
  /** Scheme + host only; the credential-bearing URL never leaves Rust. */
  displayUrl: string;
  lastRefreshedAt: number | null;
  autoRefreshHours: number;
}

// ── Content ───────────────────────────────────────────────────────────────────

export interface Channel {
  id: number;
  sourceId: number;
  tvgId: string | null;
  name: string;
  country: string | null;
  category: string | null;
  rawGroup: string | null;
  logoUrl: string | null;
  isFavorite: boolean;
  isInWatchlist: boolean;
  lastPlayedAt: number | null;
  addedAt: number | null;
  dedupGroup: string | null;
  // Live brand-family grouping (Phase: live grouping).
  channelGroup: string | null;
  region: string | null;
}

/// One live brand-family bundle (e.g. all "beIN Sports N"), from list_channel_groups.
export interface ChannelGroup {
  group: string;
  memberCount: number;
  representativeId: number;
  title: string;
  logoUrl: string | null;
  category: string | null;
  region: string | null;
  isFavorite: boolean;
  isInWatchlist: boolean;
}

export interface VodItem {
  id: number;
  sourceId: number;
  name: string;
  cleanTitle: string | null;
  year: number | null;
  logoUrl: string | null;
  category: string | null;
  isFavorite: boolean;
  isInWatchlist: boolean;
  addedAt: number | null;
  dedupGroup: string | null;
  // TMDB display layer (Phase C); null until matched.
  canonicalTitle: string | null;
  posterUrl: string | null;
  backdropUrl: string | null;
  overview: string | null;
}

export interface SeriesSummary {
  seriesTitle: string;
  sourceId: number;
  category: string | null;
  episodeCount: number;
  seasonCount: number;
  posterUrl: string | null;
  isFavorite: boolean;
  isInWatchlist: boolean;
  addedAt: number | null;
  // TMDB display layer (Phase C); null until matched.
  canonicalTitle: string | null;
  backdropUrl: string | null;
  overview: string | null;
  /// Name to show, chosen by the backend's naming policy (confident TMDB
  /// title, else the parsed provider title).
  displayTitle: string;
  /// Opaque canonical series entity; null only before the catalog compiler
  /// has bound the series.
  entityId: string | null;
  // Card facts from stored evidence (TMDB facts fetched earlier, provider
  // facts) and this profile's progress. Absent on older payloads.
  /** TMDB genres when matched, else Nova's genre. */
  genres?: string[];
  /** Out of 10. */
  rating?: number | null;
  ratingSource?: "tmdb" | "provider" | null;
  /** TMDB's production status, verbatim ("Returning Series", "Ended", …). */
  status?: string | null;
  firstYear?: number | null;
  lastYear?: number | null;
  /** The next episode TMDB has scheduled. */
  nextEpisode?: NextEpisode | null;
  /** Numbered episodes the profile has finished. */
  watchedCount?: number;
  /** Unix seconds. */
  lastWatchedAt?: number | null;
}

export interface NextEpisode {
  season: number;
  episode: number;
  name: string | null;
  /** `YYYY-MM-DD`. */
  airDate: string;
}

/** Series status filter: still airing, or ended/cancelled. */
export type SeriesStatus = "airing" | "ended";

export interface SeriesEpisode {
  id: number;
  sourceId: number;
  seriesTitle: string;
  category: string | null;
  season: number;
  episode: number;
  episodeTitle: string | null;
  logoUrl: string | null;
  addedAt: number | null;
  /// Whether `season`/`episode` are real. `absolute`: only the episode number
  /// is (e.g. "Folge 1234"); `unknown`: both are placeholders, never shown.
  numbering: "seasonEpisode" | "absolute" | "unknown";
}

/** `title`, `year` and `rating` order movies; other lists keep their natural order. */
export type ContentSort = "default" | "recentlyAdded" | "title" | "year" | "rating";

export interface ChannelFilters {
  country?: string | null;
  category?: string | null;
  query?: string | null;
  favoritesOnly?: boolean;
  watchlistOnly?: boolean;
  sort?: ContentSort;
}

export interface ContentFilters {
  category?: string | null;
  query?: string | null;
  favoritesOnly?: boolean;
  watchlistOnly?: boolean;
  sort?: ContentSort;
}

/// `playback:ended` — the Rust playback coordinator saw the native session's
/// media play to its end. The player opens `nextEpisode` when `autoplay`.
export interface PlaybackEnded {
  token: string;
  nextEpisode: SeriesEpisode | null;
  autoplay: boolean;
}

/// `playback:recover` — the native session failed; the coordinator chose what
/// to try next: the same item again, or (`otherSource`) another source of the
/// title. Bounded; after the last attempt the error stays.
export interface PlaybackRecover {
  token: string;
  mediaKind: string;
  mediaId: number;
  attempt: number;
  otherSource: boolean;
  /// `wake`: the Mac woke while this live channel played; reconnect it.
  reason: "failure" | "wake";
}

/// One card on a tab's Continue Watching rail (`list_continue_watching`).
export interface ContinueWatchingItem {
  /// What plays on click: "vod" | "episode" | "live".
  mediaKind: string;
  mediaId: number;
  sourceId: number;
  /// Movie, series (for an episode) or channel display title.
  title: string;
  subtitle: string | null;
  posterUrl: string | null;
  positionSec: number;
  durationSec: number | null;
  watchedAt: number;
  entityId: string | null;
  seriesTitle: string | null;
  season: number | null;
  episode: number | null;
  numbering: SeriesEpisode["numbering"] | null;
  episodeTitle: string | null;
  /// The next episode after a finished one (not a resume).
  upNext: boolean;
}

// ── Canonical media (P07: SourceRecord != CanonicalEntity != StreamVariant) ──
// Secret-free: no stream/source URLs. Durable user state is addressed by
// `entityId`; Rust selects the variant to play.

export interface CanonicalMediaSummary {
  entityId: string;
  kind: "movie";
  title: string;
  /** Raw provider title of the representative variant; display only. */
  sourceTitle: string;
  year: number | null;
  category: string | null;
  posterUrl: string | null;
  backdropUrl: string | null;
  overview: string | null;
  isFavorite: boolean;
  isInWatchlist: boolean;
  addedAt: number | null;
  /** Enrichment similarity cluster; presentation grouping hint, not identity. */
  dedupGroup: string | null;
  /** TMDB genres when matched, else Nova's genre. */
  genres: string[];
  runtimeMin: number | null;
  /** Out of 10. */
  rating: number | null;
  ratingSource: "tmdb" | "provider" | null;
  /** Best picture quality then audio versions across the title's sources (`4K`, `MULTI`). */
  badges: string[];
  /** How many sources of the title the playlist has. */
  versionCount: number;
}

export interface VariantSummary {
  variantId: string;
  sourceId: number;
  sourceName: string;
  label: string;
  /** False while the provider no longer lists this variant. */
  available: boolean;
  badges: string[];
  /** Failed more recently than it last played (ranked last). */
  recentlyFailed: boolean;
  /** The source Nova plays by default for this profile. */
  picked: boolean;
}

export interface ResumeState {
  positionSec: number;
  durationSec: number | null;
  completed: boolean;
}

export interface CanonicalMediaDetail extends CanonicalMediaSummary {
  variants: VariantSummary[];
  resume: ResumeState | null;
  originalTitle: string | null;
  /** `YYYY-MM-DD` or a year, from the provider. */
  releaseDate: string | null;
  voteCount: number | null;
  keywords: string[];
}

/** A facet value with how many films carry it (`list_movie_genres`). */
export interface FacetCount {
  name: string;
  count: number;
}

/** Films of one decade (`1990`), with a few posters (`list_movie_decades`). */
export interface DecadeCount {
  decade: number;
  count: number;
  posterUrls: string[];
}

/** A TMDB collection with 2+ films in the library (`list_library_collections`). */
export interface LibraryCollection {
  id: number;
  name: string;
  count: number;
  posterUrls: string[];
}

/** Transient player locator for the Rust-selected variant. */
export interface MoviePlaybackTarget {
  entityId: string;
  variantId: string;
  mediaKind: "vod";
  mediaId: number;
}

// ── Import progress event ─────────────────────────────────────────────────────

export type ImportStage =
  | "queued"
  | "connecting"
  | "receiving"
  | "persisting"
  | "complete"
  | "failed";

export interface ImportProgress {
  sourceId: number;
  stage: ImportStage;
  parsed: number;
  inserted: number;
  liveCount: number;
  movieCount: number;
  seriesCount: number;
  failedCount: number;
  skippedCount: number;
  elapsedMs: number;
  done: boolean;
  error: string | null;
  warning: string | null;
}

// ── EPG (Phase 4) ─────────────────────────────────────────────────────────────

export interface Program {
  channelTvgId: string;
  title: string;
  subTitle: string | null;
  description: string | null;
  startUtc: number;
  stopUtc: number;
  category: string | null;
  iconUrl: string | null;
}

export interface NowAndNext {
  channelTvgId: string;
  now: Program | null;
  next: Program | null;
}

/// Now-and-next for one channel row (`epg_channels_now_and_next`), from the
/// guide of the channel's own source.
export interface ChannelNowAndNext {
  channelId: number;
  now: Program | null;
  next: Program | null;
}

/** One channel row's programmes in a guide window (`epg_guide`). */
export interface ChannelGuide {
  channelId: number;
  programs: Program[];
}

/** The Live section's overview of a playlist (`list_live_overview`). */
export interface LiveOverview {
  channels: number;
  /** Channels whose `tvg_id` some guide feed has programmes for. */
  withGuide: number;
  categories: FacetCount[];
}

/// A source's guide feed (`list_epg_feeds`) — never its URL.
export interface EpgFeedStatus {
  id: number;
  kind: "xtream" | "m3u_url_tvg" | "community" | string;
  lastRefreshUtc: number | null;
  lastError: string | null;
  programmeCount: number;
}

export type EpgRefreshStage = "downloading" | "parsing" | "complete" | "failed";

/// `epg:refresh:done`: a source's guide refresh job ended.
export interface EpgRefreshDone {
  sourceId: number;
  ok: boolean;
}

export interface EpgRefreshProgress {
  sourceId: number;
  epgSourceId: number;
  stage: EpgRefreshStage;
  inserted: number;
  elapsedMs: number;
  error: string | null;
}

// ── Caption styling (Phase 6.2) ───────────────────────────────────────────────

export type CaptionFontFamily = "sans" | "serif" | "mono" | "dyslexic";
export type CaptionTextColor = "white" | "yellow" | "cyan";
export type CaptionBackground = "shadow" | "outline" | "box";

export interface CaptionStyle {
  /// Multiplier on the base caption font size (e.g. 0.85 / 1.0 / 1.25 / 1.5).
  fontScale: number;
  fontFamily: CaptionFontFamily;
  textColor: CaptionTextColor;
  background: CaptionBackground;
  /// Box-background opacity 0..1, used when `background === "box"`.
  bgOpacity: number;
}

export const DEFAULT_CAPTION_STYLE: CaptionStyle = {
  fontScale: 1.0,
  fontFamily: "sans",
  textColor: "white",
  background: "shadow",
  bgOpacity: 0.6,
};

// ── Per-profile viewing / behaviour settings (cleanup phase) ───────────────────

export type DefaultPlaybackRoute = "auto" | "remux_hls" | "transcode_hls";

export interface ProfileSettings {
  autoplayNext: boolean;
  /// "off" | "source" (legacy "translation"/"dual" values open as "source")
  defaultCaptionMode: string;
  preferredAudioLang: string | null;
  preferredSubtitleLang: string | null;
  defaultPlaybackRoute: DefaultPlaybackRoute;
  // Catalog enrichment (Phase C) — opt-in TMDB, user-supplied key.
  tmdbEnabled: boolean;
  /// TMDB title/overview language (BCP-47, e.g. "en-US", "fr-FR"). Display
  /// only; changing it refetches localized titles, never changes identity.
  metadataLanguage: string;
  /// Fill missing channel logos from the public iptv-org directory (opt-in).
  logoDirectoryEnabled: boolean;
  /// Read-only: the credential itself never reaches the webview. Change it
  /// with `set_tmdb_credential` / `clear_tmdb_credential`.
  tmdbCredentialConfigured: boolean;
  /// Last 4 characters of the saved credential, to identify it.
  tmdbCredentialHint: string | null;
  // UI accent theme id (amber | teal | rose | violet | blue).
  accentColor: string;
}

export const DEFAULT_PROFILE_SETTINGS: ProfileSettings = {
  autoplayNext: true,
  defaultCaptionMode: "off",
  preferredAudioLang: null,
  preferredSubtitleLang: null,
  defaultPlaybackRoute: "auto",
  tmdbEnabled: false,
  metadataLanguage: "en-US",
  logoDirectoryEnabled: false,
  tmdbCredentialConfigured: false,
  tmdbCredentialHint: null,
  accentColor: "amber",
};

// ── Resource governor (P06) ───────────────────────────────────────────────────

/** Deterministic work classes, highest priority first. Playback (tier 1) is
 * represented by `activePlaybackSessions`, not a class. */
export type WorkClass =
  | "realtime"
  | "interactive"
  | "explicit"
  | "metadata"
  | "background"
  | "maintenance";

/** `model_runtime_status` response. Read-only: scheduling truth stays in Rust. */
export interface ModelRuntimeStatus {
  loadedModels: number;
  activeLeases: number;
  estimatedResidentBytes: number;
  unknownResidentModels: number;
  busyModels: number;
  transitioningModels: number;
  activePlaybackSessions: number;
  queuedWork: number;
  activeWorkClass: WorkClass | null;
  residentModels: number;
  residentModelBudget: number;
  queuedJobs: number;
  activeJobs: number;
  jobSlots: number;
  suspendedWork: number;
}

// ── IPC errors ────────────────────────────────────────────────────────────────

export type ApiError =
  | { kind: "NotFound"; message: string }
  | { kind: "StorageError"; message: string }
  | { kind: "Conflict"; message: string }
  | { kind: "InvalidInput"; message: string }
  | { kind: "Internal"; message: string };

/// TMDB title artwork (artwork v1), fetched on demand for the title on screen.
/// Mirrors `commands::title_art::TitleArtView`.
export interface TitleArt {
  logoUrl: string | null;
  /** ISO 639-1 of the logo's text; null when language-neutral. */
  logoLanguage: string | null;
  logoWidth: number | null;
  logoHeight: number | null;
  /** Text-free backdrop; null keeps the title's existing backdrop. */
  backdropUrl: string | null;
}

// ── Detail-page facts (M4.6: TMDB + provider) ─────────────────────────────────
// Mirror `commands::title_facts::{TitleFactsView, EpisodeView}`.

export interface CastEntry {
  name: string;
  character?: string | null;
  profileUrl?: string | null;
}

export interface SeasonSummary {
  number: number;
  name: string;
  episodeCount: number;
  airDate?: string | null;
  posterUrl?: string | null;
}

export interface UpcomingEpisode {
  season: number;
  episode: number;
  name?: string | null;
  airDate: string;
}

export interface TitleFactsView {
  tagline: string | null;
  runtimeMin: number | null;
  /** Out of 10. */
  rating: number | null;
  ratingSource: "tmdb" | "provider" | null;
  certification: string | null;
  /** YouTube watch URL, opened in the browser via `open_trailer`. */
  trailerUrl: string | null;
  cast: CastEntry[];
  directors: string[];
  studios: string[];
  status: string | null;
  firstYear: number | null;
  lastYear: number | null;
  seasons: SeasonSummary[];
  numberOfEpisodes: number | null;
  nextEpisode: UpcomingEpisode | null;
  collection: { id: number; name: string } | null;
  /** Provider plot (used when there is no TMDB overview). */
  plot: string | null;
  genre: string | null;
}

export interface EpisodeView {
  id: number;
  season: number;
  episode: number;
  numbering: SeriesEpisode["numbering"];
  title: string | null;
  overview: string | null;
  stillUrl: string | null;
  airDate: string | null;
  runtimeMin: number | null;
  positionSec: number;
  durationSec: number | null;
  completed: boolean;
}

/** A spoken line found across the library (`search_moments`). */
export interface MomentHit {
  segmentId: number;
  startSec: number;
  endSec: number;
  text: string;
  /** `source` (spoken) or `translation`. */
  kind: string;
  /** What plays: `vod` or `episode`. */
  mediaKind: "vod" | "episode";
  mediaId: number;
  /** The film's title, or the series' for an episode. */
  title: string;
  season: number | null;
  episode: number | null;
  episodeTitle: string | null;
  /** Whether `season`/`episode` are real. */
  numbered: boolean;
  posterUrl: string | null;
  entityId: string | null;
}
