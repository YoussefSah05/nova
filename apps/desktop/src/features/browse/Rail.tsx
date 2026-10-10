//! One horizontal carousel. Primary rails (`eager`) fetch immediately so the
//! page always has content; deeper genre rails stay lazy — an
//! IntersectionObserver flips `inView` (generous rootMargin) and only then fires
//! the query, so a tab with dozens of genres costs nothing until scrolled to.

import { useEffect, useRef, useState } from "react";
import { ChevronLeftIcon, ChevronRightIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { useReducedMotion } from "@/lib/motion";
import { PosterCard, PosterCardSkeleton, type PosterCardHandlers } from "./PosterCard";
import { useRail, type BrowseTab, type RailSpec } from "./queries";

interface RailProps extends PosterCardHandlers {
  tab: BrowseTab;
  profileId: string;
  sourceId: number;
  spec: RailSpec;
  /// Fetch immediately instead of waiting to scroll into view.
  eager?: boolean;
  /// Present for genre rails — opens the full grid filtered to this category.
  onSeeAll?: (spec: RailSpec) => void;
}

export function Rail({
  tab,
  profileId,
  sourceId,
  spec,
  eager = false,
  onSeeAll,
  ...handlers
}: RailProps) {
  const ref = useRef<HTMLElement>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const [inView, setInView] = useState(eager);
  const reduce = useReducedMotion();

  useEffect(() => {
    if (eager) return;
    const el = ref.current;
    if (!el || inView) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) setInView(true);
      },
      { rootMargin: "600px 0px" }
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [eager, inView]);

  const { data, isLoading } = useRail(tab, profileId, sourceId, spec, inView);
  const live = tab === "live";

  // Resolved-but-empty rail (e.g. an empty watchlist): collapse it.
  if (inView && !isLoading && data && data.length === 0) {
    return null;
  }

  const showSkeleton = !inView || isLoading || !data;

  const nudge = (dir: 1 | -1) => {
    const el = trackRef.current;
    if (el)
      el.scrollBy({ left: dir * el.clientWidth * 0.85, behavior: reduce ? "auto" : "smooth" });
  };

  return (
    <section ref={ref} className="rail-contain group/rail flex flex-col gap-2">
      <div className="flex items-center justify-between pr-1">
        <h3 className="font-display text-[15px] font-semibold tracking-tight text-foreground/90">
          {spec.title}
        </h3>
        <div className="flex items-center gap-1">
          {onSeeAll && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => onSeeAll(spec)}
              className="h-7 gap-0.5 px-2 text-xs text-muted-foreground hover:text-foreground"
            >
              See all
              <ChevronRightIcon className="h-3.5 w-3.5" aria-hidden />
            </Button>
          )}
          <div className="hidden gap-1 opacity-0 transition-opacity group-hover/rail:opacity-100 md:flex">
            <Button
              type="button"
              variant="ghost"
              size="iconSm"
              aria-label="Scroll left"
              onClick={() => nudge(-1)}
            >
              <ChevronLeftIcon className="h-4 w-4" aria-hidden />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="iconSm"
              aria-label="Scroll right"
              onClick={() => nudge(1)}
            >
              <ChevronRightIcon className="h-4 w-4" aria-hidden />
            </Button>
          </div>
        </div>
      </div>
      <div
        ref={trackRef}
        className={cn(
          "-mx-1 flex snap-x snap-mandatory gap-3 overflow-x-auto px-1 pb-1",
          "[scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        )}
      >
        {showSkeleton
          ? Array.from({ length: 6 }, (_, i) => <PosterCardSkeleton key={i} live={live} />)
          : data.map((item) => <PosterCard key={item.key} item={item} {...handlers} />)}
      </div>
    </section>
  );
}
