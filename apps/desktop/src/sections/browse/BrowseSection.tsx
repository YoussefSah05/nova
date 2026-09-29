//! Movies, Series and Live until each gets its own design (UI-2..UI-4): the
//! section's header and filter bar over today's rails, or over the windowed
//! grid while a search or facet is applied.

import { useCallback, useDeferredValue, useEffect, useMemo, useState } from "react";

import { BrowseBoundary } from "@/features/browse/BrowseBoundary";
import { BrowseGrid } from "@/features/browse/BrowseGrid";
import { BrowseRails } from "@/features/browse/BrowseRails";
import {
  useBrowseCount,
  useCategories,
  useCountries,
  useRegions,
  type BrowseTab,
} from "@/features/browse/queries";
import { FacetBar, type FacetMenu } from "@/ui/FacetBar";
import { activeFacets, facetFilters, type FacetKind, type FacetValues } from "@/ui/facets";
import { SectionHeader } from "@/ui/SectionHeader";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { usePlayback } from "@/app/playback";
import { useScreenScroll } from "@/app/screenContext";
import { useSession } from "@/app/session";
import { NoPlaylist } from "@/sections/shared/NoPlaylist";

/// A stable empty list: `?? []` would be a new array on every render.
const NONE: string[] = [];

const COPY: Record<BrowseTab, { title: string; noun: [string, string]; search: string }> = {
  movies: { title: "Cinémathèque", noun: ["film", "films"], search: "Search movies" },
  series: { title: "Shows", noun: ["series", "series"], search: "Search series" },
  live: { title: "Channels", noun: ["channel", "channels"], search: "Search live" },
};

export function BrowseSection({ tab }: { tab: BrowseTab }) {
  const { profile, activeSourceId, sources, sourcesLoaded } = useSession();
  const scrollRef = useScreenScroll();
  const activate = useActivate();
  const { playMedia } = usePlayback();
  const { toggleFavorite, toggleWatchlist } = useLibraryActions();
  const [values, setValues] = useState<FacetValues>({});
  // Facets belong to the playlist they were picked in.
  useEffect(() => setValues({}), [activeSourceId]);
  const deferredQuery = useDeferredValue(values.query ?? "");
  const live = tab === "live";

  const facets = useMemo(
    () => activeFacets({ ...values, query: deferredQuery }),
    [values, deferredQuery]
  );
  const filters = useMemo(() => facetFilters(facets, live), [facets, live]);
  const filtering = Boolean(filters.query || filters.category || filters.country || filters.region);

  const { data: count } = useBrowseCount(tab, profile.id, activeSourceId);
  const categories = useCategories(tab, profile.id, activeSourceId).data ?? NONE;
  const regions = useRegions(profile.id, live ? activeSourceId : null).data ?? NONE;
  const countries = useCountries(profile.id, live ? activeSourceId : null).data ?? NONE;

  const menus = useMemo<FacetMenu[]>(
    () => [
      { kind: "category", options: categories, anyLabel: "Any genre" },
      ...(live
        ? ([
            { kind: "region", options: regions, anyLabel: "Any region" },
            { kind: "country", options: countries, anyLabel: "Any country" },
          ] as FacetMenu[])
        : []),
    ],
    [categories, regions, countries, live]
  );
  const chips = useMemo(() => facets.filter((f) => f.kind !== "query"), [facets]);
  const clear = useCallback(() => setValues({}), []);

  const copy = COPY[tab];
  const summary =
    count === undefined
      ? null
      : `${new Intl.NumberFormat().format(count)} ${count === 1 ? copy.noun[0] : copy.noun[1]}`;

  const setFacet = useCallback(
    (kind: FacetKind, value: string) => setValues((v) => ({ ...v, [kind]: value })),
    []
  );

  if (sourcesLoaded && sources.length === 0) return <NoPlaylist />;

  return (
    <div className="flex flex-col gap-6 px-[88px] pb-16 pt-[108px]">
      <SectionHeader title={copy.title} summary={summary} />
      <FacetBar
        searchLabel={copy.search}
        values={values}
        menus={menus}
        facets={chips}
        onChange={setFacet}
        onClear={clear}
      />
      {activeSourceId !== null && (
        <BrowseBoundary resetKey={`${tab}-${activeSourceId}-${filtering}`}>
          {filtering ? (
            <BrowseGrid
              tab={tab}
              profileId={profile.id}
              sourceId={activeSourceId}
              filters={filters}
              scrollRef={scrollRef}
              onActivate={activate}
              onToggleFavorite={toggleFavorite}
              onToggleWatchlist={toggleWatchlist}
            />
          ) : (
            <BrowseRails
              tab={tab}
              profileId={profile.id}
              sourceId={activeSourceId}
              scrollRef={scrollRef}
              onPlay={playMedia}
              onSelectCategory={(category) => setFacet("category", category)}
              onActivate={activate}
              onToggleFavorite={toggleFavorite}
              onToggleWatchlist={toggleWatchlist}
            />
          )}
        </BrowseBoundary>
      )}
    </div>
  );
}
