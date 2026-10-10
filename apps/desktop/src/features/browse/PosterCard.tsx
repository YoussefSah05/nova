//! Shared catalog card used by rails and the grid. Poster aspect for movies /
//! series, a wider logo tile for live channels. Favourite / watchlist quick
//! actions reveal on hover/focus and sit as siblings of the activate button
//! (never nested) so the markup stays valid and clicks don't conflict.

import { memo, useRef } from "react";
import { Play, Plus, Check, Heart, Info } from "lucide-react";
import { cn } from "@/lib/utils";
import { AnimatePresence } from "@/lib/motion";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { HoverPreview, useHoverPreview } from "./HoverPreview";
import { FavoriteButton } from "./FavoriteButton";
import { WatchlistButton } from "./WatchlistButton";
import { PosterFallback } from "./PosterFallback";
import type { BrowseItem } from "./queries";
import { Artwork } from "@/components/Artwork";
import { tmdbSized } from "@/lib/artwork";
import { ChannelTile } from "./ChannelTile";
import { usePrefetchTitle } from "./prefetch";
import { useChannelGuide } from "./guide";
import { GuideLine } from "./GuideViews";

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
  // A bundle card stands for several channels: no single guide.
  const guide = useChannelGuide(item.id, {
    enabled: live && !(item.memberCount && item.memberCount > 1),
  });
  const cardRef = useRef<HTMLDivElement>(null);
  const preview = useHoverPreview();
  const prefetch = usePrefetchTitle(item);
  // Movies: the best picture quality on the poster, year and rating below.
  const movie = item.movie;
  const quality = movie?.badges?.find((b) => b === "4K" || b === "FHD" || b === "HD") ?? null;
  const versions = movie?.versionCount ?? (item.variants ? item.variants.length + 1 : 1);
  const movieMeta = movie
    ? [movie.year, movie.rating ? `★ ${movie.rating.toFixed(1)}` : null].filter(Boolean).join(" · ")
    : null;
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          ref={cardRef}
          data-card
          onMouseEnter={(e) => {
            preview.onEnter(e);
            prefetch.onEnter(e);
          }}
          onMouseLeave={() => {
            preview.onLeave();
            prefetch.onLeave();
          }}
          className={cn(
            "group relative flex flex-shrink-0 snap-start flex-col gap-2 [contain:layout_paint]",
            live ? "w-44" : "w-36"
          )}
        >
          <AnimatePresence>
            {preview.open && cardRef.current && (
              <HoverPreview
                item={item}
                anchor={cardRef.current.getBoundingClientRect()}
                live={live}
                onActivate={(i) => {
                  preview.close();
                  onActivate(i);
                }}
                onToggleFavorite={onToggleFavorite}
                onToggleWatchlist={onToggleWatchlist}
                onClose={preview.close}
              />
            )}
          </AnimatePresence>
          <button
            data-card-visual
            type="button"
            onClick={() => {
              preview.close();
              onActivate(item);
            }}
            className={cn(
              "relative w-full overflow-hidden ring-ring transition-[transform,box-shadow,border-color] duration-200",
              "hover:-translate-y-1 hover:shadow-2xl hover:shadow-black/50 focus:outline-none focus-visible:ring-2",
              live
                ? "aspect-video rounded-2xl shadow-lg shadow-black/30 ring-0 hover:ring-1 hover:ring-white/25"
                : "aspect-[2/3] rounded-xl border border-border bg-muted ring-0 hover:border-primary/40"
            )}
            aria-label={item.title}
          >
            {live ? (
              <ChannelTile logoUrl={item.posterUrl} title={item.title} />
            ) : (
              <Artwork
                src={tmdbSized(item.posterUrl, "w342")}
                alt=""
                loading="lazy"
                decoding="async"
                className="h-full w-full object-cover"
                fallback={<PosterFallback title={item.title} />}
              />
            )}
            <div
              data-card-hover
              className="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/50 via-transparent to-transparent opacity-0 transition-opacity group-hover:opacity-100"
            />
            {/* Solid chips, never blurred: a grid of blurs is recomputed on
                every scroll frame. */}
            {quality && (
              <span className="pointer-events-none absolute left-2 top-2 rounded-[5px] bg-black/75 px-1.5 py-px text-xs font-medium text-white/90">
                {quality}
              </span>
            )}
            {item.memberCount && item.memberCount > 1 ? (
              <span className="pointer-events-none absolute bottom-2 left-2 rounded-md bg-black/75 px-1.5 py-0.5 text-xs font-medium text-white">
                {item.memberCount} channels
              </span>
            ) : versions > 1 ? (
              <span className="pointer-events-none absolute bottom-2 left-2 rounded-md bg-black/75 px-1.5 py-0.5 text-xs font-medium text-white">
                {versions} versions
              </span>
            ) : null}
          </button>

          {/* Quick actions — revealed on hover/focus within the card. */}
          <div
            data-card-hover
            className="pointer-events-none absolute right-2 top-2 flex gap-1 opacity-0 transition-opacity duration-150 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100"
          >
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
            {live && guide?.now ? (
              <GuideLine guide={guide} className="mt-0.5" />
            ) : movieMeta ? (
              <p className="truncate text-xs text-muted-foreground">{movieMeta}</p>
            ) : (
              item.subtitle && (
                <p className="truncate text-xs text-muted-foreground">{item.subtitle}</p>
              )
            )}
          </div>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={() => onActivate(item)}>
          {live ? <Play /> : <Info />}
          {live ? "Play" : "View details"}
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

export function PosterCardSkeleton({ live = false }: { live?: boolean }) {
  return (
    <div className={cn("flex flex-shrink-0 flex-col gap-2", live ? "w-44" : "w-36")}>
      <div
        className={cn(
          "nova-shimmer w-full rounded-xl border border-border bg-muted",
          live ? "aspect-video" : "aspect-[2/3]"
        )}
      />
      <div className="nova-shimmer h-3 w-3/4 rounded bg-muted" />
    </div>
  );
}
