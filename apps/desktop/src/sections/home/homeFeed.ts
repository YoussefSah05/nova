//! Home's data, derived purely from what the browse caches already hold:
//! the three Continue Watching lists (movies, series, live) merged by when
//! they were watched, split into the modules Home shows.

import type { ContinueWatchingItem } from "@/types";
import type { PlayMedia, PlayMediaKind } from "@/features/player/PlayerModal";
import { continueEpisodeLabel } from "@/features/browse/continueWatching";

export interface HomeFeed {
  /// The most recently watched movie or episode: the hero.
  hero: ContinueWatchingItem | null;
  /// Other movies and episodes in progress, newest first.
  alsoInProgress: ContinueWatchingItem[];
  /// Episodes to continue or start next.
  upNext: ContinueWatchingItem[];
  /// Channels watched recently, newest first.
  recentChannels: ContinueWatchingItem[];
  /// A movie watched recently, to seed "Because you watched".
  seedMovie: ContinueWatchingItem | null;
}

const byNewest = (a: ContinueWatchingItem, b: ContinueWatchingItem) => b.watchedAt - a.watchedAt;

export function homeFeed(
  movies: ContinueWatchingItem[] = [],
  series: ContinueWatchingItem[] = [],
  live: ContinueWatchingItem[] = []
): HomeFeed {
  const watching = [...movies, ...series].sort(byNewest);
  const hero = watching[0] ?? null;
  return {
    hero,
    alsoInProgress: watching.slice(1, 4),
    upNext: [...series]
      .sort(byNewest)
      .filter((e) => e !== hero)
      .slice(0, 2),
    recentChannels: [...live].sort(byNewest),
    seedMovie: [...movies].sort(byNewest).find((m) => m.entityId) ?? null,
  };
}

/// What playing a Continue Watching card opens.
export function continueMedia(item: ContinueWatchingItem): PlayMedia {
  return {
    kind: item.mediaKind as PlayMediaKind,
    id: item.mediaId,
    title: item.title,
    subtitle: item.mediaKind === "episode" ? continueEpisodeLabel(item) : item.subtitle,
    posterUrl: item.posterUrl,
  };
}

/// Watched share, 0–100 (0 when the duration is unknown).
export function progressPct(item: ContinueWatchingItem): number {
  return item.durationSec && item.durationSec > 0
    ? Math.min(100, Math.max(0, Math.round((item.positionSec / item.durationSec) * 100)))
    : 0;
}

/// `1h 12m left` / `31 min left`, or null when unknown.
export function timeLeft(item: ContinueWatchingItem): string | null {
  if (!item.durationSec || item.durationSec <= item.positionSec || item.positionSec <= 0)
    return null;
  const minutes = Math.max(1, Math.round((item.durationSec - item.positionSec) / 60));
  return minutes >= 60
    ? `${Math.floor(minutes / 60)}h ${minutes % 60}m left`
    : `${minutes} min left`;
}

/// `1:34:10` for a playback position.
export function clockTime(totalSec: number): string {
  const s = Math.max(0, Math.floor(totalSec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

/// "Good evening" for the hour.
export function greeting(date: Date): string {
  const h = date.getHours();
  if (h < 5) return "Good night";
  if (h < 12) return "Good morning";
  if (h < 18) return "Good afternoon";
  return "Good evening";
}
