//! Map a series episode to the player's `PlayMedia` (shared by the cinema
//! episode browser and autoplay-next).

import type { SeriesEpisode } from "@/types";
import type { PlayMedia } from "./PlayerModal";

export function episodeToMedia(ep: SeriesEpisode, fallbackPoster?: string | null): PlayMedia {
  const code = `S${String(ep.season).padStart(2, "0")}E${String(ep.episode).padStart(2, "0")}`;
  return {
    kind: "episode",
    id: ep.id,
    title: ep.seriesTitle,
    subtitle: ep.episodeTitle ? `${code} — ${ep.episodeTitle}` : code,
    posterUrl: ep.logoUrl ?? fallbackPoster ?? null,
  };
}
