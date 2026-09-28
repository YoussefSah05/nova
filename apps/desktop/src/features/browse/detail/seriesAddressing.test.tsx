import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

import type { SeriesSummary } from "@/types";
import { seriesKey, seriesToItem } from "../queries";
import { titleArtSubject } from "../titleArt";
import { useSeasonEpisodes, useSeriesEpisodes, useSeriesResume } from "./data";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

function series(entityId: string | null): SeriesSummary {
  return {
    seriesTitle: "FR Dark",
    sourceId: 3,
    category: null,
    episodeCount: 10,
    seasonCount: 1,
    posterUrl: null,
    isFavorite: false,
    isInWatchlist: false,
    addedAt: null,
    canonicalTitle: "Dark",
    backdropUrl: null,
    overview: null,
    displayTitle: "Dark",
    entityId,
  };
}

function wrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

beforeEach(() => mocks.invoke.mockReset());

it("addresses a series by its canonical entity, never by its provider title", async () => {
  mocks.invoke.mockResolvedValue([]);
  const s = series("ser-1");
  const w = wrapper();
  renderHook(() => useSeriesEpisodes("p", s), { wrapper: w });
  renderHook(() => useSeasonEpisodes("p", s, 2), { wrapper: w });
  renderHook(() => useSeriesResume("p", s), { wrapper: w });
  await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(3));
  expect(mocks.invoke).toHaveBeenCalledWith("list_series_episodes", {
    profileId: "p",
    seriesId: "ser-1",
  });
  expect(mocks.invoke).toHaveBeenCalledWith("get_season_episodes", {
    profileId: "p",
    seriesId: "ser-1",
    season: 2,
  });
  expect(mocks.invoke).toHaveBeenCalledWith("get_series_resume", {
    profileId: "p",
    seriesId: "ser-1",
  });
  expect(JSON.stringify(mocks.invoke.mock.calls)).not.toContain("FR Dark");
  expect(titleArtSubject(seriesToItem(s))).toEqual({ kind: "series", seriesId: "ser-1" });
  expect(seriesKey(s)).toBe("series-ser-1");
});

it("does not query a series the catalog compiler hasn't bound yet", async () => {
  const s = series(null);
  const { result } = renderHook(() => useSeriesResume("p", s), { wrapper: wrapper() });
  await waitFor(() => expect(result.current.fetchStatus).toBe("idle"));
  expect(mocks.invoke).not.toHaveBeenCalled();
  expect(titleArtSubject(seriesToItem(s))).toBeNull();
});
