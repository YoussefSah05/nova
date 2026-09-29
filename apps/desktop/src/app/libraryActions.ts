//! Favourite / watchlist from any card or page: an optimistic flip everywhere
//! the title shows (cached rails and grids, and any open page in any
//! section's stack), confirmed by Rust or rolled back.

import { useCallback, useMemo } from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  browseKeys,
  invalidateMembershipRails,
  patchItemEverywhere,
  seriesToItem,
  type BrowseItem,
  type BrowseTab,
} from "@/features/browse/queries";
import { invokeFavorite, invokeWatchlist } from "@/features/browse/toggles";
import type { SeriesSummary } from "@/types";
import { useNavActions } from "./navigation/useNav";
import { errorMessage, useSession } from "./session";

function tabForKind(kind: BrowseItem["kind"]): BrowseTab {
  return kind === "live" ? "live" : kind === "series" ? "series" : "movies";
}

type Flag = "isFavorite" | "isInWatchlist";

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
