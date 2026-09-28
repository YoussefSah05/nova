//! Text for Continue Watching cards.

import { episodeCode } from "@/features/player/episodeMedia";
import type { ContinueWatchingItem } from "@/types";

function numberCode(item: ContinueWatchingItem): string | null {
  return item.season !== null && item.episode !== null && item.numbering
    ? episodeCode({ season: item.season, episode: item.episode, numbering: item.numbering })
    : null;
}

/// The episode a series card points at: `S02E04 — Title`, whichever parts are
/// known (placeholder numbers are never shown).
export function continueEpisodeLabel(item: ContinueWatchingItem): string {
  const code = numberCode(item);
  if (code && item.episodeTitle) return `${code} — ${item.episodeTitle}`;
  return code ?? item.episodeTitle ?? "Episode";
}

/// The card's second line: time left, the episode to continue, or "Up next".
export function continueSubtitle(item: ContinueWatchingItem): string | null {
  const minutesLeft =
    item.positionSec > 0 && item.durationSec && item.durationSec > item.positionSec
      ? Math.max(1, Math.round((item.durationSec - item.positionSec) / 60))
      : null;
  const timeLeft = minutesLeft !== null ? `${minutesLeft} min left` : null;
  if (item.mediaKind === "episode") {
    const code = numberCode(item);
    const ep = code ?? item.episodeTitle ?? "Episode";
    if (item.upNext) return `Up next · ${ep}`;
    return timeLeft ? `${ep} · ${timeLeft}` : ep;
  }
  if (item.mediaKind === "live") return null;
  return timeLeft;
}
