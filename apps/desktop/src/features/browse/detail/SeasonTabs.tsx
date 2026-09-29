//! A series' seasons as tabs. With TMDB season posters they are small poster
//! tabs (name, episode count); without, compact pills. Arrow keys, Home
//! and End move between tabs, as in any tab list.

import { memo, useRef, type KeyboardEvent } from "react";

import { Artwork } from "@/components/Artwork";
import { tmdbSized } from "@/lib/artwork";
import { cn } from "@/lib/utils";
import type { SeasonSummary } from "@/types";

export const SeasonTabs = memo(function SeasonTabs({
  seasons,
  selected,
  details,
  nameOf,
  onSelect,
}: {
  /// Season numbers the provider has, in display order.
  seasons: number[];
  selected: number | null;
  /// TMDB's seasons (posters, episode counts), when known.
  details: SeasonSummary[] | undefined;
  nameOf: (n: number) => string;
  onSelect: (n: number) => void;
}) {
  const list = useRef<HTMLDivElement>(null);
  const detail = (n: number) => details?.find((s) => s.number === n);
  const posters = seasons.some((n) => detail(n)?.posterUrl);
  const onKeyDown = (e: KeyboardEvent) => {
    const at = selected === null ? 0 : seasons.indexOf(selected);
    const target =
      e.key === "ArrowRight"
        ? Math.min(seasons.length - 1, at + 1)
        : e.key === "ArrowLeft"
          ? Math.max(0, at - 1)
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? seasons.length - 1
              : null;
    const next = target === null ? undefined : seasons[target];
    if (next === undefined) return;
    e.preventDefault();
    onSelect(next);
    list.current?.querySelector<HTMLElement>(`[data-season="${next}"]`)?.focus();
  };
  return (
    <div
      ref={list}
      role="tablist"
      aria-label="Seasons"
      onKeyDown={onKeyDown}
      className={cn("flex gap-3 overflow-x-auto pb-1", !posters && "flex-wrap gap-2")}
    >
      {seasons.map((n) => {
        const on = n === selected;
        const d = detail(n);
        return (
          <button
            key={n}
            type="button"
            role="tab"
            data-season={n}
            aria-selected={on}
            tabIndex={on ? 0 : -1}
            onClick={() => onSelect(n)}
            className={cn(
              "shrink-0 transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              posters
                ? "flex w-[92px] flex-col gap-1.5 rounded-lg p-1 text-left"
                : "rounded-full px-3 py-1 text-sm",
              posters
                ? on
                  ? "bg-white/[0.08]"
                  : "hover:bg-white/[0.05]"
                : on
                  ? "bg-white text-neutral-950"
                  : "bg-white/10 text-white/75 hover:bg-white/20"
            )}
          >
            {posters ? (
              <>
                <span
                  className={cn(
                    "block aspect-[2/3] w-full overflow-hidden rounded-md bg-white/[0.06]",
                    on ? "shadow-[0_0_0_2px_var(--color-primary)]" : "opacity-80"
                  )}
                >
                  <Artwork
                    src={tmdbSized(d?.posterUrl ?? null, "w185")}
                    alt=""
                    loading="lazy"
                    decoding="async"
                    className="h-full w-full object-cover"
                  />
                </span>
                <span
                  className={cn(
                    "truncate text-xs font-medium",
                    on ? "text-white" : "text-white/70"
                  )}
                >
                  {nameOf(n)}
                </span>
                {d && d.episodeCount > 0 && (
                  <span className="text-xs text-white/50">
                    {d.episodeCount} {d.episodeCount === 1 ? "episode" : "episodes"}
                  </span>
                )}
              </>
            ) : (
              nameOf(n)
            )}
          </button>
        );
      })}
    </div>
  );
});
