//! Home's discovery modules, both built on TMDB and limited to the library:
//! this week's trending titles you already have, ranked; and more like the
//! last film you watched, with why.

import { memo } from "react";

import type { ContinueWatchingItem } from "@/types";
import { Artwork } from "@/components/Artwork";
import { tmdbSized } from "@/lib/artwork";
import { PosterCard } from "@/features/browse/PosterCard";
import { PosterFallback } from "@/features/browse/PosterFallback";
import { useRelatedTitles, useTrendingInLibrary } from "@/features/browse/detail/data";
import { movieToItem, seriesToItem, type BrowseItem } from "@/features/browse/queries";
import { WhyChip } from "@/ui/WhyChip";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { useSession } from "@/app/session";
import { HomeModule } from "./HomeModule";

const TRENDING_COUNT = 6;

export function TrendingRanked() {
  const { profile, activeSourceId } = useSession();
  const sourceId = activeSourceId ?? -1;
  const movies = useTrendingInLibrary(
    profile.id,
    sourceId,
    activeSourceId === null ? null : "movies"
  ).data;
  const series = useTrendingInLibrary(
    profile.id,
    sourceId,
    activeSourceId === null ? null : "series"
  ).data;
  // Alternate films and shows so both appear in the top six.
  const films = (movies?.movies ?? []).map(movieToItem);
  const shows = (series?.series ?? []).map(seriesToItem);
  const ranked: BrowseItem[] = [];
  for (let i = 0; ranked.length < TRENDING_COUNT && (i < films.length || i < shows.length); i++) {
    if (films[i]) ranked.push(films[i]);
    if (shows[i] && ranked.length < TRENDING_COUNT) ranked.push(shows[i]);
  }
  if (ranked.length === 0) return null;

  return (
    <HomeModule
      id="home-trending"
      title="Trending this week — and already yours"
      note="From TMDB, matched to your library"
    >
      <ol className="flex gap-6 overflow-x-auto pb-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {ranked.map((item, i) => (
          <li key={item.key} className="shrink-0">
            <RankedCard item={item} rank={i + 1} />
          </li>
        ))}
      </ol>
    </HomeModule>
  );
}

const RankedCard = memo(function RankedCard({ item, rank }: { item: BrowseItem; rank: number }) {
  const activate = useActivate();
  return (
    <button
      data-card
      type="button"
      onClick={() => activate(item)}
      aria-label={`${item.title}, number ${rank} this week`}
      className="group flex items-end rounded-xl focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span
        aria-hidden
        className="-mr-[18px] font-display text-[140px] font-medium leading-[0.78] text-transparent [-webkit-text-stroke:1.5px_rgba(255,255,255,0.28)]"
      >
        {rank}
      </span>
      <span
        data-card-visual
        className="relative h-[192px] w-[128px] overflow-hidden rounded-[10px] bg-white/[0.06] shadow-[0_8px_24px_rgba(0,0,0,.45),inset_0_0_0_1px_rgba(255,255,255,.08)] transition-transform duration-200 ease-[cubic-bezier(0.16,1,0.3,1)] group-hover:-translate-y-1"
      >
        <Artwork
          src={tmdbSized(item.posterUrl, "w342")}
          alt=""
          loading="lazy"
          decoding="async"
          className="h-full w-full object-cover"
          fallback={<PosterFallback title={item.title} />}
        />
      </span>
    </button>
  );
});

export function BecauseYouWatched({ seed }: { seed: ContinueWatchingItem | null }) {
  const { profile, activeSourceId } = useSession();
  const activate = useActivate();
  const { toggleFavorite, toggleWatchlist } = useLibraryActions();
  const subject = seed?.entityId ? ({ kind: "movie", entityId: seed.entityId } as const) : null;
  const { data } = useRelatedTitles(profile.id, activeSourceId, subject);
  const items = [
    ...(data?.movies ?? []).map(movieToItem),
    ...(data?.series ?? []).map(seriesToItem),
  ];
  if (!seed || items.length === 0) return null;

  return (
    <HomeModule
      id="home-because"
      title={
        <>
          Because you watched <span className="text-white/60">{seed.title}</span>
        </>
      }
      action={<WhyChip reason="TMDB recommends these, and they are in your library" />}
    >
      <div className="-mx-1 flex snap-x gap-4 overflow-x-auto px-1 pb-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {items.slice(0, 16).map((item) => (
          <PosterCard
            key={item.key}
            item={item}
            onActivate={activate}
            onToggleFavorite={toggleFavorite}
            onToggleWatchlist={toggleWatchlist}
          />
        ))}
      </div>
    </HomeModule>
  );
}
