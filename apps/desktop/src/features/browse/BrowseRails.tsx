//! Browse-mode surface for a tab: a vertical stack of horizontal rails
//! (Netflix-style). Continue Watching + personal rails on top, then one lazy
//! rail per genre. Rails fetch only when scrolled near, so a tab with dozens of
//! genres stays cheap.

import { ClockIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import type { ContinueWatchingItem } from "@/types";
import { type PlayMedia, type PlayMediaKind } from "@/features/player/PlayerModal";
import { Rail } from "./Rail";
import { PosterCardSkeleton, type PosterCardHandlers } from "./PosterCard";
import {
  genreRail,
  PERSONAL_RAILS,
  useCategories,
  useContinueWatching,
  type BrowseTab,
  type RailSpec,
} from "./queries";

/// Cap the number of genre rails on the landing surface; the chip-bar / "See
/// all" opens the full virtualized grid for anything beyond this.
const MAX_GENRE_RAILS = 25;

interface BrowseRailsProps extends PosterCardHandlers {
  tab: BrowseTab;
  profileId: string;
  sourceId: number;
  onPlay: (media: PlayMedia) => void;
  onSelectCategory: (category: string) => void;
}

export function BrowseRails({
  tab,
  profileId,
  sourceId,
  onPlay,
  onSelectCategory,
  ...handlers
}: BrowseRailsProps) {
  const { data: categories } = useCategories(tab, profileId, sourceId);
  const genres = (categories ?? []).slice(0, MAX_GENRE_RAILS);

  const handleSeeAll = (spec: RailSpec) => {
    if (spec.filters.category) onSelectCategory(spec.filters.category);
  };

  return (
    <div className="flex flex-col gap-6">
      <ContinueWatchingRail profileId={profileId} onPlay={onPlay} />

      {PERSONAL_RAILS.map((spec) => (
        <Rail
          key={spec.id}
          tab={tab}
          profileId={profileId}
          sourceId={sourceId}
          spec={spec}
          {...handlers}
        />
      ))}

      {genres.map((category) => (
        <Rail
          key={category}
          tab={tab}
          profileId={profileId}
          sourceId={sourceId}
          spec={genreRail(category)}
          onSeeAll={handleSeeAll}
          {...handlers}
        />
      ))}

      {!categories && (
        <div className="flex flex-col gap-2">
          <div className="h-4 w-32 animate-pulse rounded bg-white/[0.06]" />
          <div className="-mx-1 flex gap-3 overflow-hidden px-1">
            {Array.from({ length: 6 }, (_, i) => (
              <PosterCardSkeleton key={i} live={tab === "live"} />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function ContinueWatchingRail({
  profileId,
  onPlay,
}: {
  profileId: string;
  onPlay: (media: PlayMedia) => void;
}) {
  const { data } = useContinueWatching(profileId);
  if (!data || data.length === 0) return null;

  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center gap-2 text-sm font-semibold text-white/85">
        <ClockIcon className="h-4 w-4" aria-hidden />
        Continue Watching
      </div>
      <div className="-mx-1 flex snap-x gap-3 overflow-x-auto px-1 pb-1 [scrollbar-width:thin]">
        {data.map((item) => (
          <ContinueWatchingCard key={`${item.mediaKind}-${item.mediaId}`} item={item} onPlay={onPlay} />
        ))}
      </div>
    </section>
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
      className={cn(
        "flex w-48 flex-shrink-0 snap-start flex-col gap-1.5 text-left transition-transform",
        "hover:-translate-y-1 focus:outline-none focus:ring-2 focus:ring-indigo-400 focus:ring-offset-2 focus:ring-offset-neutral-950"
      )}
    >
      <div className="relative aspect-video w-48 overflow-hidden rounded-lg bg-neutral-800">
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
