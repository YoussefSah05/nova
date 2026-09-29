//! The programme guide: one row per channel (only the rows on screen are
//! mounted), programmes placed on a twelve-hour timeline, the channel column
//! and the time header pinned while scrolling, and an amber line at the
//! current time. Choosing a row shows the channel in the preview; Enter or a
//! double click plays it. Arrow keys move between rows.

import { memo, useCallback, useEffect, useLayoutEffect, useRef, type KeyboardEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import type { Channel, Program } from "@/types";
import { ChannelTile } from "@/features/browse/ChannelTile";
import { formatGuideTime } from "@/features/browse/guide";
import { cn } from "@/lib/utils";
import { cellsFor, ticks, TIMELINE_WIDTH, xFor, type GuideWindow } from "./timeline";

const NAME_W = 184;
const ROW_H = 64;
const HEADER_H = 40;

export interface GuideGridProps {
  channels: Channel[];
  /// Programmes by channel id; a missing entry is still loading.
  programs: Map<number, Program[]>;
  window: GuideWindow;
  nowSec: number;
  selectedId: number | null;
  onSelect: (channel: Channel) => void;
  onPlay: (channel: Channel) => void;
  /// The rows on screen changed (programmes are read for these).
  onVisibleRows: (first: number, last: number) => void;
  hasMore: boolean;
  onLoadMore: () => void;
}

export function GuideGrid({
  channels,
  programs,
  window,
  nowSec,
  selectedId,
  onSelect,
  onPlay,
  onVisibleRows,
  hasMore,
  onLoadMore,
}: GuideGridProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: channels.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_H,
    overscan: 8,
    scrollPaddingStart: HEADER_H,
  });
  const items = virtualizer.getVirtualItems();
  const first = items[0]?.index ?? 0;
  const last = items[items.length - 1]?.index ?? -1;

  useEffect(() => onVisibleRows(first, last), [first, last, onVisibleRows]);
  useEffect(() => {
    if (hasMore && last >= channels.length - 20) onLoadMore();
  }, [hasMore, last, channels.length, onLoadMore]);

  // Open on the present: the current time a little in from the left.
  const scrolledToNow = useRef(false);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el || scrolledToNow.current) return;
    scrolledToNow.current = true;
    el.scrollLeft = Math.max(0, xFor(nowSec, window) - 96);
  }, [nowSec, window]);

  const selectedIndex = channels.findIndex((c) => c.id === selectedId);
  const move = useCallback(
    (index: number) => {
      const channel = channels[index];
      if (!channel) return;
      onSelect(channel);
      virtualizer.scrollToIndex(index, { align: "auto" });
      // Focus once the row is mounted.
      requestAnimationFrame(() =>
        scrollRef.current
          ?.querySelector<HTMLElement>(`[data-channel="${channel.id}"]`)
          ?.focus({ preventScroll: true })
      );
    },
    [channels, onSelect, virtualizer]
  );
  const onKeyDown = (e: KeyboardEvent) => {
    const at = Math.max(0, selectedIndex);
    const target =
      e.key === "ArrowDown"
        ? at + 1
        : e.key === "ArrowUp"
          ? at - 1
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? channels.length - 1
              : null;
    if (target === null || target < 0 || target >= channels.length) return;
    e.preventDefault();
    move(target);
  };

  const nowX = xFor(nowSec, window);
  const inWindow = nowSec >= window.from && nowSec < window.to;
  const height = HEADER_H + virtualizer.getTotalSize();

  return (
    <div
      ref={scrollRef}
      onKeyDown={onKeyDown}
      className="relative h-full overflow-auto overscroll-contain [scrollbar-width:thin]"
    >
      <div className="relative" style={{ width: NAME_W + TIMELINE_WIDTH, height }}>
        {/* Time header, pinned to the top. */}
        <div
          className="sticky top-0 z-20 flex border-b border-white/[0.08] bg-[#0a0a0c]"
          style={{ height: HEADER_H }}
        >
          <div
            className="sticky left-0 z-10 flex shrink-0 items-center bg-[#0a0a0c] px-3 text-xs uppercase tracking-[0.1em] text-white/50"
            style={{ width: NAME_W }}
          >
            Today
          </div>
          <div className="relative" style={{ width: TIMELINE_WIDTH }} aria-hidden>
            {ticks(window).map((t) => (
              <span
                key={t.at}
                className="absolute top-0 flex h-full items-center border-l border-white/[0.08] pl-2 text-xs tabular-nums text-white/60"
                style={{ left: t.x }}
              >
                {formatGuideTime(t.at)}
              </span>
            ))}
            {inWindow && (
              <span
                className="absolute bottom-0 h-2 w-2 -translate-x-1/2 translate-y-1/2 rounded-full bg-primary"
                style={{ left: nowX }}
              />
            )}
          </div>
        </div>

        <div role="list" aria-label="Channels">
          {items.map((row) => {
            const channel = channels[row.index];
            return (
              <GuideRow
                key={channel.id}
                channel={channel}
                programs={programs.get(channel.id)}
                window={window}
                nowSec={nowSec}
                selected={channel.id === selectedId}
                focusable={channel.id === selectedId || (selectedIndex < 0 && row.index === 0)}
                top={HEADER_H + row.start}
                onSelect={onSelect}
                onPlay={onPlay}
              />
            );
          })}
        </div>

        {inWindow && (
          <div
            aria-hidden
            className="pointer-events-none absolute z-10 w-0.5 -translate-x-1/2 bg-primary/80"
            style={{ left: NAME_W + nowX, top: HEADER_H, height: height - HEADER_H }}
          />
        )}
      </div>
    </div>
  );
}

const GuideRow = memo(function GuideRow({
  channel,
  programs,
  window,
  nowSec,
  selected,
  focusable,
  top,
  onSelect,
  onPlay,
}: {
  channel: Channel;
  programs: Program[] | undefined;
  window: GuideWindow;
  nowSec: number;
  selected: boolean;
  focusable: boolean;
  top: number;
  onSelect: (c: Channel) => void;
  onPlay: (c: Channel) => void;
}) {
  const cells = programs ? cellsFor(programs, window, nowSec) : null;
  const current = cells?.find((c) => c.state === "now")?.program;
  return (
    <div role="listitem" className="absolute left-0 flex" style={{ top, height: ROW_H }}>
      <button
        type="button"
        data-channel={channel.id}
        tabIndex={focusable ? 0 : -1}
        aria-current={selected || undefined}
        aria-label={`${channel.name}${current ? `, now ${current.title}` : ""}`}
        onClick={() => onSelect(channel)}
        onDoubleClick={() => onPlay(channel)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            onPlay(channel);
          }
        }}
        className={cn(
          "group flex h-full text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
          selected ? "bg-white/[0.05]" : "hover:bg-white/[0.03]"
        )}
        style={{ width: NAME_W + TIMELINE_WIDTH }}
      >
        <span
          className={cn(
            "sticky left-0 z-[5] flex h-full shrink-0 items-center gap-2.5 border-b border-white/[0.05] px-3",
            selected ? "bg-[#16161a] shadow-[inset_2px_0_0_var(--color-primary)]" : "bg-[#0e0e11]"
          )}
          style={{ width: NAME_W }}
        >
          <span className="h-[30px] w-11 shrink-0 overflow-hidden rounded-md">
            <ChannelTile logoUrl={channel.logoUrl} title={channel.name} />
          </span>
          <span className="truncate text-sm font-medium">{channel.name}</span>
        </span>
        <span
          className="relative h-full shrink-0 border-b border-white/[0.05]"
          style={{ width: TIMELINE_WIDTH }}
        >
          {cells === null ? (
            <span className="nova-shimmer absolute inset-y-3 left-0 right-0 rounded-md bg-white/[0.03]" />
          ) : cells.length === 0 ? (
            <span
              className="absolute inset-y-0 flex items-center text-xs text-white/40"
              style={{ left: Math.max(0, xFor(nowSec, window) + 12) }}
            >
              No guide for this channel
            </span>
          ) : (
            cells.map((c) => (
              <span
                key={`${c.program.startUtc}-${c.program.title}`}
                title={c.program.title}
                className={cn(
                  "absolute inset-y-1.5 flex flex-col justify-center gap-0.5 overflow-hidden rounded-md px-2.5",
                  c.state === "now"
                    ? "bg-white/[0.09] text-white"
                    : c.state === "past"
                      ? "text-white/40"
                      : "bg-white/[0.035] text-white/80"
                )}
                style={{ left: c.left + 1, width: Math.max(0, c.width - 2) }}
              >
                <span className="truncate text-sm">{c.program.title}</span>
                {c.width > 72 && (
                  <span className="truncate text-xs tabular-nums text-white/50">
                    {formatGuideTime(c.program.startUtc)}
                  </span>
                )}
              </span>
            ))
          )}
        </span>
      </button>
    </div>
  );
});
