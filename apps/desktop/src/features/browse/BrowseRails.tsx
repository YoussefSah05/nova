//! Browse-mode surface for a tab: a featured hero, then a vertical stack of
//! horizontal rails. The hero + personal rails + first genre rails are eager
//! (always render content — never a blank body); deeper genre rails lazy-load.
//! Explicit empty + error states ensure the area always shows *something*.

import { type RefObject } from "react";
import { ClockIcon, FilmIcon, RotateCwIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import type { ContinueWatchingItem } from "@/types";
import { type PlayMedia, type PlayMediaKind } from "@/features/player/PlayerModal";
import { Rail } from "./Rail";
import { BrowseHero } from "./BrowseHero";
import { BrowseGrid } from "./BrowseGrid";
import { PosterFallback } from "./PosterFallback";
import { type PosterCardHandlers } from "./PosterCard";
import {
  genreRail,
  PERSONAL_RAILS,
  RECENTLY_ADDED_RAIL,
  useBrowseCount,
  useCategories,
  useContinueWatching,
  useRail,
  type BrowseTab,
  type RailSpec,
} from "./queries";
import { Artwork } from "@/components/Artwork";
import { ChannelTile } from "./ChannelTile";
import { continueEpisodeLabel, continueSubtitle } from "./continueWatching";

const MAX_GENRE_RAILS = 25;
/// How many genre rails fetch immediately; the rest lazy-load on scroll.
const EAGER_GENRE_RAILS = 4;

interface BrowseRailsProps extends PosterCardHandlers {
  tab: BrowseTab;
  profileId: string;
  sourceId: number;
  /// The shared browse scroll container — passed to the embedded "All" grid so
  /// its row-windowing math stays correct.
  scrollRef: RefObject<HTMLElement | null>;
  onPlay: (media: PlayMedia) => void;
  onSelectCategory: (category: string) => void;
}

export function BrowseRails({
  tab,
  profileId,
  sourceId,
  scrollRef,
  onPlay,
  onSelectCategory,
  ...handlers
}: BrowseRailsProps) {
  const recently = useRail(tab, profileId, sourceId, RECENTLY_ADDED_RAIL, true);
  const { data: categories } = useCategories(tab, profileId, sourceId);
  const { data: count } = useBrowseCount(tab, profileId, sourceId);
  const genres = (categories ?? []).slice(0, MAX_GENRE_RAILS);
  // Series rarely carry provider genres (until a TMDB run fills them), so the
  // rails alone leave the page sparse. Append the full catalogue as a windowed
  // grid so the surface is always rich — provider-independent, reusing list_series.
  const showAllGrid = tab === "series" || genres.length === 0;
  // The primary rail is the source of truth for "is there anything here". The
  // count is only a hint — never hide present content because a (possibly
  // stale/poisoned) count says 0.
  const railHasItems = (recently.data?.length ?? 0) > 0;

  const handleSeeAll = (spec: RailSpec) => {
    if (spec.filters.category) onSelectCategory(spec.filters.category);
  };

  // Hard error on the primary rail → retry card, never a blank void.
  if (recently.isError) {
    return (
      <StateCard
        icon={<FilmIcon className="h-8 w-8 text-muted-foreground" aria-hidden />}
        title="Couldn’t load this playlist"
        body="Something went wrong fetching content."
        action={
          <Button variant="outline" size="sm" onClick={() => recently.refetch()} className="gap-2">
            <RotateCwIcon className="h-4 w-4" aria-hidden /> Retry
          </Button>
        }
      />
    );
  }

  // Source/tab genuinely has nothing: the primary rail returned no items AND
  // it's finished loading. (Don't trust count===0 alone — see `railHasItems`.)
  if (!railHasItems && !recently.isLoading && !recently.isFetching && count === 0) {
    return (
      <StateCard
        icon={<FilmIcon className="h-8 w-8 text-muted-foreground" aria-hidden />}
        title={`No ${tab} in this playlist`}
        body="Try another tab, switch playlist, or refresh from the provider."
      />
    );
  }

  return (
    <div className="flex flex-col gap-7">
      <BrowseHero tab={tab} profileId={profileId} sourceId={sourceId} {...handlers} />

      <ContinueWatchingRail profileId={profileId} sourceId={sourceId} tab={tab} onPlay={onPlay} />

      {PERSONAL_RAILS.map((spec) => (
        <Rail
          key={spec.id}
          tab={tab}
          profileId={profileId}
          sourceId={sourceId}
          spec={spec}
          eager
          {...handlers}
        />
      ))}

      {genres.map((category, i) => (
        <Rail
          key={category}
          tab={tab}
          profileId={profileId}
          sourceId={sourceId}
          spec={genreRail(category)}
          eager={i < EAGER_GENRE_RAILS}
          onSeeAll={handleSeeAll}
          {...handlers}
        />
      ))}

      {showAllGrid && (
        <section className="flex flex-col gap-3">
          <h3 className="text-sm font-semibold text-foreground/90">
            {tab === "series" ? "All Series" : tab === "movies" ? "All Movies" : "All Channels"}
          </h3>
          <BrowseGrid
            tab={tab}
            profileId={profileId}
            sourceId={sourceId}
            filters={{}}
            scrollRef={scrollRef}
            {...handlers}
          />
        </section>
      )}
    </div>
  );
}

function StateCard({
  icon,
  title,
  body,
  action,
}: {
  icon: React.ReactNode;
  title: string;
  body: string;
  action?: React.ReactNode;
}) {
  return (
    <Card className="flex flex-col items-center justify-center gap-3 px-6 py-16 text-center">
      {icon}
      <div>
        <p className="text-sm font-medium text-foreground">{title}</p>
        <p className="mt-1 max-w-sm text-sm text-muted-foreground">{body}</p>
      </div>
      {action}
    </Card>
  );
}

const CONTINUE_TITLE: Record<BrowseTab, string> = {
  movies: "Continue Watching",
  series: "Continue Watching",
  live: "Recently Watched",
};

/// Each tab has its own rail: unfinished movies, series to continue (or their
/// next episode), recently watched channels.
function ContinueWatchingRail({
  profileId,
  sourceId,
  tab,
  onPlay,
}: {
  profileId: string;
  sourceId: number;
  tab: BrowseTab;
  onPlay: (media: PlayMedia) => void;
}) {
  const { data } = useContinueWatching(profileId, sourceId, tab);
  if (!data || data.length === 0) return null;

  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center gap-2 text-sm font-semibold text-foreground/90">
        <ClockIcon className="h-4 w-4" aria-hidden />
        {CONTINUE_TITLE[tab]}
      </div>
      <div className="-mx-1 flex snap-x gap-3 overflow-x-auto px-1 pb-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {data.map((item) => (
          <ContinueWatchingCard
            key={`${item.mediaKind}-${item.mediaId}`}
            item={item}
            onPlay={onPlay}
          />
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
  const subtitle = continueSubtitle(item);

  return (
    <button
      type="button"
      onClick={() =>
        onPlay({
          kind: item.mediaKind as PlayMediaKind,
          id: item.mediaId,
          title: item.title,
          subtitle: item.mediaKind === "episode" ? continueEpisodeLabel(item) : item.subtitle,
          posterUrl: item.posterUrl,
        })
      }
      className={cn(
        "flex w-48 flex-shrink-0 snap-start flex-col gap-1.5 text-left transition-transform",
        "hover:-translate-y-1 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      )}
    >
      <div
        className={cn(
          "relative aspect-video w-48 overflow-hidden rounded-xl",
          item.mediaKind === "live" ? "shadow-lg shadow-black/30" : "border border-border bg-muted"
        )}
      >
        {item.mediaKind === "live" ? (
          <ChannelTile logoUrl={item.posterUrl} title={item.title} />
        ) : (
          <Artwork
            src={item.posterUrl}
            alt=""
            loading="lazy"
            decoding="async"
            className="h-full w-full object-cover"
            fallback={<PosterFallback title={item.title} />}
          />
        )}
        {progress > 0 && (
          <div className="absolute inset-x-0 bottom-0 h-1 bg-black/40">
            <div className="h-full bg-primary" style={{ width: `${progress}%` }} />
          </div>
        )}
      </div>
      <p className="line-clamp-1 text-xs font-medium text-foreground">{item.title}</p>
      {subtitle && <p className="truncate text-[11px] text-muted-foreground">{subtitle}</p>}
    </button>
  );
}
