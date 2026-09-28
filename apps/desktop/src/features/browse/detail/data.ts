//! Detail-page data: facts (TMDB + provider), season episodes and where a
//! series continues. React Query caches them; closing the player refreshes
//! progress (`invalidateDetailProgress`).

import { useQuery, type QueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

import type {
  ContinueWatchingItem,
  EpisodeView,
  SeriesEpisode,
  SeriesSummary,
  TitleFactsView,
} from "@/types";
import type { TitleArtSubject } from "../titleArt";

const subjectKey = (s: TitleArtSubject) =>
  s.kind === "movie" ? `movie:${s.entityId}` : `series:${s.sourceId}:${s.seriesTitle}`;

export function useTitleFacts(profileId: string, subject: TitleArtSubject | null) {
  return useQuery({
    queryKey: ["title-facts", profileId, subject ? subjectKey(subject) : null],
    queryFn: () => invoke<TitleFactsView>("get_title_facts", { profileId, subject }),
    enabled: subject !== null,
    staleTime: 30 * 60 * 1000,
    retry: false,
  });
}

const seriesArgs = (profileId: string, series: SeriesSummary) => ({
  profileId,
  sourceId: series.sourceId,
  seriesTitle: series.seriesTitle,
});

/// All provider episodes (fetched from an Xtream panel on first open): the
/// season list comes from here.
export function useSeriesEpisodes(profileId: string, series: SeriesSummary) {
  return useQuery({
    queryKey: ["series-episodes", profileId, series.sourceId, series.seriesTitle],
    queryFn: () => invoke<SeriesEpisode[]>("list_series_episodes", seriesArgs(profileId, series)),
    staleTime: 5 * 60 * 1000,
  });
}

export function useSeasonEpisodes(profileId: string, series: SeriesSummary, season: number | null) {
  return useQuery({
    queryKey: ["season-episodes", profileId, series.sourceId, series.seriesTitle, season],
    queryFn: () =>
      invoke<EpisodeView[]>("get_season_episodes", { ...seriesArgs(profileId, series), season }),
    enabled: season !== null,
    staleTime: 5 * 60 * 1000,
  });
}

export function useSeriesResume(profileId: string, series: SeriesSummary) {
  return useQuery({
    queryKey: ["series-resume", profileId, series.sourceId, series.seriesTitle],
    queryFn: () =>
      invoke<ContinueWatchingItem | null>("get_series_resume", seriesArgs(profileId, series)),
    staleTime: 0,
  });
}

/// Watching changes resume points and episode progress.
export function invalidateDetailProgress(qc: QueryClient, profileId: string) {
  for (const key of ["series-resume", "season-episodes", "canonical-movie"]) {
    void qc.invalidateQueries({ queryKey: [key, profileId] });
  }
}
