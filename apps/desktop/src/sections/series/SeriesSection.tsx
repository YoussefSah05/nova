//! Series, "Shows": shows are places to return to, so the section leads with
//! the ones in progress (wide cards with a tick per episode), then the week's
//! new episodes on a calendar, then every show on a windowed wall of wide
//! cards with their status. Searching, a genre or a status narrows the wall;
//! the facets show as chips from the same model as the other sections.

import { useCallback, useDeferredValue, useEffect, useMemo, useState } from "react";

import type { ContentSort, SeriesStatus } from "@/types";
import { SimpleSelect } from "@/components/ui/select";
import { BrowseBoundary } from "@/features/browse/BrowseBoundary";
import { BrowseGrid } from "@/features/browse/BrowseGrid";
import {
  FAVORITES_RAIL,
  WATCHLIST_RAIL,
  seriesFacetApplied,
  useBrowseCount,
  useRail,
  useSeriesGenres,
  type BrowseFilters,
} from "@/features/browse/queries";
import { FacetBar, type FacetMenu } from "@/ui/FacetBar";
import type { Facet, FacetKind } from "@/ui/facets";
import { SectionHeader } from "@/ui/SectionHeader";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { useScreenScroll } from "@/app/screenContext";
import { useSession } from "@/app/session";
import { NoPlaylist } from "@/sections/shared/NoPlaylist";
import { ContinueShows } from "./ContinueShows";
import { ThisWeek } from "./ThisWeek";
import { useShowsInProgress, useWarmShowFacts, useWeekSchedule } from "./useShows";

interface ShowFacets {
  query: string;
  genre: string | null;
  status: SeriesStatus | null;
}

const NO_FACETS: ShowFacets = { query: "", genre: null, status: null };

const SORTS: { value: ContentSort; label: string }[] = [
  { value: "recentlyAdded", label: "Recently updated" },
  { value: "title", label: "A–Z" },
  { value: "year", label: "Newest first" },
  { value: "rating", label: "Best rated" },
];

const STATUS_LABELS: Record<SeriesStatus, string> = {
  airing: "Still airing",
  ended: "Ended",
};

export function SeriesSection() {
  const { profile, activeSourceId, sources, sourcesLoaded } = useSession();
  const scrollRef = useScreenScroll();
  const activate = useActivate();
  const { toggleFavorite, toggleWatchlist } = useLibraryActions();
  const [facets, setFacets] = useState<ShowFacets>(NO_FACETS);
  const [sort, setSort] = useState<ContentSort>("recentlyAdded");
  // Facets belong to the playlist they were picked in.
  useEffect(() => setFacets(NO_FACETS), [activeSourceId]);
  const query = useDeferredValue(facets.query.trim());

  const { data: count } = useBrowseCount("series", profile.id, activeSourceId);
  const genres = useSeriesGenres(profile.id, activeSourceId).data;
  const progress = useShowsInProgress();
  const { shows: airing, week } = useWeekSchedule();
  const watchlist = useRail("series", profile.id, activeSourceId, WATCHLIST_RAIL, true).data;
  const favourites = useRail("series", profile.id, activeSourceId, FAVORITES_RAIL, true).data;
  const followed = useMemo(
    () => [
      ...progress.cards.values(),
      ...[...(watchlist ?? []), ...(favourites ?? [])].flatMap((i) => (i.series ? [i.series] : [])),
    ],
    [progress.cards, watchlist, favourites]
  );
  useWarmShowFacts(followed);

  const filters = useMemo<BrowseFilters>(
    () => ({ query: query || null, genre: facets.genre, status: facets.status, sort }),
    [query, facets.genre, facets.status, sort]
  );
  const narrowed = Boolean(query || facets.genre || facets.status);
  // Plain search is one relevance-ranked set: no sort, no list count.
  const plainSearch = Boolean(query) && !seriesFacetApplied(filters);
  const countFilters = useMemo<BrowseFilters>(() => ({ ...filters, sort: undefined }), [filters]);
  const { data: matching } = useBrowseCount(
    "series",
    profile.id,
    narrowed && !plainSearch ? activeSourceId : null,
    countFilters
  );

  const chips = useMemo<Facet[]>(() => {
    const out: Facet[] = [];
    if (facets.status)
      out.push({
        kind: "status",
        value: facets.status,
        label: STATUS_LABELS[facets.status],
        origin: "user",
      });
    if (facets.genre) out.push({ kind: "genre", value: facets.genre, origin: "user" });
    return out;
  }, [facets]);

  const menus = useMemo<FacetMenu[]>(
    () => [
      {
        kind: "genre",
        options: (genres ?? []).map((g) => ({ value: g.name, label: `${g.name} (${g.count})` })),
        anyLabel: "Any genre",
      },
      {
        kind: "status",
        options: (Object.keys(STATUS_LABELS) as SeriesStatus[]).map((value) => ({
          value,
          label: STATUS_LABELS[value],
        })),
        anyLabel: "Any status",
      },
    ],
    [genres]
  );

  const onChange = useCallback((kind: FacetKind, value: string) => {
    setFacets((f) => {
      switch (kind) {
        case "query":
          return { ...f, query: value };
        case "genre":
          return { ...f, genre: value || null };
        case "status":
          return { ...f, status: value === "airing" || value === "ended" ? value : null };
        default:
          return f;
      }
    });
  }, []);
  const clear = useCallback(() => setFacets(NO_FACETS), []);

  if (sourcesLoaded && sources.length === 0) return <NoPlaylist />;

  const summary =
    count === undefined
      ? null
      : [
          `${count.toLocaleString()} ${count === 1 ? "show" : "shows"}`,
          airing.length > 0 ? `${airing.length} with a new episode this week` : null,
        ]
          .filter(Boolean)
          .join(" · ");

  return (
    <div className="flex flex-col gap-10 px-[88px] pb-20 pt-[108px]">
      <div className="flex flex-col gap-6">
        <SectionHeader title="Shows" summary={summary} />
        <FacetBar
          searchLabel="Search shows"
          values={{ query: facets.query, genre: facets.genre ?? "", status: facets.status ?? "" }}
          menus={menus}
          facets={chips}
          onChange={onChange}
          onClear={clear}
          trailing={
            !plainSearch && (
              <SimpleSelect
                value={sort}
                options={SORTS}
                onChange={(v) => setSort(v)}
                ariaLabel="Sort shows"
                className="h-9 min-w-[160px] rounded-[10px] border-white/10 bg-white/[0.04]"
              />
            )
          }
        />
      </div>
      {activeSourceId !== null && (
        <BrowseBoundary resetKey={`series-${activeSourceId}-${narrowed}`}>
          {narrowed ? (
            <section aria-labelledby="shows-matching" className="flex flex-col gap-[18px]">
              <h2 id="shows-matching" className="font-display text-[28px] font-medium">
                {plainSearch
                  ? `Best matches for “${query}”`
                  : matching === undefined
                    ? "Shows"
                    : `${matching.toLocaleString()} ${matching === 1 ? "show" : "shows"}`}
              </h2>
              <BrowseGrid
                tab="series"
                layout="landscape"
                profileId={profile.id}
                sourceId={activeSourceId}
                filters={filters}
                scrollRef={scrollRef}
                onActivate={activate}
                onToggleFavorite={toggleFavorite}
                onToggleWatchlist={toggleWatchlist}
              />
            </section>
          ) : (
            <div className="flex flex-col gap-14">
              <ContinueShows items={progress.items} cards={progress.cards} />
              <ThisWeek week={week} />
              <section aria-labelledby="shows-all" className="flex flex-col gap-[18px]">
                <div className="flex items-baseline gap-3.5">
                  <h2 id="shows-all" className="font-display text-[28px] font-medium">
                    All shows
                  </h2>
                  <span className="text-sm text-white/60">
                    {SORTS.find((s) => s.value === sort)?.label}
                  </span>
                </div>
                <BrowseGrid
                  tab="series"
                  layout="landscape"
                  profileId={profile.id}
                  sourceId={activeSourceId}
                  filters={filters}
                  scrollRef={scrollRef}
                  onActivate={activate}
                  onToggleFavorite={toggleFavorite}
                  onToggleWatchlist={toggleWatchlist}
                />
              </section>
            </div>
          )}
        </BrowseBoundary>
      )}
    </div>
  );
}
