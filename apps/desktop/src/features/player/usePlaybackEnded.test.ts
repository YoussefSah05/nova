import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import type { PlaybackEnded, PlaybackRecover, SeriesEpisode } from "@/types";
import { usePlaybackEnded, usePlaybackRecover } from "./usePlaybackEnded";

type Handler = (event: { payload: unknown }) => void;
const mocks = vi.hoisted(() => ({
  handlers: new Map<string, Handler>(),
  unlisten: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: Handler) => {
    mocks.handlers.set(name, handler);
    return mocks.unlisten;
  }),
}));

const next = { id: 7, season: 1, episode: 3 } as SeriesEpisode;
const ended = (payload: Partial<PlaybackEnded>) =>
  mocks.handlers.get("playback:ended")?.({
    payload: { token: "tok-1", nextEpisode: next, autoplay: true, ...payload },
  });
const recover = (payload: Partial<PlaybackRecover>) =>
  mocks.handlers.get("playback:recover")?.({
    payload: {
      token: "tok-1",
      mediaKind: "vod",
      mediaId: 12,
      attempt: 1,
      otherSource: true,
      reason: "failure",
      ...payload,
    },
  });

beforeEach(() => {
  mocks.handlers.clear();
  mocks.unlisten.mockReset();
});

it("opens the next episode it is handed, for its own session only", async () => {
  const openNext = vi.fn();
  const { unmount } = renderHook(() => usePlaybackEnded(true, "tok-1", openNext));
  await waitFor(() => expect(mocks.handlers.has("playback:ended")).toBe(true));

  ended({ token: "tok-old" });
  ended({ autoplay: false });
  ended({ nextEpisode: null });
  expect(openNext).not.toHaveBeenCalled();

  ended({});
  expect(openNext).toHaveBeenCalledTimes(1);
  expect(openNext).toHaveBeenCalledWith(next);
  unmount();
  expect(mocks.unlisten).toHaveBeenCalled();
});

it("handles a recovery announced before its open settled, and drops stale ones", async () => {
  const act = vi.fn();
  const { rerender } = renderHook(({ token }) => usePlaybackRecover(true, token, act), {
    initialProps: { token: null as string | null },
  });
  await waitFor(() => expect(mocks.handlers.has("playback:recover")).toBe(true));

  // Opening: the new session fails before the player knows its token.
  recover({ token: "tok-old", attempt: 9 });
  recover({ token: "tok-new", attempt: 2 });
  expect(act).not.toHaveBeenCalled();

  rerender({ token: "tok-new" });
  expect(act).toHaveBeenCalledTimes(1);
  expect(act).toHaveBeenCalledWith(expect.objectContaining({ token: "tok-new", attempt: 2 }));

  // A later switch never replays the stale event.
  rerender({ token: null });
  rerender({ token: "tok-old" });
  expect(act).toHaveBeenCalledTimes(1);

  recover({ token: "tok-old", attempt: 3 });
  expect(act).toHaveBeenCalledTimes(2);
});

it("does not listen when the native engine is off", () => {
  renderHook(() => usePlaybackEnded(false, "tok-1", vi.fn()));
  renderHook(() => usePlaybackRecover(false, "tok-1", vi.fn()));
  expect(mocks.handlers.size).toBe(0);
});
