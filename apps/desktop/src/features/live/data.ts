//! Live data: the playlist overview, the guide's channel rows (paged, or a
//! bundle's channels), and the programmes of the rows on screen, fetched in
//! blocks so a scroll through thousands of channels reads only what shows.

import {
  keepPreviousData,
  useInfiniteQuery,
  useQueries,
  useQuery,
  type InfiniteData,
} from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useMemo, useRef } from "react";

import type { Channel, ChannelGuide, LiveOverview, LiveChannelSummary, Program } from "@/types";
import { liveChannelToItem, browseKeys } from "@/features/browse/queries";
import { guideKeys } from "@/features/browse/guide";
import { useScreenActive } from "@/lib/screenActive";
import type { GuideWindow } from "./timeline";

/// Channel rows per page of the guide.
export const GUIDE_PAGE = 200;
/// Channel rows per programme request.
export const GUIDE_BLOCK = 30;

/// Which channels the guide lists: a slice of the playlist, or one bundle.
export type GuideScope =
  | {
      kind: "playlist";
      category: string | null;
      region: string | null;
      country: string | null;
      favoritesOnly: boolean;
      query: string | null;
    }
  | { kind: "bundle"; group: string };

export type GuideChannelPages = InfiniteData<Channel[], number>;

export function useLiveOverview(profileId: string, sourceId: number | null) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: [...browseKeys.source(profileId, sourceId ?? -1), "liveOverview"] as const,
    queryFn: () => invoke<LiveOverview>("list_live_overview", { profileId, sourceId }),
    enabled: sourceId !== null,
    staleTime: 60_000,
  });
}

/// The guide's channel rows, a page at a time (a bundle is one page).
export function useGuideChannels(profileId: string, sourceId: number | null, scope: GuideScope) {
  return useInfiniteQuery({
    subscribed: useScreenActive(),
    queryKey: [...browseKeys.source(profileId, sourceId ?? -1), "guideChannels", scope] as const,
    queryFn: async ({ pageParam }): Promise<Channel[]> => {
      const rows = await invoke<LiveChannelSummary[]>("list_live_channels", {
        profileId,
        sourceId,
        limit: GUIDE_PAGE,
        offset: pageParam,
        group: scope.kind === "bundle" ? scope.group : null,
        filters:
          scope.kind === "playlist"
            ? {
                category: scope.category,
                region: scope.region,
                country: scope.country,
                favoritesOnly: scope.favoritesOnly,
                query: scope.query,
              }
            : null,
      });
      return rows.map((c) => liveChannelToItem(c).channel).filter((c): c is Channel => !!c);
    },
    initialPageParam: 0,
    getNextPageParam: (last, _pages, offset) =>
      last.length === GUIDE_PAGE ? offset + GUIDE_PAGE : undefined,
    enabled: sourceId !== null,
    // A new search keeps the rows on screen until its results arrive.
    placeholderData: keepPreviousData,
  });
}

export interface GuidePrograms {
  /// Programmes by channel id; a missing entry is still loading.
  programs: Map<number, Program[]>;
  /// Channels whose programmes could not be read.
  failed: Set<number>;
}

/// Programmes of the guide rows in `[first, last]` (and the selected row),
/// by channel id. Rows are read in fixed blocks so scrolling reuses what's
/// cached; while the window moves to the next half hour, rows keep their
/// previous programmes (clipped to the new window) until the new ones land.
export function useGuidePrograms(
  profileId: string,
  channels: Channel[],
  first: number,
  last: number,
  window: GuideWindow,
  alsoIndex: number = -1
): GuidePrograms {
  const subscribed = useScreenActive();
  const blocks = useMemo(() => {
    if (channels.length === 0) return [];
    const wanted = new Set<number>();
    if (last >= first) {
      const to = Math.floor(Math.min(last, channels.length - 1) / GUIDE_BLOCK);
      for (let b = Math.floor(first / GUIDE_BLOCK); b <= to; b++) wanted.add(b);
    }
    if (alsoIndex >= 0 && alsoIndex < channels.length)
      wanted.add(Math.floor(alsoIndex / GUIDE_BLOCK));
    return [...wanted]
      .sort((a, b) => a - b)
      .map((b) => channels.slice(b * GUIDE_BLOCK, (b + 1) * GUIDE_BLOCK).map((c) => c.id));
  }, [channels, first, last, alsoIndex]);
  const results = useQueries({
    // Per-query `subscribed` is ignored by useQueries: it goes here.
    subscribed,
    queries: blocks.map((ids) => ({
      queryKey: [...guideKeys.all, profileId, "grid", window.from, ids] as const,
      queryFn: () =>
        invoke<ChannelGuide[]>("epg_guide", {
          profileId,
          channelIds: ids,
          fromUtc: window.from,
          toUtc: window.to,
        }),
      staleTime: 5 * 60_000,
      retry: false,
    })),
  });
  // What each channel last showed, across window moves.
  const last_ = useRef(new Map<number, Program[]>());
  const previousScope = useRef("");
  const currentScope = `${profileId}:${channels[0]?.sourceId ?? ""}`;
  if (previousScope.current !== currentScope) {
    last_.current.clear();
    previousScope.current = currentScope;
  }
  // Rebuilt each render (a few blocks); each channel's programme array keeps
  // its identity, so memoized rows don't re-render.
  const programs = new Map<number, Program[]>();
  const failed = new Set<number>();
  blocks.forEach((ids, i) => {
    const r = results[i];
    if (r?.data) {
      for (const g of r.data) {
        programs.set(g.channelId, g.programs);
        last_.current.set(g.channelId, g.programs);
      }
      return;
    }
    for (const id of ids) {
      const previous = last_.current.get(id);
      if (previous) programs.set(id, previous);
      else if (r?.isError) failed.add(id);
    }
  });
  return { programs, failed };
}
