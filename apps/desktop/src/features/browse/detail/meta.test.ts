import { describe, expect, it } from "vitest";

import type { ContinueWatchingItem, TitleFactsView } from "@/types";
import { formatRuntime, metaLine, seriesPrimaryAction, yearSpan } from "./meta";

const facts = (over: Partial<TitleFactsView>): TitleFactsView => ({
  tagline: null,
  runtimeMin: null,
  rating: null,
  ratingSource: null,
  certification: null,
  trailerUrl: null,
  cast: [],
  directors: [],
  studios: [],
  status: null,
  firstYear: null,
  lastYear: null,
  seasons: [],
  numberOfEpisodes: null,
  nextEpisode: null,
  collection: null,
  plot: null,
  genre: null,
  ...over,
});

describe("detail meta", () => {
  it("formats runtimes", () => {
    expect(formatRuntime(148)).toBe("2h 28m");
    expect(formatRuntime(45)).toBe("45m");
    expect(formatRuntime(120)).toBe("2h");
  });

  it("spans years by status", () => {
    expect(yearSpan(facts({ firstYear: 2011, lastYear: 2019, status: "Ended" }))).toBe("2011–2019");
    expect(yearSpan(facts({ firstYear: 2019, lastYear: 2024, status: "Returning Series" }))).toBe(
      "2019–"
    );
    expect(yearSpan(undefined, 1999)).toBe("1999");
    expect(yearSpan(undefined)).toBeNull();
  });

  it("builds the movie and series lines", () => {
    const movie = metaLine(
      facts({ firstYear: 2010, runtimeMin: 148, rating: 8.37, certification: "PG-13" }),
      { kind: "movie", genre: "Sci-Fi" }
    );
    expect(movie).toEqual({ certification: "PG-13", parts: ["2010", "2h 28m", "★ 8.4", "Sci-Fi"] });
    const series = metaLine(
      facts({
        seasons: [
          { number: 0, name: "Specials", episodeCount: 2 },
          { number: 1, name: "S1", episodeCount: 10 },
          { number: 2, name: "S2", episodeCount: 8 },
        ],
      }),
      { kind: "series", seasonCount: 5 }
    );
    expect(series.parts).toEqual(["2 seasons"]);
    expect(metaLine(undefined, { kind: "series", seasonCount: 1 }).parts).toEqual(["1 season"]);
  });

  it("picks the series action", () => {
    const resume: ContinueWatchingItem = {
      mediaKind: "episode",
      mediaId: 42,
      sourceId: 1,
      title: "Dark",
      subtitle: null,
      posterUrl: null,
      positionSec: 600,
      durationSec: 2400,
      watchedAt: 0,
      entityId: null,
      seriesTitle: "Dark",
      season: 2,
      episode: 3,
      numbering: "seasonEpisode",
      episodeTitle: null,
      upNext: false,
    };
    const first = { id: 1, season: 1, episode: 1, numbering: "seasonEpisode" as const };
    expect(seriesPrimaryAction(resume, first)).toEqual({
      label: "Resume S02E03",
      episodeId: 42,
      episodeCode: "S02E03",
    });
    expect(seriesPrimaryAction({ ...resume, upNext: true }, first)?.label).toBe(
      "Next episode · S02E03"
    );
    expect(seriesPrimaryAction(null, first)).toEqual({
      label: "Play S01E01",
      episodeId: 1,
      episodeCode: "S01E01",
    });
    expect(seriesPrimaryAction(null, undefined)).toBeNull();
  });
});
