//! Favourite / watchlist from any card or page: an optimistic flip everywhere
//! the title shows (cached rails and grids, and any open page in any
//! section's stack), confirmed by Rust or rolled back.

import { useCallback, useMemo } from "react";
import { useQueryClient, type QueryClient, type QueryKey } from "@tanstack/react-query";

import {
  browseKeys,
  invalidateMembershipRails,
  patchItemEverywhere,
  seriesKey,
  seriesToItem,
  type BrowseItem,
  type BrowseTab,
} from "@/features/browse/queries";
import { invokeFavorite, invokeWatchlist } from "@/features/browse/toggles";
import type { CanonicalMediaSummary, SeriesSummary } from "@/types";
import { useNavActions } from "./navigation/useNav";
import { errorMessage, useSession } from "./session";

function tabForKind(kind: BrowseItem["kind"]): BrowseTab {
  return kind === "live" ? "live" : kind === "series" ? "series" : "movies";
}

type Flag = "isFavorite" | "isInWatchlist";

interface DiscoveryData {
  collection?: CanonicalMediaSummary[];
  movies?: CanonicalMediaSummary[];
  series?: SeriesSummary[];
}

/// Discovery lists (Home's "Because you watched", trending, a page's "More
/// like this") hold their own copies of titles: flip the flag there too.
/// Returns the rollback.
function patchDiscovery(
  qc: QueryClient,
  profileId: string,
  key: string,
  patch: Partial<Record<Flag, boolean>>
): () => void {
  const before: [QueryKey, DiscoveryData][] = [];
  for (const prefix of ["related-titles", "trending-library"]) {
    for (const [queryKey, data] of qc.getQueriesData<DiscoveryData>({
      queryKey: [prefix, profileId],
    })) {
      if (!data) continue;
      let changed = false;
      const movie = (m: CanonicalMediaSummary) =>
        `movie-${m.entityId}` === key ? ((changed = true), { ...m, ...patch }) : m;
      const show = (s: SeriesSummary) =>
        seriesKey(s) === key ? ((changed = true), { ...s, ...patch }) : s;
      const next: DiscoveryData = {
        ...data,
        collection: data.collection?.map(movie),
        movies: data.movies?.map(movie),
        series: data.series?.map(show),
      };
      if (changed) {
        before.push([queryKey, data]);
        qc.setQueryData(queryKey, next);
      }
    }
  }
  return () => before.forEach(([queryKey, data]) => qc.setQueryData(queryKey, data));
}

export interface LibraryActions {
  toggleFavorite: (item: BrowseItem, next: boolean) => Promise<void>;
  toggleWatchlist: (item: BrowseItem, next: boolean) => Promise<void>;
  toggleSeriesFavorite: (series: SeriesSummary, next: boolean) => Promise<void>;
  toggleSeriesWatchlist: (series: SeriesSummary, next: boolean) => Promise<void>;
}

export function useLibraryActions(): LibraryActions {
  const qc = useQueryClient();
  const { profile, activeSourceId, setError } = useSession();
  const { patchItem } = useNavActions();

  const toggle = useCallback(
    async (item: BrowseItem, flag: Flag, next: boolean) => {
      if (activeSourceId === null) return;
      const rollback = patchItemEverywhere(qc, profile.id, activeSourceId, item.key, {
        [flag]: next,
      });
      const rollbackDiscovery = patchDiscovery(qc, profile.id, item.key, { [flag]: next });
      patchItem(item.key, { [flag]: next });
      try {
        await (flag === "isFavorite"
          ? invokeFavorite(item, profile.id, next)
          : invokeWatchlist(item, profile.id, next));
        invalidateMembershipRails(qc, profile.id, activeSourceId, tabForKind(item.kind));
        if (flag === "isFavorite") {
          void qc.invalidateQueries({ queryKey: browseKeys.continueWatching(profile.id) });
        }
      } catch (e: unknown) {
        rollback();
        rollbackDiscovery();
        patchItem(item.key, { [flag]: !next });
        setError(errorMessage(e));
      }
    },
    [qc, profile.id, activeSourceId, patchItem, setError]
  );

  return useMemo(
    () => ({
      toggleFavorite: (item, next) => toggle(item, "isFavorite", next),
      toggleWatchlist: (item, next) => toggle(item, "isInWatchlist", next),
      toggleSeriesFavorite: (series, next) => toggle(seriesToItem(series), "isFavorite", next),
      toggleSeriesWatchlist: (series, next) => toggle(seriesToItem(series), "isInWatchlist", next),
    }),
    [toggle]
  );
}
