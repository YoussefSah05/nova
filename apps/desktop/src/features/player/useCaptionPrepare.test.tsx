import { renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import { useCaptionPrepare, type CaptionPrepareTarget } from "./useCaptionPrepare";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const film: CaptionPrepareTarget = {
  profileId: "p",
  mediaKind: "vod",
  mediaId: 7,
  fromSec: 61.8,
};

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
});

const calls = (command: string) => invoke.mock.calls.filter(([c]) => c === command);

it("prepares nothing without a sign the user wants captions", () => {
  renderHook(() => useCaptionPrepare(film, undefined, false));
  expect(invoke).not.toHaveBeenCalled();
});

it("prepares from the resume point and cancels when the page closes", () => {
  const { unmount } = renderHook(() => useCaptionPrepare(film, { index: 1, language: "fr" }, true));
  expect(calls("prepare_captions")).toHaveLength(1);
  const args = calls("prepare_captions")[0][1];
  expect(args).toMatchObject({
    profileId: "p",
    mediaKind: "vod",
    mediaId: 7,
    fromSec: 61,
    audioTrackIndex: 1,
    trackLang: "fr",
  });
  unmount();
  expect(calls("cancel_caption_prepare")).toEqual([
    ["cancel_caption_prepare", { sessionId: args.sessionId }],
  ]);
});

it("keeps the work when the menu closes so the player can adopt it", () => {
  const { rerender } = renderHook(({ intent }) => useCaptionPrepare(film, undefined, intent), {
    initialProps: { intent: true },
  });
  rerender({ intent: false });
  expect(calls("prepare_captions")).toHaveLength(1);
  expect(calls("cancel_caption_prepare")).toHaveLength(0);
});

it("restarts for another version or audio track", () => {
  const { rerender } = renderHook(({ target, track }) => useCaptionPrepare(target, track, true), {
    initialProps: {
      target: film,
      track: undefined as { index: number; language: null } | undefined,
    },
  });
  rerender({ target: film, track: { index: 2, language: null } });
  rerender({ target: { ...film, mediaId: 8 }, track: { index: 2, language: null } });
  expect(calls("prepare_captions")).toHaveLength(3);
  expect(calls("cancel_caption_prepare")).toHaveLength(2);
});
