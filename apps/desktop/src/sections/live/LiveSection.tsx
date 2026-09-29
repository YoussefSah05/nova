//! Live, "Channels": a data-dense guide rather than a poster wall. A
//! category rail with counts on the left, the programme guide (or, toggled,
//! the logo wall) in the middle, and the chosen channel beside it. The whole
//! section fits the window; the guide scrolls on its own. Opening it asks
//! Rust to refresh a playlist's guide that has gone stale (once a session).

import { useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SearchIcon } from "lucide-react";

import { BrowseBoundary } from "@/features/browse/BrowseBoundary";
import { BrowseGrid } from "@/features/browse/BrowseGrid";
import { formatGuideTime, useMinute } from "@/features/browse/guide";
import type { BrowseFilters } from "@/features/browse/queries";
import { useLiveOverview, type GuideScope } from "@/features/live/data";
import { LiveGuide } from "@/features/live/LiveGuide";
import { cn } from "@/lib/utils";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { useSession } from "@/app/session";
import { NoPlaylist } from "@/sections/shared/NoPlaylist";

type Mode = "guide" | "wall";
/// A rail entry: every channel, favourites, or one category.
type Slice = { kind: "all" } | { kind: "favourites" } | { kind: "category"; name: string };

/// Playlists whose guide freshness was checked this session.
const checked = new Set<string>();

export function LiveSection() {
  const { profile, activeSourceId, sources, sourcesLoaded } = useSession();
  const activate = useActivate();
  const { toggleFavorite, toggleWatchlist } = useLibraryActions();
  const { data: overview } = useLiveOverview(profile.id, activeSourceId);
  const [mode, setMode] = useState<Mode>("guide");
  const [slice, setSlice] = useState<Slice>({ kind: "all" });
  const [search, setSearch] = useState("");
  const query = useDeferredValue(search.trim()) || null;
  const wallRef = useRef<HTMLDivElement>(null);
  const minute = useMinute();

  // A new playlist starts on every channel.
  useEffect(() => {
    setSlice({ kind: "all" });
    setSearch("");
  }, [activeSourceId]);

  useEffect(() => {
    if (activeSourceId === null) return;
    const key = `${profile.id}:${activeSourceId}`;
    if (checked.has(key)) return;
    checked.add(key);
    void invoke("epg_refresh_if_stale", { profileId: profile.id, sourceId: activeSourceId }).catch(
      () => undefined
    );
  }, [profile.id, activeSourceId]);

  const scope = useMemo<GuideScope>(
    () => ({
      kind: "playlist",
      category: slice.kind === "category" ? slice.name : null,
      favoritesOnly: slice.kind === "favourites",
      query,
    }),
    [slice, query]
  );
  const wallFilters = useMemo<BrowseFilters>(
    () => ({
      category: scope.kind === "playlist" ? scope.category : null,
      favoritesOnly: scope.kind === "playlist" && scope.favoritesOnly,
      query,
    }),
    [scope, query]
  );

  if (sourcesLoaded && sources.length === 0) return <NoPlaylist />;

  const summary = overview
    ? `${overview.channels.toLocaleString()} ${overview.channels === 1 ? "channel" : "channels"} · guide data for ${overview.withGuide.toLocaleString()}`
    : null;
  const sliceLabel =
    slice.kind === "all"
      ? "every channel"
      : slice.kind === "favourites"
        ? "favourites"
        : slice.name;

  return (
    <div className="flex h-screen flex-col gap-4 px-6 pb-6 pt-[84px]">
      <header className="flex h-14 shrink-0 items-center gap-4">
        <h1 className="ml-2 font-display text-[28px] font-medium tracking-[-0.01em]">Channels</h1>
        {summary && <span className="text-sm text-white/60">{summary}</span>}
        <span className="flex-1" />
        <label className="flex h-9 w-60 items-center gap-2 rounded-[10px] border border-white/10 bg-white/[0.05] px-2.5 text-white/60 focus-within:border-white/25">
          <SearchIcon className="h-4 w-4 shrink-0" strokeWidth={1.5} aria-hidden />
          <input
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search channels"
            aria-label="Search channels"
            className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-white/45"
          />
        </label>
        <span className="flex items-center gap-2 text-sm tabular-nums text-white/70">
          <span aria-hidden className="h-2 w-2 rounded-full bg-primary" />
          Now {formatGuideTime(minute * 60)}
        </span>
        <div
          role="group"
          aria-label="Layout"
          className="flex rounded-xl border border-white/[0.08] bg-white/[0.05] p-1"
        >
          {(["guide", "wall"] as const).map((m) => (
            <button
              key={m}
              type="button"
              aria-pressed={mode === m}
              onClick={() => setMode(m)}
              className={cn(
                "h-8 rounded-[9px] px-3.5 text-sm font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                mode === m ? "bg-white/[0.12] text-white" : "text-white/60 hover:text-white/85"
              )}
            >
              {m === "guide" ? "Guide" : "Wall"}
            </button>
          ))}
        </div>
      </header>

      <div className="flex min-h-0 flex-1 gap-4">
        <nav
          aria-label="Categories"
          className="flex w-52 shrink-0 flex-col gap-0.5 overflow-y-auto overscroll-contain pr-1 [scrollbar-width:thin]"
        >
          <RailItem
            label="All channels"
            count={overview?.channels}
            on={slice.kind === "all"}
            onPick={() => setSlice({ kind: "all" })}
          />
          <RailItem
            label="Favourites"
            on={slice.kind === "favourites"}
            onPick={() => setSlice({ kind: "favourites" })}
          />
          {(overview?.categories.length ?? 0) > 0 && (
            <span className="mx-3 my-2 h-px bg-white/[0.08]" aria-hidden />
          )}
          {overview?.categories.map((c) => (
            <RailItem
              key={c.name}
              label={c.name}
              count={c.count}
              on={slice.kind === "category" && slice.name === c.name}
              onPick={() => setSlice({ kind: "category", name: c.name })}
            />
          ))}
        </nav>

        {activeSourceId !== null && (
          <BrowseBoundary resetKey={`live-${activeSourceId}-${mode}-${JSON.stringify(scope)}`}>
            {mode === "guide" ? (
              <LiveGuide
                key={JSON.stringify(scope)}
                scope={scope}
                emptyLabel={
                  query
                    ? `No channel matches “${query}” in ${sliceLabel}.`
                    : `No channels in ${sliceLabel}.`
                }
              />
            ) : (
              <div
                ref={wallRef}
                className="min-h-0 flex-1 overflow-y-auto overscroll-contain pr-1 [scrollbar-width:thin]"
              >
                <BrowseGrid
                  tab="live"
                  profileId={profile.id}
                  sourceId={activeSourceId}
                  filters={wallFilters}
                  scrollRef={wallRef}
                  onActivate={activate}
                  onToggleFavorite={toggleFavorite}
                  onToggleWatchlist={toggleWatchlist}
                />
              </div>
            )}
          </BrowseBoundary>
        )}
      </div>
    </div>
  );
}

function RailItem({
  label,
  count,
  on,
  onPick,
}: {
  label: string;
  count?: number;
  on: boolean;
  onPick: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onPick}
      className={cn(
        "flex h-9 shrink-0 items-center justify-between gap-3 rounded-[10px] px-3 text-left text-sm transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        on ? "bg-white/[0.08] font-medium text-white" : "text-white/70 hover:bg-white/[0.04]"
      )}
    >
      <span className="truncate">{label}</span>
      {count !== undefined && (
        <span className="shrink-0 text-xs tabular-nums text-white/50">
          {count.toLocaleString()}
        </span>
      )}
    </button>
  );
}
