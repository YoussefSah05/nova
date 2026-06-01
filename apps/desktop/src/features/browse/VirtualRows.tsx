//! Windowed list renderer. Only the rows in (and near) the viewport are
//! mounted, so a 5,000-row catalog scrolls as cheaply as a 20-row one.
//!
//! The scroll container is shared with the rest of the browse view (the
//! `overflow-y-auto` <section> in Home), so the list does not start at the top
//! of the scrollable area. We feed the virtualizer a `scrollMargin` equal to
//! the list's offset within that container (measured in a layout effect) so its
//! windowing math stays correct.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Key, ReactNode, RefObject } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import { cn } from "@/lib/utils";

interface VirtualRowsProps<T> {
  items: T[];
  /** The shared `overflow-y-auto` scroll container (must be `position: relative`). */
  scrollRef: RefObject<HTMLElement | null>;
  /** Approximate row height in px; real heights are measured after mount. */
  estimateSize: number;
  keyFor: (item: T, index: number) => Key;
  renderRow: (item: T, index: number) => ReactNode;
  /** Fired when the last row enters the overscan window (infinite scroll). */
  onEndReached?: () => void;
  className?: string;
  overscan?: number;
}

export function VirtualRows<T>({
  items,
  scrollRef,
  estimateSize,
  keyFor,
  renderRow,
  onEndReached,
  className,
  overscan = 8,
}: VirtualRowsProps<T>) {
  const listRef = useRef<HTMLDivElement>(null);
  const [scrollMargin, setScrollMargin] = useState(0);

  // The list's distance from the top of the (relatively-positioned) scroll
  // container. Re-measured every commit (the header/filter chips above it can
  // change height); the equality guard makes it converge in one extra render
  // rather than loop.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useLayoutEffect(() => {
    const top = listRef.current?.offsetTop ?? 0;
    setScrollMargin((prev) => (prev === top ? prev : top));
  });

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => estimateSize,
    overscan,
    scrollMargin,
  });

  const virtualItems = virtualizer.getVirtualItems();
  const last = virtualItems[virtualItems.length - 1];
  const lastIndex = last?.index ?? -1;

  // Prefetch the next page once the tail row is within the overscan window.
  useEffect(() => {
    if (lastIndex >= items.length - 1 && items.length > 0) {
      onEndReached?.();
    }
  }, [lastIndex, items.length, onEndReached]);

  return (
    <div
      ref={listRef}
      className={cn(
        "relative w-full overflow-hidden rounded-2xl border border-white/5 bg-white/[0.02]",
        className
      )}
      style={{ height: virtualizer.getTotalSize() }}
    >
      {virtualItems.map((vi) => (
        <div
          key={keyFor(items[vi.index], vi.index)}
          data-index={vi.index}
          ref={virtualizer.measureElement}
          className="absolute left-0 top-0 w-full"
          style={{ transform: `translateY(${vi.start - virtualizer.options.scrollMargin}px)` }}
        >
          {renderRow(items[vi.index], vi.index)}
        </div>
      ))}
    </div>
  );
}
