import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import type { PlaybackEnded, SeriesEpisode } from "@/types";
import { usePlaybackEnded } from "./usePlaybackEnded";

const mocks = vi.hoisted(() => ({
  handler: null as ((event: { payload: PlaybackEnded }) => void) | null,
  unlisten: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_name: string, handler: (event: { payload: PlaybackEnded }) => void) => {
    mocks.handler = handler;
    return mocks.unlisten;
  }),
}));

const next = { id: 7, season: 1, episode: 3 } as SeriesEpisode;
const ended = (payload: Partial<PlaybackEnded>) =>
  mocks.handler?.({ payload: { token: "tok-1", nextEpisode: next, autoplay: true, ...payload } });

beforeEach(() => {
  mocks.handler = null;
  mocks.unlisten.mockReset();
});

it("opens the next episode it is handed, for its own session only", async () => {
  const openNext = vi.fn();
  const tokenRef = { current: "tok-1" };
  const openingRef = { current: false };
  const { unmount } = renderHook(() => usePlaybackEnded(true, tokenRef, openingRef, openNext));
  await waitFor(() => expect(mocks.handler).not.toBeNull());

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

it("does not listen when the native engine is off", () => {
  renderHook(() => usePlaybackEnded(false, { current: "tok-1" }, { current: false }, vi.fn()));
  expect(mocks.handler).toBeNull();
});
