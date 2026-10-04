//! A series as a landscape card: the show's backdrop with its name set over
//! it, its status from TMDB, a meta line and the profile's progress. Shows
//! read as places to return to, so they get the wide format films don't.
//! Quick actions and the context menu match the poster card's.

import { memo } from "react";
import { Check, Heart, Info, Plus } from "lucide-react";

import { Artwork } from "@/components/Artwork";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { tmdbSized } from "@/lib/artwork";
import { cn } from "@/lib/utils";
import { showMeta, statusLabel } from "./shows";
import { FavoriteButton } from "./FavoriteButton";
import { PosterFallback } from "./PosterFallback";
import type { PosterCardHandlers } from "./PosterCard";
import { usePrefetchTitle } from "./prefetch";
import type { BrowseItem } from "./queries";
import { WatchlistButton } from "./WatchlistButton";

export const ShowCard = memo(function ShowCard({
  item,
  onActivate,
  onToggleFavorite,
  onToggleWatchlist,
}: { item: BrowseItem } & PosterCardHandlers) {
  const prefetch = usePrefetchTitle(item);
  const s = item.series;
  const status = s ? statusLabel(s) : null;
  const meta = s ? showMeta(s) : item.subtitle;
  const watched = s && s.episodeCount > 0 ? Math.min(1, (s.watchedCount ?? 0) / s.episodeCount) : 0;
  const art = tmdbSized(s?.backdropUrl ?? null, "w780") ?? item.posterUrl;
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          onMouseEnter={prefetch.onEnter}
          onMouseLeave={prefetch.onLeave}
          data-card
          className="group relative flex min-w-0 flex-col gap-2.5 [contain:layout_paint]"
        >
          <button
            type="button"
            onClick={() => onActivate(item)}
            aria-label={[item.title, status, meta].filter(Boolean).join(", ")}
            className="relative aspect-video w-full overflow-hidden rounded-xl bg-[#111113] shadow-[inset_0_0_0_1px_rgba(255,255,255,0.08)] transition-transform duration-200 ease-[cubic-bezier(0.16,1,0.3,1)] hover:-translate-y-1 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <Artwork
              src={art}
              alt=""
              loading="lazy"
              decoding="async"
              className="absolute inset-0 h-full w-full object-cover"
              fallback={<PosterFallback title={item.title} />}
            />
            <span
              aria-hidden
              className="absolute inset-0 bg-gradient-to-t from-[#0a0a0c]/[0.78] via-[#0a0a0c]/10 to-transparent"
            />
            <span className="absolute inset-x-3.5 bottom-3 line-clamp-2 text-left font-display text-[22px] font-medium leading-tight drop-shadow">
              {item.title}
            </span>
            {status && (
              <span
                className={cn(
                  "absolute right-3 top-3 rounded-md bg-[#0a0a0c]/75 px-2 py-0.5 text-xs font-medium",
                  status === "Returning" ? "text-primary" : "text-white/70"
                )}
              >
                {status}
              </span>
            )}
            {watched > 0 && (
              <span aria-hidden className="absolute inset-x-0 bottom-0 h-[3px] bg-white/10">
                <span className="block h-full bg-primary" style={{ width: `${watched * 100}%` }} />
              </span>
            )}
          </button>
          <div className="pointer-events-none absolute left-2.5 top-2.5 flex gap-1 opacity-0 transition-opacity duration-150 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100">
            <WatchlistButton
              inWatchlist={item.isInWatchlist}
              onToggle={(next) => onToggleWatchlist(item, next)}
              label={item.title}
              size="sm"
            />
            <FavoriteButton
              isFavorite={item.isFavorite}
              onToggle={(next) => onToggleFavorite(item, next)}
              label={item.title}
              size="sm"
            />
          </div>
          {meta && <p className="truncate text-xs text-white/60">{meta}</p>}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={() => onActivate(item)}>
          <Info />
          View details
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={() => onToggleWatchlist(item, !item.isInWatchlist)}>
          {item.isInWatchlist ? <Check /> : <Plus />}
          {item.isInWatchlist ? "Remove from Watchlist" : "Add to Watchlist"}
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => onToggleFavorite(item, !item.isFavorite)}>
          <Heart className={cn(item.isFavorite && "fill-current text-primary")} />
          {item.isFavorite ? "Remove favourite" : "Favourite"}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
});
