//! Search data: the plan a typed search becomes (text plus understood
//! facets), and one query per group — films, shows, channels, moments. A
//! plain text search uses the hybrid (meaning + words) commands; facets use
//! the filtered lists, with the remaining words as their text filter.

import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

import type { CanonicalMediaSummary, ChannelGroup, MomentHit, SeriesSummary } from "@/types";
import {
  browseKeys,
  channelGroupToItem,
  movieToItem,
  seriesToItem,
  type BrowseItem,
} from "@/features/browse/queries";
import { useScreenActive } from "@/lib/screenActive";
import type { Facet } from "@/ui/facets";
import type { SearchType } from "@/ui/interpret";

/// Results per group.
export const GROUP_LIMIT = { movies: 18, series: 8, live: 12, moments: 20 } as const;

export interface SearchPlan {
  text: string;
  genre: string | null;
  yearFrom: number | null;
  yearTo: number | null;
  /// Only these groups, when the search named one ("shows"); else all.
  types: SearchType[] | null;
}

export function planFrom(text: string, facets: Facet[]): SearchPlan {
  const get = (kind: Facet["kind"]) => facets.find((f) => f.kind === kind)?.value ?? null;
  const decade = get("decade");
  const year = get("year");
  const types = facets.filter((f) => f.kind === "type").map((f) => f.value as SearchType);
  return {
    text: text.trim(),
    genre: get("genre"),
    yearFrom: year ? Number(year) : decade ? Number(decade) : null,
    yearTo: year ? Number(year) : decade ? Number(decade) + 9 : null,
    types: types.length > 0 ? types : null,
  };
}

export function shows(plan: SearchPlan, type: SearchType): boolean {
  return plan.types === null || plan.types.includes(type);
}

const faceted = (p: SearchPlan) => p.genre !== null || p.yearFrom !== null;

function key(profileId: string, sourceId: number | null, group: string, plan: SearchPlan) {
  return [...browseKeys.source(profileId, sourceId ?? -1), "search", group, plan] as const;
}

export function useSearchMovies(profileId: string, sourceId: number | null, plan: SearchPlan) {
  const enabled =
    sourceId !== null &&
    shows(plan, "movies") &&
    (!!plan.text || faceted(plan) || plan.types !== null);
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: key(profileId, sourceId, "movies", plan),
    queryFn: async (): Promise<BrowseItem[]> => {
      const rows =
        faceted(plan) || !plan.text
          ? await invoke<CanonicalMediaSummary[]>("list_movies", {
              profileId,
              sourceId,
              limit: GROUP_LIMIT.movies,
              offset: 0,
              filters: {
                query: plan.text || null,
                genre: plan.genre,
                yearFrom: plan.yearFrom,
                yearTo: plan.yearTo,
                sort: "rating",
              },
            })
          : await invoke<CanonicalMediaSummary[]>("search_movies", {
              profileId,
              sourceId,
              query: plan.text,
              limit: GROUP_LIMIT.movies,
            });
      return rows.map(movieToItem);
    },
    enabled,
  });
}

/// Shows have no year filter: a search with a year or decade names films.
export function seriesApply(plan: SearchPlan): boolean {
  return (
    shows(plan, "series") &&
    plan.yearFrom === null &&
    (!!plan.text || plan.genre !== null || plan.types !== null)
  );
}

export function useSearchSeries(profileId: string, sourceId: number | null, plan: SearchPlan) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: key(profileId, sourceId, "series", plan),
    queryFn: async (): Promise<BrowseItem[]> => {
      const rows =
        plan.genre !== null || !plan.text
          ? await invoke<SeriesSummary[]>("list_series", {
              profileId,
              sourceId,
              limit: GROUP_LIMIT.series,
              offset: 0,
              filters: { query: plan.text || null, genre: plan.genre, sort: "rating" },
            })
          : await invoke<SeriesSummary[]>("semantic_search_series", {
              profileId,
              sourceId,
              query: plan.text,
              limit: GROUP_LIMIT.series,
            });
      return rows.map(seriesToItem);
    },
    enabled: sourceId !== null && seriesApply(plan),
  });
}

/// Channels are found by name or subject; genres and years don't apply.
export function channelsApply(plan: SearchPlan): boolean {
  return shows(plan, "live") && (!!plan.text || plan.types !== null) && !faceted(plan);
}

export function useSearchChannels(profileId: string, sourceId: number | null, plan: SearchPlan) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: key(profileId, sourceId, "live", plan),
    queryFn: async (): Promise<BrowseItem[]> =>
      (
        await invoke<ChannelGroup[]>(
          plan.text ? "semantic_search_channels" : "list_channel_groups",
          {
            profileId,
            sourceId,
            query: plan.text,
            limit: GROUP_LIMIT.live,
            ...(!plan.text ? { offset: 0 } : {}),
          }
        )
      ).map(channelGroupToItem),
    enabled: sourceId !== null && channelsApply(plan),
  });
}

/// Lines of dialogue: words only, whatever the facets.
export function momentsApply(plan: SearchPlan): boolean {
  return Array.from(plan.text.trim()).length >= 2 && plan.types === null;
}

export function useSearchMoments(profileId: string, sourceId: number | null, plan: SearchPlan) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: key(profileId, sourceId, "moments", { ...plan, genre: null }),
    queryFn: () =>
      invoke<MomentHit[]>("search_moments", {
        profileId,
        sourceId,
        query: plan.text,
        limit: GROUP_LIMIT.moments,
      }),
    enabled: sourceId !== null && momentsApply(plan),
  });
}

/// Whether meaning-based search can run (its model is installed).
export function useSemanticSearchAvailable() {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: ["capability", "semanticSearch"] as const,
    queryFn: async () =>
      (await invoke<{ installed: boolean }>("get_enrich_model_status")).installed,
    staleTime: 5 * 60_000,
    retry: false,
  });
}
