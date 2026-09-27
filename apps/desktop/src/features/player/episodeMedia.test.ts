import { describe, expect, it } from "vitest";

import type { SeriesEpisode } from "@/types";
import { episodeCode, episodeLabel } from "./episodeMedia";

const ep = (patch: Partial<SeriesEpisode>): SeriesEpisode => ({
  id: 1,
  sourceId: 1,
  seriesTitle: "Show",
  category: null,
  season: 3,
  episode: 7,
  episodeTitle: null,
  logoUrl: null,
  streamUrl: "x",
  addedAt: null,
  numbering: "seasonEpisode",
  ...patch,
});

describe("episode labels", () => {
  it("shows only the numbering that is actually known", () => {
    expect(episodeCode(ep({}))).toBe("S03E07");
    expect(episodeCode(ep({}), false)).toBe("E07");
    expect(episodeCode(ep({ numbering: "absolute", season: 1, episode: 1234 }))).toBe("E1234");
    expect(episodeCode(ep({ numbering: "unknown", season: 1, episode: 1 }))).toBeNull();
  });

  it("never presents placeholder numbers as S01E01", () => {
    const unknown = ep({ numbering: "unknown", season: 1, episode: 1 });
    expect(episodeLabel(unknown)).toBe("Episode");
    expect(episodeLabel({ ...unknown, episodeTitle: "Pilot" })).toBe("Pilot");
    expect(episodeLabel(ep({ episodeTitle: "One Minute" }))).toBe("S03E07 — One Minute");
  });
});
