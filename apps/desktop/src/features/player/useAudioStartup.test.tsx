import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAudioStartup } from "./useAudioStartup";
import { initialCaptionMode } from "./playbackLaunch";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const defaults = {
  mediaKind: "vod",
  mediaId: 1,
  token: "native",
  ready: true,
  tracks: [
    { id: 4, lang: "fre" },
    { id: 9, lang: "ara" },
  ],
  selectedId: 4,
  profileLoaded: true,
  preferredLanguage: "fr",
};
beforeEach(() => invoke.mockReset().mockResolvedValue(undefined));
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("selects the requested source ordinal and waits for the actual native acknowledgement", async () => {
  const requested = { mediaKind: "vod" as const, mediaId: 1, index: 1, language: "ar" };
  const { result, rerender } = renderHook((props) => useAudioStartup(props), {
    initialProps: { ...defaults, ready: false, requested },
  });
  expect(invoke).not.toHaveBeenCalled();
  rerender({ ...defaults, requested });
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("mpv_set_audio_track", { token: "native", id: "9" })
  );
  expect(result.current.settled).toBe(false);
  rerender({ ...defaults, selectedId: 9, requested });
  await waitFor(() => expect(result.current.settled).toBe(true));
  rerender({ ...defaults, selectedId: 9, ready: false, requested });
  expect(result.current.settled).toBe(true);
  expect(invoke).toHaveBeenCalledTimes(1);
  // Startup acknowledgement must not pin captions to this track forever.
  rerender({ ...defaults, selectedId: 4, requested });
  expect(result.current.settled).toBe(true);
  expect(invoke).toHaveBeenCalledTimes(1);
});

it("rejects another source's ordinal visibly and keeps automatic audio", async () => {
  const { result } = renderHook(() =>
    useAudioStartup({
      ...defaults,
      requested: { mediaKind: "vod", mediaId: 2, index: 1, language: "ar" },
    })
  );
  await waitFor(() => expect(result.current.settled).toBe(true));
  expect(result.current.note).toContain("no longer available");
  expect(invoke).not.toHaveBeenCalled();
});

it("bounds a missing audio acknowledgement without blocking video", async () => {
  vi.useFakeTimers();
  const { result } = renderHook(() =>
    useAudioStartup({
      ...defaults,
      requested: { mediaKind: "vod", mediaId: 1, index: 1, language: "ar" },
    })
  );
  await act(async () => {});
  expect(result.current.settled).toBe(false);
  await act(async () => vi.advanceTimersByTime(3000));
  expect(result.current.settled).toBe(true);
  expect(result.current.note).toContain("not confirmed");
});

it("falls back on a rejected command and gates a new native session independently", async () => {
  invoke.mockRejectedValueOnce(new Error("gone"));
  const { result, rerender } = renderHook((props) => useAudioStartup(props), {
    initialProps: { ...defaults, preferredLanguage: "ar" },
  });
  await waitFor(() => expect(result.current.note).toContain("could not be selected"));
  expect(result.current.settled).toBe(true);
  rerender({ ...defaults, token: "replacement", ready: false, preferredLanguage: "ar" });
  expect(result.current.settled).toBe(false);
});

it("explicit normal/source/translated modes override the profile only for this launch", () => {
  expect(initialCaptionMode({ captionMode: "off" }, "source")).toBe("off");
  expect(initialCaptionMode({ captionMode: "source" }, "off")).toBe("source");
  expect(initialCaptionMode({ captionMode: "translation", translationTarget: "ar" }, "off")).toBe(
    "translation"
  );
  expect(initialCaptionMode(undefined, "source")).toBe("source");
});

it("does not transcribe a guessed track when the engine has no selected audio", async () => {
  const { result } = renderHook(() =>
    useAudioStartup({ ...defaults, selectedId: null, preferredLanguage: null })
  );
  await waitFor(() => expect(result.current.settled).toBe(true));
  expect(result.current.hasAudio).toBe(false);
  expect(invoke).not.toHaveBeenCalled();
});
