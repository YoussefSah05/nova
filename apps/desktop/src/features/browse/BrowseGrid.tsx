//! Search / filter / see-all surface: a responsive poster grid backed by an
//! infinite query and windowed by row. Column count is derived from the live
//! container width; items are chunked into rows and only on-screen rows mount,
//! so the grid stays smooth across tens of thousands of results.

import { useEffect, useMemo, useState, type RefObject } from "react";
import { DatabaseIcon } from "lucide-react";

import { VirtualRows } from "./VirtualRows";
import { PosterCard, PosterCardSkeleton, type PosterCardHandlers } from "./PosterCard";
import { useBrowseCount, useBrowseGrid, type BrowseFilters, type BrowseItem, type BrowseTab } from "./queries";

interface BrowseGridProps extends PosterCardHandlers {
  tab: BrowseTab;
  profileId: string;
  sourceId: number;
  filters: BrowseFilters;
  scrollRef: RefObject<HTMLElement | null>;
}

function chunk<T>(items: T[], size: number): T[][] {
  if (size <= 1) return items.map((it) => [it]);
  const rows: T[][] = [];
  for (let i = 0; i < items.length; i += size) rows.push(items.slice(i, i + size));
  return rows;
}

/// Live container width (drives responsive column count) via ResizeObserver.
function useElementWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    setWidth(el.clientWidth);
    const observer = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w) setWidth(w);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

export function BrowseGrid({
  tab,
  profileId,
  sourceId,
  filters,
  scrollRef,
  ...handlers
}: BrowseGridProps) {
  const { data, isLoading, hasNextPage, isFetchingNextPage, fetchNextPage } = useBrowseGrid(
    tab,
    profileId,
    sourceId,
    filters
  );
  const { data: total } = useBrowseCount(tab, profileId, sourceId, filters);

  const items = useMemo<BrowseItem[]>(() => data?.pages.flat() ?? [], [data]);
  const live = tab === "live";
  const width = useElementWidth(scrollRef);
  const cardMin = live ? 200 : 168;
  const columns = Math.max(1, Math.floor((width - 48) / cardMin));
  const rows = useMemo(() => chunk(items, columns), [items, columns]);

  if (isLoading) {
    return (
      <div
        className="grid gap-3"
        style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}
      >
        {Array.from({ length: columns * 4 }, (_, i) => (
          <PosterCardSkeleton key={i} live={live} />
        ))}
      </div>
    );
  }

  if (items.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center rounded-2xl border border-white/5 bg-white/[0.02] px-6 py-14 text-center">
        <DatabaseIcon className="h-9 w-9 text-white/20" aria-hidden />
        <p className="mt-3 text-sm font-medium text-white">Nothing matches this view</p>
        <p className="mt-1 max-w-sm text-sm text-white/35">Clear filters or try a broader search.</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      {total !== undefined && (
        <p className="text-xs text-white/35">
          Showing {items.length.toLocaleString()} of {total.toLocaleString()} {tab}
        </p>
      )}
      <VirtualRows
        items={rows}
        scrollRef={scrollRef}
        estimateSize={live ? 150 : 280}
        overscan={3}
        className="rounded-none border-0 bg-transparent"
        keyFor={(_, index) => index}
        onEndReached={() => {
          if (hasNextPage && !isFetchingNextPage) void fetchNextPage();
        }}
        renderRow={(row) => (
          <div
            className="grid gap-3 pb-3"
            style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}
          >
            {row.map((item) => (
              <PosterCard key={item.key} item={item} {...handlers} />
            ))}
          </div>
        )}
      />
    </div>
  );
}
