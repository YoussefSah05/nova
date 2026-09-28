import { beforeEach, expect, it, vi } from "vitest";

import type { SeriesSummary } from "@/types";
import { seriesToItem } from "./queries";
import { invokeFavorite, invokeWatchlist } from "./toggles";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

const series = (entityId: string | null): SeriesSummary => ({
  seriesTitle: "FR Dark",
  sourceId: 3,
  category: null,
  episodeCount: 1,
  seasonCount: 1,
  posterUrl: null,
  isFavorite: false,
  isInWatchlist: false,
  addedAt: null,
  canonicalTitle: null,
  backdropUrl: null,
  overview: null,
  displayTitle: "FR Dark",
  entityId,
});

beforeEach(() => mocks.invoke.mockReset().mockResolvedValue(undefined));

it("toggles a series through its canonical entity", async () => {
  const item = seriesToItem(series("ser-1"));
  await invokeFavorite(item, "p", true);
  await invokeWatchlist(item, "p", false);
  expect(mocks.invoke).toHaveBeenCalledWith("set_series_favorite", {
    profileId: "p",
    seriesId: "ser-1",
    isFavorite: true,
  });
  expect(mocks.invoke).toHaveBeenCalledWith("set_series_in_watchlist", {
    profileId: "p",
    seriesId: "ser-1",
    inWatchlist: false,
  });
  expect(JSON.stringify(mocks.invoke.mock.calls)).not.toContain("FR Dark");
});

it("refuses an unbound series with a clear message instead of sending null", async () => {
  const item = seriesToItem(series(null));
  await expect(invokeFavorite(item, "p", true)).rejects.toThrow(/still being prepared/);
  await expect(invokeWatchlist(item, "p", true)).rejects.toThrow(/still being prepared/);
  expect(mocks.invoke).not.toHaveBeenCalled();
});
