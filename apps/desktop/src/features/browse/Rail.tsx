//! One horizontal carousel. It mounts its query lazily: an IntersectionObserver
//! flips `inView` (with a generous rootMargin so it prefetches just before
//! entering the viewport), and only then does `useRail` fire its LIMIT-20 fetch.
//! Off-screen rails cost nothing, which is what lets a tab show many genre rails.

import { useEffect, useRef, useState } from "react";
import { ChevronRightIcon } from "lucide-react";

import { PosterCard, PosterCardSkeleton, type PosterCardHandlers } from "./PosterCard";
import { useRail, type BrowseTab, type RailSpec } from "./queries";

interface RailProps extends PosterCardHandlers {
  tab: BrowseTab;
  profileId: string;
  sourceId: number;
  spec: RailSpec;
  /// Present for genre rails — opens the full grid filtered to this category.
  onSeeAll?: (spec: RailSpec) => void;
}

export function Rail({ tab, profileId, sourceId, spec, onSeeAll, ...handlers }: RailProps) {
  const ref = useRef<HTMLElement>(null);
  const [inView, setInView] = useState(false);

  useEffect(() => {
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
  }, [inView]);

  const { data, isLoading } = useRail(tab, profileId, sourceId, spec, inView);
  const live = tab === "live";

  // Resolved-but-empty rail (e.g. an empty watchlist): collapse it.
  if (inView && !isLoading && data && data.length === 0) {
    return null;
  }

  const showSkeleton = !inView || isLoading || !data;

  return (
    <section ref={ref} className="flex flex-col gap-2">
      <div className="flex items-center justify-between pr-1">
        <h3 className="text-sm font-semibold text-white/85">{spec.title}</h3>
        {onSeeAll && (
          <button
            type="button"
            onClick={() => onSeeAll(spec)}
            className="flex items-center gap-0.5 rounded-md px-1.5 py-0.5 text-xs font-medium text-white/45 transition-colors hover:text-white focus:outline-none focus:ring-2 focus:ring-indigo-400/40"
          >
            See all
            <ChevronRightIcon className="h-3.5 w-3.5" aria-hidden />
          </button>
        )}
      </div>
      <div className="-mx-1 flex snap-x gap-3 overflow-x-auto px-1 pb-1 [scrollbar-width:thin]">
        {showSkeleton
          ? Array.from({ length: 6 }, (_, i) => <PosterCardSkeleton key={i} live={live} />)
          : data.map((item) => <PosterCard key={item.key} item={item} {...handlers} />)}
      </div>
    </section>
  );
}
