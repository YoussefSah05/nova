//! Map a series episode to the player's `PlayMedia` (shared by the cinema
//! episode browser and autoplay-next).

import type { SeriesEpisode } from "@/types";
import type { PlayMedia } from "./PlayerModal";

const pad = (n: number) => String(n).padStart(2, "0");

/// The episode's number code, only as far as it is actually known:
/// `S03E07`, `E1234` for absolute numbering, or null when unknown (placeholder
/// numbers are never shown as a real S01E01). `withSeason: false` gives `E07`
/// for lists already grouped by season.
export function episodeCode(
  ep: Pick<SeriesEpisode, "season" | "episode" | "numbering">,
  withSeason = true
): string | null {
  switch (ep.numbering) {
    case "unknown":
      return null;
    case "absolute":
      return `E${pad(ep.episode)}`;
    default:
      return withSeason ? `S${pad(ep.season)}E${pad(ep.episode)}` : `E${pad(ep.episode)}`;
  }
}

/// `S03E07 — One Minute`, falling back to whichever part is known, or "Episode".
export function episodeLabel(ep: SeriesEpisode, withSeason = true): string {
  const code = episodeCode(ep, withSeason);
  if (code && ep.episodeTitle) return `${code} — ${ep.episodeTitle}`;
  return code ?? ep.episodeTitle ?? "Episode";
}

/// `seriesTitle` should be the series' display title (the player header);
/// the episode row only carries the parsed provider series title.
export function episodeToMedia(
  ep: SeriesEpisode,
  fallbackPoster?: string | null,
  seriesTitle?: string
): PlayMedia {
  return {
    kind: "episode",
    id: ep.id,
    title: seriesTitle ?? ep.seriesTitle,
    subtitle: episodeLabel(ep),
    posterUrl: ep.logoUrl ?? fallbackPoster ?? null,
  };
}
