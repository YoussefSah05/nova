import { describe, expect, it } from "vitest";

import type { SeriesSummary } from "@/types";
import { localDate, progressTicks, showMeta, statusLabel, watchedLine, weekOf } from "./shows";

function show(extra: Partial<SeriesSummary> = {}): SeriesSummary {
  return {
    seriesTitle: "Dark",
    sourceId: 1,
    category: "Drama",
    episodeCount: 26,
    seasonCount: 3,
    posterUrl: null,
    isFavorite: false,
    isInWatchlist: false,
    addedAt: null,
    canonicalTitle: null,
    backdropUrl: null,
    overview: null,
    displayTitle: "Dark",
    entityId: "ser-1",
    ...extra,
  };
}

describe("show descriptions", () => {
  it("reads status only from TMDB's status or schedule", () => {
    expect(statusLabel(show({ status: "Ended" }))).toBe("Ended");
    expect(statusLabel(show({ status: "Canceled" }))).toBe("Cancelled");
    expect(statusLabel(show({ status: "Returning Series" }))).toBe("Returning");
    expect(
      statusLabel(
        show({ nextEpisode: { season: 2, episode: 1, name: null, airDate: "2026-10-01" } })
      )
    ).toBe("Returning");
    expect(statusLabel(show())).toBeNull();
  });

  it("describes seasons, years and genres", () => {
    expect(
      showMeta(
        show({
          status: "Ended",
          firstYear: 2017,
          lastYear: 2020,
          genres: ["Mystery", "Drama", "Sci-Fi"],
        })
      )
    ).toBe("3 seasons · 2017–2020 · Mystery, Drama");
    expect(showMeta(show({ seasonCount: 1, status: "Returning Series", firstYear: 2024 }))).toBe(
      "1 season · 2024– · Drama"
    );
  });

  it("shows progress as ticks until there are too many episodes", () => {
    expect(watchedLine(show({ watchedCount: 14 }))).toBe("14 of 26 watched");
    expect(watchedLine(show({ watchedCount: 0 }))).toBeNull();
    const ticks = progressTicks(show({ episodeCount: 4, watchedCount: 1 }));
    expect(ticks).toEqual([true, false, false, false]);
    expect(progressTicks(show({ episodeCount: 120, watchedCount: 3 }))).toBeNull();
  });

  it("buckets next episodes into the viewer's local week", () => {
    const today = new Date(2026, 8, 29, 23, 30);
    const airing = show({
      displayTitle: "Slow Horses",
      nextEpisode: { season: 4, episode: 4, name: null, airDate: "2026-10-01" },
    });
    const week = weekOf(today, [airing]);
    expect(week.map((d) => d.date)).toEqual([
      "2026-09-29",
      "2026-09-30",
      "2026-10-01",
      "2026-10-02",
      "2026-10-03",
      "2026-10-04",
      "2026-10-05",
    ]);
    expect(week[0].label).toBe("Today");
    expect(week[1].label).toBe("Tomorrow");
    expect(week[2].shows).toEqual([airing]);
    expect(localDate(new Date(2026, 0, 5))).toBe("2026-01-05");
  });
});
