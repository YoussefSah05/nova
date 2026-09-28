//! Detail-page data: facts (TMDB + provider), season episodes and where a
//! series continues. React Query caches them; closing the player refreshes
//! progress (`invalidateDetailProgress`).

import { useQuery, type QueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

import type {
  CanonicalMediaSummary,
  ContinueWatchingItem,
  EpisodeView,
  SeriesEpisode,
  SeriesSummary,
  TitleFactsView,
} from "@/types";
import type { TitleArtSubject } from "../titleArt";

const subjectKey = (s: TitleArtSubject) =>
  s.kind === "movie" ? `movie:${s.entityId}` : `series:${s.seriesId}`;

export function useTitleFacts(profileId: string, subject: TitleArtSubject | null) {
  return useQuery({
    queryKey: ["title-facts", profileId, subject ? subjectKey(subject) : null],
    queryFn: () => invoke<TitleFactsView>("get_title_facts", { profileId, subject }),
    enabled: subject !== null,
    staleTime: 30 * 60 * 1000,
    retry: false,
  });
}

/// Series are addressed by their canonical entity: Rust resolves where the
/// series is listed now, so a provider rename can't leave a stale reference.
const seriesArgs = (profileId: string, series: SeriesSummary) => ({
  profileId,
  seriesId: series.entityId,
});

/// All provider episodes (fetched from an Xtream panel on first open): the
/// season list comes from here.
export function useSeriesEpisodes(profileId: string, series: SeriesSummary) {
  return useQuery({
    queryKey: ["series-episodes", profileId, series.entityId],
    queryFn: () => invoke<SeriesEpisode[]>("list_series_episodes", seriesArgs(profileId, series)),
    enabled: series.entityId !== null,
    staleTime: 5 * 60 * 1000,
  });
}

export function useSeasonEpisodes(profileId: string, series: SeriesSummary, season: number | null) {
  return useQuery({
    queryKey: ["season-episodes", profileId, series.entityId, season],
    queryFn: () =>
      invoke<EpisodeView[]>("get_season_episodes", { ...seriesArgs(profileId, series), season }),
    enabled: season !== null && series.entityId !== null,
    staleTime: 5 * 60 * 1000,
  });
}

export function useSeriesResume(profileId: string, series: SeriesSummary) {
  return useQuery({
    queryKey: ["series-resume", profileId, series.entityId],
    queryFn: () =>
      invoke<ContinueWatchingItem | null>("get_series_resume", seriesArgs(profileId, series)),
    enabled: series.entityId !== null,
    staleTime: 0,
  });
}

export interface RelatedTitles {
  collectionName: string | null;
  collection: CanonicalMediaSummary[];
  movies: CanonicalMediaSummary[];
  series: SeriesSummary[];
}

export interface TrendingTitles {
  movies: CanonicalMediaSummary[];
  series: SeriesSummary[];
}

/// "More like this" + collection: library titles only (empty with TMDB off).
export function useRelatedTitles(
  profileId: string,
  sourceId: number | null,
  subject: TitleArtSubject | null
) {
  return useQuery({
    queryKey: ["related-titles", profileId, sourceId, subject ? subjectKey(subject) : null],
    queryFn: () => invoke<RelatedTitles>("get_related_titles", { profileId, sourceId, subject }),
    enabled: subject !== null && sourceId !== null,
    staleTime: 30 * 60 * 1000,
    retry: false,
  });
}

/// This week's trending works that the library has.
export function useTrendingInLibrary(
  profileId: string,
  sourceId: number,
  section: "movies" | "series" | null
) {
  return useQuery({
    queryKey: ["trending-library", profileId, sourceId, section],
    queryFn: () =>
      invoke<TrendingTitles>("get_trending_in_library", { profileId, sourceId, section }),
    enabled: section !== null,
    staleTime: 60 * 60 * 1000,
    retry: false,
  });
}

/// Watching changes resume points and episode progress.
export function invalidateDetailProgress(qc: QueryClient, profileId: string) {
  for (const key of ["series-resume", "season-episodes", "canonical-movie"]) {
    void qc.invalidateQueries({ queryKey: [key, profileId] });
  }
}
