//! Cinema-mode episode browser (cleanup phase).
//!
//! Shown in the player's cinema panel for series episodes: a season selector and
//! a horizontal strip of the selected season's episodes, with the currently
//! playing one highlighted. Clicking an episode switches playback in place.

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Play } from "lucide-react";

import { cn } from "@/lib/utils";
import type { SeriesEpisode } from "@/types";
import type { PlayMedia } from "./PlayerModal";
import { episodeLabel, episodeToMedia } from "./episodeMedia";
import { Artwork } from "@/components/Artwork";

export function CinemaEpisodeBrowser({
  profileId,
  currentEpisodeId,
  fallbackPoster,
  seriesDisplayTitle,
  onSelectMedia,
}: {
  profileId: string;
  currentEpisodeId: number;
  fallbackPoster?: string | null;
  /// Display title of the playing series, reused for sibling episodes.
  seriesDisplayTitle?: string;
  onSelectMedia: (m: PlayMedia) => void;
}) {
  const [episodes, setEpisodes] = useState<SeriesEpisode[]>([]);
  const [loading, setLoading] = useState(true);
  const [season, setSeason] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    void (async () => {
      try {
        const eps = await invoke<SeriesEpisode[]>("list_player_episodes", {
          profileId,
          episodeId: currentEpisodeId,
        });
        if (!cancelled) setEpisodes(eps);
      } catch {
        if (!cancelled) setEpisodes([]);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [profileId, currentEpisodeId]);

  const { seasons, bySeason, seriesTitle } = useMemo(() => {
    const map = new Map<number, SeriesEpisode[]>();
    for (const ep of episodes) {
      const list = map.get(ep.season) ?? [];
      list.push(ep);
      map.set(ep.season, list);
    }
    for (const list of map.values()) list.sort((a, b) => a.episode - b.episode);
    return {
      seasons: Array.from(map.keys()).sort((a, b) => a - b),
      bySeason: map,
      seriesTitle: episodes[0]?.seriesTitle ?? "",
    };
  }, [episodes]);

  // Default the visible season to the one containing the current episode.
  useEffect(() => {
    if (season != null || episodes.length === 0) return;
    const current = episodes.find((e) => e.id === currentEpisodeId);
    setSeason(current?.season ?? seasons[0] ?? null);
  }, [episodes, seasons, season, currentEpisodeId]);

  const shown = season != null ? (bySeason.get(season) ?? []) : [];

  return (
    <div className="flex h-full flex-col gap-2 px-6 py-3">
      <div className="flex items-center gap-3">
        <p className="truncate text-xs font-semibold uppercase tracking-wide text-white/45">
          {seriesDisplayTitle || seriesTitle || "Episodes"}
        </p>
        {seasons.length > 1 && (
          <div className="flex gap-1.5 overflow-x-auto [scrollbar-width:none]">
            {seasons.map((s) => (
              <button
                key={s}
                type="button"
                onClick={() => setSeason(s)}
                className={cn(
                  "shrink-0 rounded-full px-2.5 py-0.5 text-[11px] font-medium transition-colors",
                  s === season
                    ? "bg-white/20 text-white"
                    : "bg-white/5 text-white/60 hover:bg-white/10"
                )}
              >
                Season {s}
              </button>
            ))}
          </div>
        )}
      </div>

      {loading ? (
        <p className="flex flex-1 items-center text-sm text-white/45">Loading episodes…</p>
      ) : episodes.length === 0 ? (
        <p className="flex flex-1 items-center text-sm text-white/45">
          No other episodes available.
        </p>
      ) : (
        <div className="-mx-1 flex flex-1 gap-3 overflow-x-auto px-1 [scrollbar-width:thin]">
          {shown.map((ep) => {
            const active = ep.id === currentEpisodeId;
            return (
              <button
                key={ep.id}
                type="button"
                onClick={() =>
                  !active && onSelectMedia(episodeToMedia(ep, fallbackPoster, seriesDisplayTitle))
                }
                aria-current={active}
                className={cn(
                  "group flex w-40 flex-shrink-0 flex-col gap-1.5 rounded-lg p-2 text-left transition-colors",
                  "focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  active ? "bg-white/15" : "hover:bg-white/10"
                )}
              >
                <div className="relative aspect-video w-full overflow-hidden rounded bg-neutral-800">
                  <Artwork
                    src={ep.logoUrl}
                    alt=""
                    loading="lazy"
                    className="h-full w-full object-cover"
                  />
                  <div
                    className={cn(
                      "absolute inset-0 flex items-center justify-center bg-black/30 opacity-0 transition-opacity",
                      active ? "opacity-100" : "group-hover:opacity-100"
                    )}
                  >
                    {active ? (
                      <span className="rounded-full bg-[var(--color-accent)] px-2 py-0.5 text-[10px] font-semibold text-white">
                        Now playing
                      </span>
                    ) : (
                      <Play className="h-6 w-6 fill-current text-white" />
                    )}
                  </div>
                </div>
                <span className="line-clamp-2 text-[11px] font-medium text-white/85">
                  {episodeLabel(ep, false)}
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
