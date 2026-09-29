//! Search / filter / see-all surface: a responsive poster grid backed by an
//! bounded page query and windowed by row. Column count is derived from the live
//! container width; items are chunked into rows and only on-screen rows mount,
//! so the grid stays smooth across tens of thousands of results.

import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { RotateCwIcon, SearchXIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { VirtualRows } from "./VirtualRows";
import { PosterCard, type PosterCardHandlers } from "./PosterCard";
import {
  GRID_PAGE_SIZE,
  useBrowseCount,
  useBrowseGrid,
  type BrowseFilters,
  type BrowseTab,
} from "./queries";

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
/// The callback is rAF-coalesced and only commits on a real change, so a flurry
/// of resize ticks during scroll never re-chunks the grid every frame.
function useElementWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    setWidth(el.clientWidth);
    let raf = 0;
    const observer = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (!w) return;
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => setWidth((prev) => (prev === w ? prev : w)));
    });
    observer.observe(el);
    return () => {
      cancelAnimationFrame(raf);
      observer.disconnect();
    };
  }, [ref]);
  return width;
}

/// The grid measures its own width (not the scroll container's, whose
/// padding depends on the page), so its columns always fit the cards.
export function BrowseGrid(props: BrowseGridProps) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const width = useElementWidth(wrapRef);
  return (
    <div ref={wrapRef} className="w-full">
      <BrowseGridBody {...props} width={width} />
    </div>
  );
}

/// Room each card needs: its width plus the 12px gap.
const GAP = 12;

function BrowseGridBody({
  width,
  tab,
  profileId,
  sourceId,
  filters,
  scrollRef,
  ...handlers
}: BrowseGridProps & { width: number }) {
  const identity = JSON.stringify([tab, profileId, sourceId, filters]);
  const [page, setPage] = useState({ identity, offset: 0 });
  const offset = page.identity === identity ? page.offset : 0;
  const { data, isLoading, isError, refetch } = useBrowseGrid(
    tab,
    profileId,
    sourceId,
    filters,
    offset
  );
  const { data: total } = useBrowseCount(tab, profileId, sourceId, filters);
  const items = useMemo(() => data?.items ?? [], [data]);
  function navigate(next: number) {
    setPage({ identity, offset: next });
    scrollRef.current?.scrollTo({ top: 0, behavior: "instant" });
  }
  const live = tab === "live";
  const cardMin = live ? 200 : 168;
  const columns = Math.max(1, Math.floor((width + GAP) / cardMin));
  const rows = useMemo(() => chunk(items, columns), [items, columns]);

  if (isError) {
    return (
      <Card className="flex flex-col items-center justify-center gap-3 px-6 py-16 text-center">
        <SearchXIcon className="h-8 w-8 text-muted-foreground" aria-hidden />
        <p className="text-sm font-medium text-foreground">Search failed</p>
        <Button variant="outline" size="sm" onClick={() => refetch()} className="gap-2">
          <RotateCwIcon className="h-4 w-4" aria-hidden /> Retry
        </Button>
      </Card>
    );
  }

  if (isLoading) {
    return (
      <div
        className="grid gap-3"
        style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}
      >
        {Array.from({ length: columns * 4 }, (_, i) => (
          <Skeleton key={i} className={live ? "aspect-video" : "aspect-[2/3]"} />
        ))}
      </div>
    );
  }

  if (items.length === 0 && offset === 0) {
    return (
      <Card className="flex flex-col items-center justify-center gap-3 px-6 py-16 text-center">
        <SearchXIcon className="h-8 w-8 text-muted-foreground" aria-hidden />
        <p className="text-sm font-medium text-foreground">Nothing matches this view</p>
        <p className="max-w-sm text-sm text-muted-foreground">
          Clear filters or try a broader search.
        </p>
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-3" aria-label="Library pages">
        <p className="text-xs text-muted-foreground" role="status">
          Page {Math.floor(offset / GRID_PAGE_SIZE) + 1} · {items.length.toLocaleString()} titles on
          this page
          {total !== undefined && ` · ${total.toLocaleString()} source entries`}
        </p>
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={offset === 0}
            onClick={() => navigate(Math.max(0, offset - GRID_PAGE_SIZE))}
          >
            Previous page
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={data?.nextOffset == null}
            onClick={() => {
              if (data?.nextOffset != null) navigate(data.nextOffset);
            }}
          >
            Next page
          </Button>
        </div>
      </div>
      <VirtualRows
        items={rows}
        scrollRef={scrollRef}
        estimateSize={live ? 150 : 280}
        overscan={6}
        className="rounded-none border-0 bg-transparent"
        keyFor={(_, index) => index}
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
