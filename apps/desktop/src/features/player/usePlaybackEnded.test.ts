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
      ...payload,
    },
  });

beforeEach(() => {
  mocks.handlers.clear();
  mocks.unlisten.mockReset();
});

it("opens the next episode it is handed, for its own session only", async () => {
  const openNext = vi.fn();
  const tokenRef = { current: "tok-1" };
  const openingRef = { current: false };
  const { unmount } = renderHook(() => usePlaybackEnded(true, tokenRef, openingRef, openNext));
  await waitFor(() => expect(mocks.handlers.has("playback:ended")).toBe(true));

  ended({ token: "tok-old" });
  ended({ autoplay: false });
  ended({ nextEpisode: null });
  // The user is already switching to something else: their choice wins.
  openingRef.current = true;
  ended({});
  openingRef.current = false;
  expect(openNext).not.toHaveBeenCalled();

  ended({});
  expect(openNext).toHaveBeenCalledTimes(1);
  expect(openNext).toHaveBeenCalledWith(next);
  unmount();
  expect(mocks.unlisten).toHaveBeenCalled();
});

it("acts on a recovery decision for its own session only", async () => {
  const act = vi.fn();
  const tokenRef = { current: "tok-1" };
  const openingRef = { current: false };
  renderHook(() => usePlaybackRecover(true, tokenRef, openingRef, act));
  await waitFor(() => expect(mocks.handlers.has("playback:recover")).toBe(true));

  recover({ token: "tok-old" });
  openingRef.current = true;
  recover({});
  openingRef.current = false;
  expect(act).not.toHaveBeenCalled();

  recover({ attempt: 2 });
  expect(act).toHaveBeenCalledTimes(1);
  expect(act).toHaveBeenCalledWith(expect.objectContaining({ mediaId: 12, attempt: 2 }));
});

it("does not listen when the native engine is off", () => {
  renderHook(() => usePlaybackEnded(false, { current: "tok-1" }, { current: false }, vi.fn()));
  renderHook(() => usePlaybackRecover(false, { current: "tok-1" }, { current: false }, vi.fn()));
  expect(mocks.handlers.size).toBe(0);
});
