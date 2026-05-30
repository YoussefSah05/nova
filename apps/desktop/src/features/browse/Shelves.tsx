import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { BookmarkIcon, ClockIcon, HeartIcon, SparklesIcon } from "lucide-react";
import type { Channel, ContinueWatchingItem, SeriesSummary, VodItem } from "@/types";
import { type PlayMedia, type PlayMediaKind } from "@/features/player/PlayerModal";
import { cn } from "@/lib/utils";

export type ShelfTab = "live" | "movies" | "series";

interface ShelvesProps {
  profileId: string;
  sourceId: number;
  tab: ShelfTab;
  refreshKey: number;
  onSelectSeries: (series: SeriesSummary) => void;
  onPlay: (media: PlayMedia) => void;
}

const SHELF_SIZE = 20;

export function Shelves({
  profileId,
  sourceId,
  tab,
  refreshKey,
  onSelectSeries,
  onPlay,
}: ShelvesProps) {
  const [continueWatching, setContinueWatching] = useState<ContinueWatchingItem[]>([]);
  const [recent, setRecent] = useState<ShelfItem[]>([]);
  const [favorites, setFavorites] = useState<ShelfItem[]>([]);
  const [watchlist, setWatchlist] = useState<ShelfItem[]>([]);
  const [hasContent, setHasContent] = useState(true);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const items = await invoke<ContinueWatchingItem[]>("list_continue_watching", {
          profileId,
          limit: SHELF_SIZE,
        });
        if (!cancelled) setContinueWatching(items);
      } catch {
        if (!cancelled) setContinueWatching([]);
      }
    }

    void load();
    return () => {
      cancelled = true;
    };
  }, [profileId, refreshKey]);

  useEffect(() => {
    let cancelled = false;

    async function loadShelves() {
      try {
        const [recentItems, favoriteItems, watchlistItems] = await Promise.all([
          fetchShelf(profileId, sourceId, tab, { sort: "recentlyAdded" }),
          fetchShelf(profileId, sourceId, tab, { favoritesOnly: true }),
          fetchShelf(profileId, sourceId, tab, { watchlistOnly: true }),
        ]);
        if (!cancelled) {
          setRecent(recentItems);
          setFavorites(favoriteItems);
          setWatchlist(watchlistItems);
          setHasContent(
            recentItems.length > 0 || favoriteItems.length > 0 || watchlistItems.length > 0
          );
        }
      } catch {
        if (!cancelled) {
          setRecent([]);
          setFavorites([]);
          setWatchlist([]);
          setHasContent(false);
        }
      }
    }

    void loadShelves();
    return () => {
      cancelled = true;
    };
  }, [profileId, sourceId, tab, refreshKey]);

  const showContinueWatching = continueWatching.length > 0;

  if (!showContinueWatching && !hasContent) {
    return null;
  }

  return (
    <section className="flex flex-col gap-4 px-6 pb-2 pt-1">
      {showContinueWatching && (
        <Shelf title="Continue Watching" icon={<ClockIcon className="h-4 w-4" aria-hidden />}>
          {continueWatching.map((item) => (
            <ContinueWatchingCard
              key={`${item.mediaKind}-${item.mediaId}`}
              item={item}
              onPlay={onPlay}
            />
          ))}
        </Shelf>
      )}

      {watchlist.length > 0 && (
        <Shelf
          title="Your Watchlist"
          icon={<BookmarkIcon className="h-4 w-4 fill-current text-indigo-200" aria-hidden />}
        >
          {watchlist.map((item) => (
            <ShelfCard
              key={item.key}
              item={item}
              onClick={shelfItemHandler(item, onSelectSeries, onPlay)}
            />
          ))}
        </Shelf>
      )}

      {recent.length > 0 && (
        <Shelf title="Recently Added" icon={<SparklesIcon className="h-4 w-4" aria-hidden />}>
          {recent.map((item) => (
            <ShelfCard
              key={item.key}
              item={item}
              onClick={shelfItemHandler(item, onSelectSeries, onPlay)}
            />
          ))}
        </Shelf>
      )}

      {favorites.length > 0 && (
        <Shelf
          title="Your Favourites"
          icon={<HeartIcon className="h-4 w-4 fill-current text-rose-300" aria-hidden />}
        >
          {favorites.map((item) => (
            <ShelfCard
              key={item.key}
              item={item}
              onClick={shelfItemHandler(item, onSelectSeries, onPlay)}
            />
          ))}
        </Shelf>
      )}
    </section>
  );
}

function Shelf({
  title,
  icon,
  children,
}: {
  title: string;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-white/45">
        {icon}
        {title}
      </div>
      <div className="-mx-1 flex gap-3 overflow-x-auto px-1 pb-1 [scrollbar-width:thin]">
        {children}
      </div>
    </div>
  );
}

interface ShelfItem {
  key: string;
  title: string;
  subtitle: string | null;
  posterUrl: string | null;
  /// Set for series rows — clicking opens the series detail view.
  series?: SeriesSummary;
  /// Set for live/movie rows — clicking opens the player.
  play?: PlayMedia;
}

/// Resolve a shelf card's click handler: series open the detail view, live and
/// movie rows open the player. Rows with neither stay non-interactive.
function shelfItemHandler(
  item: ShelfItem,
  onSelectSeries: (series: SeriesSummary) => void,
  onPlay: (media: PlayMedia) => void
): (() => void) | undefined {
  if (item.series) return () => onSelectSeries(item.series!);
  if (item.play) return () => onPlay(item.play!);
  return undefined;
}

function ShelfCard({ item, onClick }: { item: ShelfItem; onClick?: () => void }) {
  const interactive = Boolean(onClick);
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!interactive}
      className={cn(
        "flex w-32 flex-shrink-0 flex-col gap-1.5 text-left transition-transform focus:outline-none focus:ring-2 focus:ring-indigo-400 focus:ring-offset-2 focus:ring-offset-neutral-950 disabled:cursor-default",
        interactive && "hover:-translate-y-0.5"
      )}
    >
      <div className="aspect-[2/3] w-32 overflow-hidden rounded-lg bg-neutral-800">
        {item.posterUrl ? (
          <img src={item.posterUrl} alt="" loading="lazy" className="h-full w-full object-cover" />
        ) : null}
      </div>
      <p className="line-clamp-2 text-xs font-medium text-white">{item.title}</p>
      {item.subtitle && <p className="truncate text-[11px] text-white/40">{item.subtitle}</p>}
    </button>
  );
}

function ContinueWatchingCard({
  item,
  onPlay,
}: {
  item: ContinueWatchingItem;
  onPlay: (media: PlayMedia) => void;
}) {
  const progress =
    item.durationSec && item.durationSec > 0
      ? Math.min(100, Math.round((item.positionSec / item.durationSec) * 100))
      : 0;

  return (
    <button
      type="button"
      onClick={() =>
        onPlay({
          kind: item.mediaKind as PlayMediaKind,
          id: item.mediaId,
          title: item.title,
          subtitle: item.subtitle,
          posterUrl: item.posterUrl,
        })
      }
      className="flex w-44 flex-shrink-0 flex-col gap-1.5 text-left transition-transform hover:-translate-y-0.5 focus:outline-none focus:ring-2 focus:ring-indigo-400 focus:ring-offset-2 focus:ring-offset-neutral-950"
    >
      <div className="relative aspect-video w-44 overflow-hidden rounded-lg bg-neutral-800">
        {item.posterUrl && (
          <img src={item.posterUrl} alt="" loading="lazy" className="h-full w-full object-cover" />
        )}
        <div className="absolute inset-x-0 bottom-0 h-1 bg-black/40">
          <div className="h-full bg-rose-400" style={{ width: `${progress}%` }} />
        </div>
      </div>
      <p className="line-clamp-1 text-xs font-medium text-white">{item.title}</p>
      {item.subtitle && <p className="truncate text-[11px] text-white/40">{item.subtitle}</p>}
    </button>
  );
}

interface ShelfFilters {
  sort?: "recentlyAdded";
  favoritesOnly?: boolean;
  watchlistOnly?: boolean;
}

async function fetchShelf(
  profileId: string,
  sourceId: number,
  tab: ShelfTab,
  extra: ShelfFilters
): Promise<ShelfItem[]> {
  if (tab === "live") {
    const channels = await invoke<Channel[]>("list_channels", {
      profileId,
      sourceId,
      limit: SHELF_SIZE,
      offset: 0,
      filters: { ...extra },
    });
    return channels.map((c) => {
      const subtitle = c.country ?? c.category ?? null;
      return {
        key: `live-${c.id}`,
        title: c.name,
        subtitle,
        posterUrl: c.logoUrl,
        play: { kind: "live", id: c.id, title: c.name, subtitle, posterUrl: c.logoUrl },
      };
    });
  }

  if (tab === "movies") {
    const items = await invoke<VodItem[]>("list_vod", {
      profileId,
      sourceId,
      limit: SHELF_SIZE,
      offset: 0,
      filters: { ...extra },
    });
    return items.map((v) => {
      const title = v.cleanTitle ?? v.name;
      const subtitle = v.year?.toString() ?? v.category;
      return {
        key: `vod-${v.id}`,
        title,
        subtitle,
        posterUrl: v.logoUrl,
        play: { kind: "vod", id: v.id, title, subtitle, posterUrl: v.logoUrl },
      };
    });
  }

  const items = await invoke<SeriesSummary[]>("list_series", {
    profileId,
    sourceId,
    limit: SHELF_SIZE,
    offset: 0,
    filters: { ...extra },
  });
  return items.map((s) => ({
    key: `series-${s.seriesTitle}`,
    title: s.seriesTitle,
    subtitle: s.category,
    posterUrl: s.posterUrl,
    series: s,
  }));
}
