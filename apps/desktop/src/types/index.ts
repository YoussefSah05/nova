// ── Profiles ──────────────────────────────────────────────────────────────────

export interface Profile {
  id: string;
  name: string;
  avatarColor: string;
  createdAt: number;
  lastUsedAt: number | null;
}

// ── Sources ───────────────────────────────────────────────────────────────────

export interface Source {
  id: number;
  kind: string;
  name: string;
  url: string;
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
  streamUrl: string;
  isFavorite: boolean;
  isInWatchlist: boolean;
  lastPlayedAt: number | null;
  addedAt: number | null;
}

export interface VodItem {
  id: number;
  sourceId: number;
  name: string;
  cleanTitle: string | null;
  year: number | null;
  logoUrl: string | null;
  streamUrl: string;
  category: string | null;
  isFavorite: boolean;
  isInWatchlist: boolean;
  addedAt: number | null;
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
}

export interface SeriesEpisode {
  id: number;
  sourceId: number;
  seriesTitle: string;
  category: string | null;
  season: number;
  episode: number;
  episodeTitle: string | null;
  logoUrl: string | null;
  streamUrl: string;
  addedAt: number | null;
}

export type ContentSort = "default" | "recentlyAdded";

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

export interface ContinueWatchingItem {
  mediaKind: string;
  mediaId: number;
  sourceId: number;
  title: string;
  subtitle: string | null;
  posterUrl: string | null;
  positionSec: number;
  durationSec: number | null;
  watchedAt: number;
}

// ── Import progress event ─────────────────────────────────────────────────────

export type ImportStage = "connecting" | "receiving" | "persisting" | "complete" | "failed";

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

export type EpgRefreshStage = "downloading" | "parsing" | "complete" | "failed";

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

// ── IPC errors ────────────────────────────────────────────────────────────────

export type ApiError =
  | { kind: "NotFound"; message: string }
  | { kind: "StorageError"; message: string }
  | { kind: "Conflict"; message: string }
  | { kind: "Internal"; message: string };
