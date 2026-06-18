/* eslint-disable react-refresh/only-export-components -- co-locates the hover-intent hook with its preview component */
//! Netflix-style hover preview. When a poster is hovered with intent (a short
//! dwell, never during a scroll), an expanded glass card is portalled over the
//! card's position: backdrop, title, meta, overview and the same Play / watchlist
//! / favourite actions. Portalled + fixed-positioned so it overflows the rail's
//! clipped/`contain`ed bounds; clamped to the viewport. Only ever one preview
//! (the hovered card) renders, so rail virtualization perf is unaffected.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Play, Tv } from "lucide-react";
import { motion, useReducedMotion } from "@/lib/motion";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { FavoriteButton } from "./FavoriteButton";
import { WatchlistButton } from "./WatchlistButton";
import { PosterFallback } from "./PosterFallback";
import type { BrowseItem } from "./queries";

/// Module-level scroll suppression: previews never open within this window of the
/// last scroll, so fast scrolling past cards can't spawn (then immediately tear
/// down) previews. Registered once, passively.
let lastScrollAt = 0;
if (typeof window !== "undefined") {
  window.addEventListener(
    "scroll",
    () => {
      lastScrollAt = performance.now();
    },
    { capture: true, passive: true }
  );
}

export interface HoverPreviewProps {
  item: BrowseItem;
  /// Bounding rect of the source poster (viewport coords).
  anchor: DOMRect;
  live: boolean;
  onActivate: (item: BrowseItem) => void;
  onToggleFavorite: (item: BrowseItem, next: boolean) => void;
  onToggleWatchlist: (item: BrowseItem, next: boolean) => void;
  /// Pointer left both the card and the preview.
  onClose: () => void;
}

function metaLine(item: BrowseItem, live: boolean): string {
  const parts: string[] = [];
  if (live && item.memberCount && item.memberCount > 1) parts.push(`${item.memberCount} channels`);
  const year = item.vod?.year;
  if (year) parts.push(String(year));
  if (item.series) {
    const s = item.series.seasonCount;
    if (s) parts.push(`${s} season${s === 1 ? "" : "s"}`);
  }
  const category = item.vod?.category ?? item.series?.category ?? item.channel?.category ?? null;
  if (category) parts.push(category);
  return parts.join("  ·  ");
}

export function HoverPreview({
  item,
  anchor,
  live,
  onActivate,
  onToggleFavorite,
  onToggleWatchlist,
  onClose,
}: HoverPreviewProps) {
  const reduce = useReducedMotion();
  const ref = useRef<HTMLDivElement>(null);
  // Width is ~1.5× the poster, centred on it and clamped to the viewport.
  const width = anchor.width * 1.5;
  const [pos, setPos] = useState(() => clamp(anchor, width));

  useLayoutEffect(() => {
    setPos(clamp(anchor, width));
  }, [anchor, width]);

  const backdrop = item.vod?.backdropUrl ?? item.series?.backdropUrl ?? item.posterUrl;
  const overview = item.vod?.overview ?? item.series?.overview ?? null;
  const meta = metaLine(item, live);

  return createPortal(
    <motion.div
      ref={ref}
      role="dialog"
      aria-label={item.title}
      initial={reduce ? false : { opacity: 0, scale: 0.92 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={reduce ? undefined : { opacity: 0, scale: 0.96 }}
      transition={{ duration: 0.16, ease: [0.16, 1, 0.3, 1] }}
      onMouseLeave={onClose}
      style={{ position: "fixed", top: pos.top, left: pos.left, width, zIndex: 60 }}
      className="nova-glass overflow-hidden rounded-xl"
    >
      <button
        type="button"
        onClick={() => onActivate(item)}
        className="block w-full overflow-hidden"
        aria-label={item.title}
      >
        <div
          className={cn("relative w-full overflow-hidden", live ? "aspect-video" : "aspect-video")}
        >
          {backdrop ? (
            <img
              src={backdrop}
              alt=""
              decoding="async"
              className={cn("h-full w-full", live ? "object-contain p-4" : "object-cover")}
            />
          ) : (
            <PosterFallback title={item.title} live={live} />
          )}
          <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/70 via-black/10 to-transparent" />
        </div>
      </button>

      <div className="flex flex-col gap-2 p-3">
        <div className="flex items-center gap-2">
          <Button size="sm" className="gap-1.5" onClick={() => onActivate(item)}>
            {live ? <Tv /> : <Play />}
            {live ? "Watch" : "Play"}
          </Button>
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
        <p className="font-display line-clamp-1 text-sm font-semibold text-foreground">
          {item.title}
        </p>
        {meta && <p className="text-[11px] text-muted-foreground">{meta}</p>}
        {overview && (
          <p className="line-clamp-3 text-[11px] leading-relaxed text-muted-foreground">
            {overview}
          </p>
        )}
      </div>
    </motion.div>,
    document.body
  );
}

/// Centre the preview over its anchor, then nudge it inside the viewport.
function clamp(anchor: DOMRect, width: number): { top: number; left: number } {
  const margin = 12;
  let left = anchor.left - (width - anchor.width) / 2;
  left = Math.max(margin, Math.min(left, window.innerWidth - width - margin));
  // Lift slightly above the poster top; keep it on-screen vertically too.
  let top = anchor.top - 12;
  top = Math.max(margin, Math.min(top, window.innerHeight - 320));
  return { top, left };
}

/// Hover-intent gate shared by cards. Returns handlers + the open state. Opens
/// after `delay`ms of dwell (never within 180ms of a scroll); closing is left to
/// the preview's own `onMouseLeave` so moving onto it doesn't dismiss it.
export function useHoverPreview(delay = 450) {
  const [open, setOpen] = useState(false);
  const timer = useRef<number | null>(null);

  const cancel = () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
  };

  useEffect(() => cancel, []);

  return {
    open,
    onEnter: () => {
      cancel();
      timer.current = window.setTimeout(() => {
        if (performance.now() - lastScrollAt > 180) setOpen(true);
      }, delay);
    },
    onLeave: () => {
      cancel();
    },
    close: () => {
      cancel();
      setOpen(false);
    },
  };
}
