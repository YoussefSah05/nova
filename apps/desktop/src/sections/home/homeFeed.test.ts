import { describe, expect, it } from "vitest";

import type { ContinueWatchingItem } from "@/types";
import { clockTime, greeting, homeFeed, progressPct, timeLeft } from "./homeFeed";

function cw(
  kind: string,
  id: number,
  watchedAt: number,
  extra: Partial<ContinueWatchingItem> = {}
) {
  return {
    mediaKind: kind,
    mediaId: id,
    sourceId: 1,
    title: `${kind}-${id}`,
    subtitle: null,
    posterUrl: null,
    positionSec: 600,
    durationSec: 3600,
    watchedAt,
    entityId: kind === "vod" ? `mov-${id}` : null,
    seriesTitle: null,
    season: null,
    episode: null,
    numbering: null,
    episodeTitle: null,
    upNext: false,
    ...extra,
  } satisfies ContinueWatchingItem;
}

describe("home feed", () => {
  it("puts the most recent movie or episode in the hero and splits the rest", () => {
    const movies = [cw("vod", 1, 10), cw("vod", 2, 40)];
    const series = [cw("episode", 3, 30), cw("episode", 4, 20)];
    const live = [cw("live", 5, 50), cw("live", 6, 60)];
    const feed = homeFeed(movies, series, live);
    expect(feed.hero?.mediaId).toBe(2);
    expect(feed.alsoInProgress.map((i) => i.mediaId)).toEqual([3, 4, 1]);
    expect(feed.upNext.map((i) => i.mediaId)).toEqual([3, 4]);
    expect(feed.recentChannels.map((i) => i.mediaId)).toEqual([6, 5]);
    expect(feed.seedMovie?.mediaId).toBe(2);
  });

  it("never repeats the hero episode under Up next", () => {
    const feed = homeFeed([], [cw("episode", 3, 30), cw("episode", 4, 20)], []);
    expect(feed.hero?.mediaId).toBe(3);
    expect(feed.upNext.map((i) => i.mediaId)).toEqual([4]);
  });

  it("handles an empty history", () => {
    const feed = homeFeed();
    expect(feed.hero).toBeNull();
    expect(feed.seedMovie).toBeNull();
    expect(feed.upNext).toEqual([]);
  });

  it("formats progress and time", () => {
    const item = cw("vod", 1, 1, { positionSec: 5650, durationSec: 9960 });
    expect(progressPct(item)).toBe(57);
    expect(timeLeft(item)).toBe("1h 12m left");
    expect(timeLeft(cw("vod", 1, 1, { positionSec: 0 }))).toBeNull();
    expect(clockTime(5650)).toBe("1:34:10");
    expect(clockTime(59)).toBe("0:59");
    expect(greeting(new Date(2026, 8, 29, 21, 40))).toBe("Good evening");
    expect(greeting(new Date(2026, 8, 29, 8, 0))).toBe("Good morning");
  });
});
