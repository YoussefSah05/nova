//! Movies, "Cinémathèque": ways into the film library before any grid — a
//! letterboxed spotlight, collections, decades and genres — then every film
//! on a windowed poster wall. Choosing a collection, decade or genre (or
//! searching) narrows the wall; the facets show as chips with the same model
//! the command palette will use for interpreted requests.

import { useCallback, useDeferredValue, useEffect, useMemo, useState } from "react";

import type { ContentSort, LibraryCollection } from "@/types";
import { SimpleSelect } from "@/components/ui/select";
import { BrowseBoundary } from "@/features/browse/BrowseBoundary";
import { BrowseGrid } from "@/features/browse/BrowseGrid";
import { Rail } from "@/features/browse/Rail";
import {
  RECENTLY_ADDED_RAIL,
  movieFacetApplied,
  useBrowseCount,
  useLibraryCollections,
  useMovieDecades,
  useMovieGenres,
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
import { CollectionsShelf, DecadesShelf, GenresShelf } from "./Shelves";
import { Spotlight } from "./Spotlight";

interface MovieFacets {
  query: string;
  genre: string | null;
  decade: number | null;
  collection: { id: number; name: string } | null;
}

const NO_FACETS: MovieFacets = { query: "", genre: null, decade: null, collection: null };

const SORTS: { value: ContentSort; label: string }[] = [
  { value: "recentlyAdded", label: "Recently added" },
  { value: "title", label: "A–Z" },
  { value: "year", label: "Newest first" },
  { value: "rating", label: "Best rated" },
];

export function MoviesSection() {
  const { profile, activeSourceId, sources, sourcesLoaded } = useSession();
  const scrollRef = useScreenScroll();
  const activate = useActivate();
  const { toggleFavorite, toggleWatchlist } = useLibraryActions();
  const [facets, setFacets] = useState<MovieFacets>(NO_FACETS);
  const [sort, setSort] = useState<ContentSort>("recentlyAdded");
  // Facets belong to the playlist they were picked in.
  useEffect(() => setFacets(NO_FACETS), [activeSourceId]);
  const query = useDeferredValue(facets.query.trim());

  const { data: count } = useBrowseCount("movies", profile.id, activeSourceId);
  const genresQuery = useMovieGenres(profile.id, activeSourceId);
  const decadesQuery = useMovieDecades(profile.id, activeSourceId);
  const genres = genresQuery.data;
  const decades = decadesQuery.data;
  const collections = useLibraryCollections(profile.id, activeSourceId).data;

  const filters = useMemo<BrowseFilters>(
    () => ({
      query: query || null,
      genre: facets.genre,
      yearFrom: facets.decade,
      yearTo: facets.decade === null ? null : facets.decade + 9,
      collectionId: facets.collection?.id ?? null,
      sort,
    }),
    [query, facets.genre, facets.decade, facets.collection, sort]
  );
  const narrowed = Boolean(query || facets.genre || facets.decade !== null || facets.collection);
  // Plain search is one relevance-ranked set from `search_movies`: no sort,
  // and no list count (it would count a different match).
  const plainSearch = Boolean(query) && !movieFacetApplied(filters);
  // The count does not depend on the order; keep sort out of its key.
  const countFilters = useMemo<BrowseFilters>(() => ({ ...filters, sort: undefined }), [filters]);
  const { data: matching } = useBrowseCount(
    "movies",
    profile.id,
    narrowed && !plainSearch ? activeSourceId : null,
    countFilters
  );

  const chips = useMemo<Facet[]>(() => {
    const out: Facet[] = [];
    if (facets.collection)
      out.push({
        kind: "collection",
        value: String(facets.collection.id),
        label: facets.collection.name,
        origin: "user",
      });
    if (facets.decade !== null)
      out.push({
        kind: "decade",
        value: String(facets.decade),
        label: `${facets.decade}s`,
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
        kind: "decade",
        options: (decades ?? []).map((d) => ({ value: String(d.decade), label: `${d.decade}s` })),
        anyLabel: "Any decade",
      },
    ],
    [genres, decades]
  );

  const onChange = useCallback((kind: FacetKind, value: string) => {
    setFacets((f) => {
      switch (kind) {
        case "query":
          return { ...f, query: value };
        case "genre":
          return { ...f, genre: value || null };
        case "decade":
          return { ...f, decade: value ? Number(value) : null };
        case "collection":
          return { ...f, collection: value ? f.collection : null };
        default:
          return f;
      }
    });
  }, []);
  const clear = useCallback(() => setFacets(NO_FACETS), []);
  const pickCollection = useCallback(
    (c: LibraryCollection) => setFacets((f) => ({ ...f, collection: { id: c.id, name: c.name } })),
    []
  );
  const pickDecade = useCallback(
    (decade: number) => setFacets((f) => ({ ...f, decade: f.decade === decade ? null : decade })),
    []
  );
  const pickGenre = useCallback(
    (genre: string) => setFacets((f) => ({ ...f, genre: f.genre === genre ? null : genre })),
    []
  );

  if (sourcesLoaded && sources.length === 0) return <NoPlaylist />;

  const summary =
    count === undefined ? null : `${count.toLocaleString()} ${count === 1 ? "film" : "films"}`;

  return (
    <div className="flex flex-col gap-10 px-[88px] pb-20 pt-[108px]">
      <div className="flex flex-col gap-6">
        <SectionHeader title="Cinémathèque" summary={summary} />
        <FacetBar
          searchLabel="Search movies"
          values={{
            query: facets.query,
            genre: facets.genre ?? "",
            decade: facets.decade === null ? "" : String(facets.decade),
          }}
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
                ariaLabel="Sort films"
                className="h-9 min-w-[150px] rounded-[10px] border-white/10 bg-white/[0.04]"
              />
            )
          }
        />
      </div>
      {activeSourceId !== null && (
        <BrowseBoundary resetKey={`movies-${activeSourceId}-${narrowed}`}>
          {narrowed ? (
            <section aria-labelledby="movies-matching" className="flex flex-col gap-[18px]">
              <h2 id="movies-matching" className="font-display text-[28px] font-medium">
                {plainSearch
                  ? `Best matches for “${query}”`
                  : matching === undefined
                    ? "Films"
                    : `${matching.toLocaleString()} ${matching === 1 ? "film" : "films"}`}
              </h2>
              <BrowseGrid
                tab="movies"
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
              <Spotlight />
              <CollectionsShelf collections={collections ?? []} onPick={pickCollection} />
              <DecadesShelf
                decades={decades}
                loading={decadesQuery.isPending}
                active={facets.decade}
                onPick={pickDecade}
              />
              <GenresShelf
                genres={genres}
                loading={genresQuery.isPending}
                active={facets.genre}
                onPick={pickGenre}
              />
              <Rail
                tab="movies"
                profileId={profile.id}
                sourceId={activeSourceId}
                spec={{ ...RECENTLY_ADDED_RAIL, title: "Just arrived" }}
                eager
                onActivate={activate}
                onToggleFavorite={toggleFavorite}
                onToggleWatchlist={toggleWatchlist}
              />
              <section aria-labelledby="movies-all" className="flex flex-col gap-[18px]">
                <h2 id="movies-all" className="font-display text-[28px] font-medium">
                  Every film
                </h2>
                <BrowseGrid
                  tab="movies"
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
