//! Title artwork (TMDB title logo + text-free backdrop) for the title on
//! screen. Lazy: asked for only by the hero and detail views, cached by React
//! Query, and the backend fetches from TMDB at most once per title.

import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

import type { TitleArt } from "@/types";
import type { BrowseItem } from "./queries";

export type TitleArtSubject =
  | { kind: "movie"; entityId: string }
  | { kind: "series"; sourceId: number; seriesTitle: string };

export function titleArtSubject(item: BrowseItem | null | undefined): TitleArtSubject | null {
  if (!item) return null;
  if (item.kind === "movie" && item.entityId) return { kind: "movie", entityId: item.entityId };
  if (item.kind === "series" && item.series) {
    return {
      kind: "series",
      sourceId: item.series.sourceId,
      seriesTitle: item.series.seriesTitle,
    };
  }
  return null;
}

function subjectKey(s: TitleArtSubject): string {
  return s.kind === "movie" ? `movie:${s.entityId}` : `series:${s.sourceId}:${s.seriesTitle}`;
}

export function useTitleArt(
  profileId: string,
  item: BrowseItem | null | undefined
): TitleArt | undefined {
  const subject = titleArtSubject(item);
  const { data } = useQuery({
    queryKey: ["title-art", profileId, subject ? subjectKey(subject) : null],
    queryFn: () => invoke<TitleArt>("get_title_art", { profileId, subject }),
    enabled: subject !== null,
    staleTime: 60 * 60 * 1000,
    retry: false,
  });
  return data ?? undefined;
}
