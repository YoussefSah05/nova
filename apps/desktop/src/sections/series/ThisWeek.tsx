//! "This week": a seven-day calendar of the next episodes TMDB has scheduled
//! for the playlist's shows, by the viewer's local date. Air dates are known
//! for shows whose facts Nova has fetched, so the shows the profile follows
//! (in progress, watchlist, favourites) are warmed first (`useShows.ts`).

import { memo } from "react";

import type { SeriesSummary } from "@/types";
import { Artwork } from "@/components/Artwork";
import { PosterFallback } from "@/features/browse/PosterFallback";
import { seriesToItem, type BrowseItem } from "@/features/browse/queries";
import { episodeCode, type weekOf } from "@/features/browse/shows";
import { tmdbSized } from "@/lib/artwork";
import { cn } from "@/lib/utils";
import { useActivate } from "@/app/useActivate";

export const ThisWeek = memo(function ThisWeek({ week }: { week: ReturnType<typeof weekOf> }) {
  const activate = useActivate();
  if (!week.some((d) => d.shows.length > 0)) return null;
  return (
    <section aria-labelledby="shows-week" className="nova-cv flex flex-col gap-[18px]">
      <div className="flex items-baseline gap-3.5">
        <h2 id="shows-week" className="font-display text-[28px] font-medium">
          This week
        </h2>
        <span className="text-sm text-white/60">
          Next episodes of shows in your library, by air date
        </span>
      </div>
      <ol className="grid grid-cols-7 gap-2.5">
        {week.map((day) => (
          <li
            key={day.date}
            aria-label={`${day.label} ${day.dayOfMonth}`}
            className={cn(
              "flex min-h-[214px] flex-col gap-3 rounded-[14px] bg-[#111113] p-3.5",
              day.today ? "border border-primary/40" : "border border-white/[0.06]"
            )}
          >
            <span className="flex items-baseline gap-2">
              <span
                className={cn(
                  "text-xs uppercase tracking-[0.1em]",
                  day.today ? "text-primary" : "text-white/60"
                )}
              >
                {day.label}
              </span>
              <span className="font-display text-[22px] font-medium">{day.dayOfMonth}</span>
            </span>
            {day.shows.length === 0 ? (
              <span className="text-xs text-white/40">Nothing new</span>
            ) : (
              day.shows.map((s) => <AiringShow key={s.seriesTitle} show={s} onOpen={activate} />)
            )}
          </li>
        ))}
      </ol>
    </section>
  );
});

function AiringShow({ show, onOpen }: { show: SeriesSummary; onOpen: (i: BrowseItem) => void }) {
  const next = show.nextEpisode!;
  const code = `${episodeCode(next.season, next.episode)}${next.episode === 1 ? " · premiere" : ""}`;
  return (
    <button
      type="button"
      onClick={() => onOpen(seriesToItem(show))}
      aria-label={`${show.displayTitle}, ${code}`}
      className="group flex flex-col gap-1.5 rounded-lg text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span className="relative block h-[72px] overflow-hidden rounded-lg bg-white/[0.04] shadow-[inset_0_0_0_1px_rgba(255,255,255,.08)]">
        <Artwork
          src={tmdbSized(show.backdropUrl, "w300") ?? show.posterUrl}
          alt=""
          loading="lazy"
          decoding="async"
          className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-[1.03]"
          fallback={<PosterFallback title={show.displayTitle} />}
        />
      </span>
      <span className="line-clamp-2 text-sm font-medium">{show.displayTitle}</span>
      <span className="text-xs text-white/60">{code}</span>
    </button>
  );
}
