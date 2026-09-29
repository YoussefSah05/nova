//! The Shows section's data: shows in progress with fresh cards, the week's
//! schedule, and warming the facts of followed shows so air dates are known.

import { useEffect, useMemo } from "react";
import { useQueryClient } from "@tanstack/react-query";

import type { ContinueWatchingItem, SeriesSummary } from "@/types";
import { titleFactsQuery } from "@/features/browse/detail/data";
import {
  browseKeys,
  useContinueWatching,
  useSeriesCards,
  useSeriesSchedule,
} from "@/features/browse/queries";
import { localDate, weekOf } from "@/features/browse/shows";
import { useSession } from "@/app/session";

/// Most shows the Continue module lists.
const MAX_IN_PROGRESS = 6;
/// Most followed shows whose facts one visit fetches.
const WARM_LIMIT = 24;

export function useShowsInProgress(): {
  items: ContinueWatchingItem[];
  cards: Map<string, SeriesSummary>;
} {
  const { profile, activeSourceId } = useSession();
  const { data } = useContinueWatching(profile.id, activeSourceId ?? -1, "series");
  const items = useMemo(
    () =>
      (data ?? [])
        .filter((i) => i.mediaKind === "episode" && i.seriesTitle)
        .slice(0, MAX_IN_PROGRESS),
    [data]
  );
  const titles = useMemo(() => items.map((i) => i.seriesTitle!), [items]);
  const { data: cards } = useSeriesCards(profile.id, activeSourceId, titles);
  const byTitle = useMemo(
    () => new Map((cards ?? []).map((c) => [c.seriesTitle, c] as const)),
    [cards]
  );
  return { items, cards: byTitle };
}

/// Fetch the facts of followed shows Nova hasn't described yet (one at a
/// time, once per session: the facts query caches them), then refresh the
/// schedule if any were fetched.
export function useWarmShowFacts(shows: SeriesSummary[]) {
  const qc = useQueryClient();
  const { profile, activeSourceId } = useSession();
  const ids = useMemo(
    () =>
      [...new Set(shows.map((s) => s.entityId).filter((id): id is string => !!id))].slice(
        0,
        WARM_LIMIT
      ),
    [shows]
  );
  const key = ids.join(",");
  useEffect(() => {
    if (!key || activeSourceId === null) return;
    let cancelled = false;
    void (async () => {
      let fetched = 0;
      for (const seriesId of key.split(",")) {
        if (cancelled) return;
        const query = titleFactsQuery(profile.id, { kind: "series", seriesId });
        if (qc.getQueryData(query.queryKey) !== undefined) continue;
        try {
          await qc.fetchQuery(query);
          fetched += 1;
        } catch {
          // Offline or TMDB off: the schedule shows what is known.
        }
      }
      if (fetched > 0 && !cancelled)
        void qc.invalidateQueries({
          queryKey: [...browseKeys.source(profile.id, activeSourceId), "seriesSchedule"],
        });
    })();
    return () => {
      cancelled = true;
    };
  }, [key, qc, profile.id, activeSourceId]);
}

/// The week's window and schedule (shared with the section summary).
export function useWeekSchedule() {
  const { profile, activeSourceId } = useSession();
  const today = localDate(new Date());
  const [y, m, d] = today.split("-").map(Number);
  const start = useMemo(() => new Date(y, m - 1, d), [y, m, d]);
  const end = localDate(new Date(y, m - 1, d + 6));
  const { data } = useSeriesSchedule(profile.id, activeSourceId, today, end);
  const shows = useMemo(() => data ?? [], [data]);
  const week = useMemo(() => weekOf(start, shows), [start, shows]);
  return { shows, week };
}
