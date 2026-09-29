//! Title artwork (TMDB title logo + text-free backdrop) for the title on
//! screen. Lazy: asked for only by the hero and detail views, cached by React
//! Query, and the backend fetches from TMDB at most once per title.

import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

import type { TitleArt } from "@/types";
import type { BrowseItem } from "./queries";
import { useScreenActive } from "@/lib/screenActive";

export type TitleArtSubject =
  | { kind: "movie"; entityId: string }
  | { kind: "series"; seriesId: string };

export function titleArtSubject(item: BrowseItem | null | undefined): TitleArtSubject | null {
  if (!item) return null;
  if (item.kind === "movie" && item.entityId) return { kind: "movie", entityId: item.entityId };
  if (item.kind === "series" && item.series?.entityId) {
    return { kind: "series", seriesId: item.series.entityId };
  }
  return null;
}

function subjectKey(s: TitleArtSubject): string {
  return s.kind === "movie" ? `movie:${s.entityId}` : `series:${s.seriesId}`;
}

export function useTitleArt(
  profileId: string,
  item: BrowseItem | null | undefined
): TitleArt | undefined {
  return useTitleArtFor(profileId, titleArtSubject(item));
}

/// The one definition of a title's art query (hooks and prefetch share it).
export function titleArtQuery(profileId: string, subject: TitleArtSubject | null) {
  return {
    queryKey: ["title-art", profileId, subject ? subjectKey(subject) : null] as const,
    queryFn: () => invoke<TitleArt>("get_title_art", { profileId, subject }),
    staleTime: 60 * 60 * 1000,
    retry: false,
  };
}

export function useTitleArtFor(
  profileId: string,
  subject: TitleArtSubject | null
): TitleArt | undefined {
  const { data } = useQuery({
    subscribed: useScreenActive(),
    ...titleArtQuery(profileId, subject),
    enabled: subject !== null,
  });
  return data ?? undefined;
}
