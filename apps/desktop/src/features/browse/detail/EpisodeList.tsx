//! A season's episodes on a series page: still, number and title, runtime and
//! air date, overview, progress and a watched check. Click plays.

import { Check, Play } from "lucide-react";

import { Artwork } from "@/components/Artwork";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { EpisodeView } from "@/types";
import { episodeProgress, episodeViewLabel, formatRuntime } from "./meta";

function formatAirDate(date: string): string {
  const d = new Date(`${date}T00:00:00`);
  return Number.isNaN(d.getTime())
    ? date
    : d.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

export function EpisodeList({
  episodes,
  loading,
  currentId,
  onPlay,
}: {
  episodes: EpisodeView[] | undefined;
  loading: boolean;
  /// The episode the primary action points at (highlighted).
  currentId?: number;
  onPlay: (ep: EpisodeView) => void;
}) {
  if (loading && !episodes) {
    return (
      <div className="flex flex-col gap-3">
        {Array.from({ length: 4 }, (_, i) => (
          <div key={i} className="flex gap-4">
            <Skeleton className="aspect-video w-44 shrink-0 rounded-lg" />
            <div className="flex flex-1 flex-col gap-2 py-1">
              <Skeleton className="h-4 w-1/3" />
              <Skeleton className="h-3 w-2/3" />
            </div>
          </div>
        ))}
      </div>
    );
  }
  if (!episodes || episodes.length === 0) {
    return <p className="py-6 text-sm text-white/50">No episodes in this season.</p>;
  }
  return (
    <ol className="flex flex-col gap-1">
      {episodes.map((ep) => {
        const progress = episodeProgress(ep);
        const details = [
          ep.runtimeMin ? formatRuntime(ep.runtimeMin) : null,
          ep.airDate ? formatAirDate(ep.airDate) : null,
        ].filter(Boolean);
        return (
          <li key={ep.id}>
            <button
              type="button"
              onClick={() => onPlay(ep)}
              aria-label={`Play ${episodeViewLabel(ep, true)}`}
              className={cn(
                "group flex w-full gap-4 rounded-xl p-2 text-left transition-colors hover:bg-white/[0.06] focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                currentId === ep.id && "bg-white/[0.05]"
              )}
            >
              <div className="relative aspect-video w-44 shrink-0 overflow-hidden rounded-lg bg-white/5">
                <Artwork
                  src={ep.stillUrl}
                  alt=""
                  loading="lazy"
                  decoding="async"
                  className="h-full w-full object-cover"
                />
                <span className="absolute inset-0 flex items-center justify-center bg-black/40 opacity-0 transition-opacity group-hover:opacity-100">
                  <Play className="h-6 w-6 fill-white text-white" />
                </span>
                {progress > 0 && !ep.completed && (
                  <span className="absolute inset-x-0 bottom-0 h-1 bg-black/50">
                    <span
                      className="block h-full bg-primary"
                      style={{ width: `${progress * 100}%` }}
                    />
                  </span>
                )}
              </div>
              <div className="flex min-w-0 flex-1 flex-col gap-1 py-1">
                <div className="flex items-center gap-2">
                  <span className="line-clamp-1 text-sm font-medium text-white/90">
                    {episodeViewLabel(ep)}
                  </span>
                  {ep.completed && (
                    <Check className="h-4 w-4 shrink-0 text-primary" aria-label="Watched" />
                  )}
                </div>
                {details.length > 0 && (
                  <span className="text-xs text-white/45">{details.join(" · ")}</span>
                )}
                {ep.overview && (
                  <p className="line-clamp-2 text-xs leading-5 text-white/60">{ep.overview}</p>
                )}
              </div>
            </button>
          </li>
        );
      })}
    </ol>
  );
}
