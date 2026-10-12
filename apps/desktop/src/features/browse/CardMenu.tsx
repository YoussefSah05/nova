//! The context menu of a browse card, and one shared menu for a whole grid.
//!
//! A Radix context menu per card costs ~66 µs to mount; a grid row of eight
//! adds about half a millisecond every time it scrolls into view. Grids
//! render cards without their own menu (`sharedMenu`) and wrap themselves in
//! `GridContextMenu`, which finds the right-clicked card by `data-card-key`.

import { useMemo, useState, type ReactNode } from "react";
import { Check, Heart, Info, Play, Plus } from "lucide-react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { cn } from "@/lib/utils";
import type { PosterCardHandlers } from "./PosterCard";
import type { BrowseItem } from "./queries";

export function CardMenuItems({
  item,
  onActivate,
  onToggleFavorite,
  onToggleWatchlist,
}: { item: BrowseItem } & PosterCardHandlers) {
  const live = item.kind === "live";
  return (
    <>
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
    </>
  );
}

export function GridContextMenu({
  items,
  children,
  ...handlers
}: { items: BrowseItem[]; children: ReactNode } & PosterCardHandlers) {
  const [target, setTarget] = useState<BrowseItem | null>(null);
  const byKey = useMemo(() => new Map(items.map((item) => [item.key, item])), [items]);
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          onContextMenu={(event) => {
            const card =
              event.target instanceof Element ? event.target.closest("[data-card-key]") : null;
            const item = card ? byKey.get(card.getAttribute("data-card-key") ?? "") : undefined;
            // Outside a card: no menu at all (a prevented event stops Radix too).
            if (!item) return event.preventDefault();
            setTarget(item);
          }}
        >
          {children}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        {target && <CardMenuItems item={target} {...handlers} />}
      </ContextMenuContent>
    </ContextMenu>
  );
}
