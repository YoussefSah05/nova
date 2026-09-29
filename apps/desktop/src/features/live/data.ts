//! Live data: the playlist overview, the guide's channel rows (paged, or a
//! bundle's channels), and the programmes of the rows on screen, fetched in
//! blocks so a scroll through thousands of channels reads only what shows.

import { useInfiniteQuery, useQueries, useQuery, type InfiniteData } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useMemo } from "react";

import type { Channel, ChannelGuide, LiveOverview, Program } from "@/types";
import { browseKeys } from "@/features/browse/queries";
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
    queryFn: ({ pageParam }): Promise<Channel[]> =>
      scope.kind === "bundle"
        ? invoke<Channel[]>("list_channels_in_group", { profileId, sourceId, group: scope.group })
        : invoke<Channel[]>("list_channels", {
            profileId,
            sourceId,
            limit: GUIDE_PAGE,
            offset: pageParam,
            filters: {
              category: scope.category,
              favoritesOnly: scope.favoritesOnly,
              query: scope.query,
            },
          }),
    initialPageParam: 0,
    getNextPageParam: (last, _pages, offset) =>
      scope.kind === "playlist" && last.length === GUIDE_PAGE ? offset + GUIDE_PAGE : undefined,
    enabled: sourceId !== null,
  });
}

/// Programmes of the guide rows in `[first, last]`, by channel id. Rows are
/// read in fixed blocks so scrolling reuses what's cached.
export function useGuidePrograms(
  profileId: string,
  channels: Channel[],
  first: number,
  last: number,
  window: GuideWindow,
  /// A row read even when scrolled away (the selected channel).
  alsoIndex: number = -1
): Map<number, Program[]> {
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
    queries: blocks.map((ids) => ({
      subscribed,
      queryKey: [...guideKeys.all, profileId, "grid", window.from, ids] as const,
      queryFn: () =>
        invoke<ChannelGuide[]>("epg_guide", {
          profileId,
          channelIds: ids,
          fromUtc: window.from,
          toUtc: window.to,
        }),
      staleTime: 5 * 60_000,
      placeholderData: (previous: ChannelGuide[] | undefined) => previous,
      retry: false,
    })),
  });
  // Rebuilt each render (a few blocks); each channel's programme array keeps
  // its identity, so memoized rows don't re-render.
  const map = new Map<number, Program[]>();
  for (const r of results) for (const g of r.data ?? []) map.set(g.channelId, g.programs);
  return map;
}
