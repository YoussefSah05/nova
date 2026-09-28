import { describe, expect, it } from "vitest";

import type { ContinueWatchingItem } from "@/types";
import { continueEpisodeLabel, continueSubtitle } from "./continueWatching";

const base: ContinueWatchingItem = {
  mediaKind: "episode",
  mediaId: 1,
  sourceId: 1,
  title: "Dark",
  subtitle: null,
  posterUrl: null,
  positionSec: 600,
  durationSec: 2400,
  watchedAt: 0,
  entityId: null,
  seriesTitle: "Dark",
  season: 1,
  episode: 2,
  numbering: "seasonEpisode",
  episodeTitle: "Lies",
  upNext: false,
};

describe("continue watching text", () => {
  it("shows the episode and the time left when resuming", () => {
    expect(continueSubtitle(base)).toBe("S01E02 · 30 min left");
    expect(continueEpisodeLabel(base)).toBe("S01E02 — Lies");
  });

  it("announces the next episode", () => {
    expect(continueSubtitle({ ...base, upNext: true, positionSec: 0, season: 2, episode: 1 })).toBe(
      "Up next · S02E01"
    );
  });

  it("never shows placeholder numbers", () => {
    const unknown = { ...base, numbering: "unknown" as const };
    expect(continueSubtitle(unknown)).toBe("Lies · 30 min left");
    expect(continueEpisodeLabel(unknown)).toBe("Lies");
  });

  it("gives movies the time left and live nothing", () => {
    const movie = { ...base, mediaKind: "vod", season: null, episode: null, numbering: null };
    expect(continueSubtitle(movie)).toBe("30 min left");
    expect(continueSubtitle({ ...movie, positionSec: 0 })).toBeNull();
    expect(continueSubtitle({ ...movie, mediaKind: "live" })).toBeNull();
  });
});
