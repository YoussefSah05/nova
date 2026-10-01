import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";
import { browseKeys } from "@/features/browse/queries";
import { useScreenActive } from "@/lib/screenActive";
import type { LibraryHistoryItem } from "@/types/generated/LibraryHistoryItem";

export type HistoryKind = "all" | LibraryHistoryItem["mediaKind"];
export const HISTORY_PAGE_SIZE = 50;

export function useLibraryHistory(
  profileId: string,
  sourceId: number,
  kind: HistoryKind,
  offset: number
) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: [...browseKeys.source(profileId, sourceId), "history", kind, offset],
    queryFn: () =>
      invoke<LibraryHistoryItem[]>("list_library_history", {
        profileId,
        sourceId,
        kind: kind === "all" ? null : kind,
        limit: HISTORY_PAGE_SIZE + 1,
        offset,
      }),
    gcTime: 0,
  });
}

export function historyDay(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}

export function historySubtitle(item: LibraryHistoryItem): string {
  if (item.mediaKind === "live") return "Channel";
  if (item.mediaKind === "vod") return "Film";
  const code =
    item.numbered && item.season !== null && item.episode !== null
      ? `S${String(item.season).padStart(2, "0")} · E${String(item.episode).padStart(2, "0")}`
      : "Episode";
  return [code, item.episodeTitle].filter(Boolean).join(" · ");
}

export function historyProgress(item: LibraryHistoryItem): string {
  if (item.mediaKind === "live") return "Watched live";
  if (item.completed) return "Completed";
  const minutes = Math.floor(Math.max(0, item.positionSec) / 60);
  const position = minutes > 0 ? `${minutes} min` : "Less than a minute";
  return item.durationSec && item.durationSec > 0
    ? `${position} of ${Math.ceil(item.durationSec / 60)} min`
    : `${position} watched`;
}
