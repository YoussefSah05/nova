//! Shared catalog card used by rails and the grid. Poster aspect for movies /
//! series, a wider logo tile for live channels. Favourite / watchlist quick
//! actions reveal on hover/focus and sit as siblings of the activate button
//! (never nested) so the markup stays valid and clicks don't conflict.

import { memo } from "react";
import { cn } from "@/lib/utils";
import { FavoriteButton } from "./FavoriteButton";
import { WatchlistButton } from "./WatchlistButton";
import type { BrowseItem } from "./queries";

export interface PosterCardHandlers {
  onActivate: (item: BrowseItem) => void;
  onToggleFavorite: (item: BrowseItem, next: boolean) => void;
  onToggleWatchlist: (item: BrowseItem, next: boolean) => void;
}

export const PosterCard = memo(function PosterCard({
  item,
  onActivate,
  onToggleFavorite,
  onToggleWatchlist,
}: { item: BrowseItem } & PosterCardHandlers) {
  const live = item.kind === "live";
  return (
    <div
      className={cn(
        "group relative flex flex-shrink-0 snap-start flex-col gap-2",
        live ? "w-44" : "w-36"
      )}
    >
      <button
        type="button"
        onClick={() => onActivate(item)}
        className={cn(
          "relative w-full overflow-hidden rounded-xl border border-border bg-muted ring-0 ring-ring transition-[transform,box-shadow,border-color] duration-200",
          "hover:-translate-y-1 hover:border-border-strong hover:shadow-2xl hover:shadow-black/40 focus:outline-none focus-visible:ring-2",
          live ? "aspect-video" : "aspect-[2/3]"
        )}
        aria-label={item.title}
      >
        {item.posterUrl ? (
          <img
            src={item.posterUrl}
            alt=""
            loading="lazy"
            className={cn("h-full w-full", live ? "object-contain p-3" : "object-cover")}
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center px-2 text-center text-xs text-muted-foreground">
            {item.title}
          </div>
        )}
        <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/50 via-transparent to-transparent opacity-0 transition-opacity group-hover:opacity-100" />
        {item.variants && item.variants.length > 0 && (
          <span className="pointer-events-none absolute bottom-2 left-2 rounded-md bg-black/70 px-1.5 py-0.5 text-[10px] font-semibold text-white backdrop-blur-sm">
            {item.variants.length + 1} versions
          </span>
        )}
      </button>

      {/* Quick actions — revealed on hover/focus within the card. */}
      <div className="pointer-events-none absolute right-2 top-2 flex gap-1 opacity-0 transition-opacity duration-150 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100">
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

      <div className="min-w-0">
        <p className="line-clamp-1 text-xs font-medium text-foreground">{item.title}</p>
        {item.subtitle && <p className="truncate text-[11px] text-muted-foreground">{item.subtitle}</p>}
      </div>
    </div>
  );
});

export function PosterCardSkeleton({ live = false }: { live?: boolean }) {
  return (
    <div className={cn("flex flex-shrink-0 flex-col gap-2", live ? "w-44" : "w-36")}>
      <div
        className={cn(
          "w-full animate-pulse rounded-xl border border-border bg-muted",
          live ? "aspect-video" : "aspect-[2/3]"
        )}
      />
      <div className="h-3 w-3/4 animate-pulse rounded bg-muted" />
    </div>
  );
}
