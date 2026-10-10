import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type { CaptionCue } from "@/types/generated/CaptionCue";
import type { SourceCaptionsState } from "./useSourceCaptions";
import { useTranslatedCaptions } from "./useTranslatedCaptions";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);
const source: SourceCaptionsState = {
  sessionId: "session",
  transcriptId: 7,
  status: "streaming",
  language: "en",
  modelPct: null,
  error: null,
  downloadModel: vi.fn(),
  retry: vi.fn(),
  cues: Array.from({ length: 12 }, (_, i) => ({
    segmentId: i + 1,
    start: i,
    end: i + 1,
    text: `line ${i}`,
  })),
};
const cue: CaptionCue = { segmentId: 1, start: 0, end: 1, text: "Bonjour" };
const playhead = () => 0;

beforeEach(() => {
  vi.useFakeTimers();
  mockInvoke.mockReset();
});
afterEach(() => vi.useRealTimers());

describe("committed caption translation", () => {
  it("submits stored IDs in batches of eight, never frontend text", async () => {
    mockInvoke.mockImplementation(async (_command, args) => {
      const ids = (args as { segmentIds: number[] }).segmentIds;
      return ids.map((id) => ({ segmentId: id, start: id - 1, end: id, text: `translated ${id}` }));
    });
    const { result } = renderHook(() => useTranslatedCaptions("p", source, true, "fr", playhead));
    await act(async () => {});
    expect(mockInvoke).toHaveBeenCalledWith("translate_caption_segments", {
      profileId: "p",
      sessionId: "session",
      transcriptId: 7,
      targetLang: "fr",
      segmentIds: [1, 2, 3, 4, 5, 6, 7, 8],
    });
    expect(result.current.cues).toHaveLength(8);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(result.current.cues).toHaveLength(12);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
  });

  it("clears old target text and discards an outstanding old-target response", async () => {
    let finish!: (cues: CaptionCue[]) => void;
    mockInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        })
    );
    mockInvoke.mockResolvedValueOnce([{ ...cue, text: "مرحبا" }]);
    const { result, rerender } = renderHook(
      ({ target }) => useTranslatedCaptions("p", source, true, target, playhead),
      {
        initialProps: { target: "fr" },
      }
    );
    rerender({ target: "ar" });
    expect(result.current.cues).toEqual([]);
    await act(async () => {});
    expect(result.current.cues[0].text).toBe("مرحبا");
    await act(async () => {
      finish([cue]);
    });
    expect(result.current.cues[0].text).toBe("مرحبا");
  });

  it("keeps a safe failure note and retries only when requested", async () => {
    mockInvoke.mockRejectedValueOnce({
      kind: "Internal",
      message: "Translation model unavailable; source captions remain available",
    });
    mockInvoke.mockResolvedValueOnce([cue]);
    const { result } = renderHook(() => useTranslatedCaptions("p", source, true, "fr", playhead));
    await act(async () => {});
    expect(result.current.failed).toBe(true);
    expect(result.current.note).toContain("source captions remain available");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    await act(async () => {
      result.current.retry();
    });
    expect(result.current.failed).toBe(false);
    expect(result.current.cues).toEqual([cue]);
  });

  it("retries a busy slot but ignores a response after captions are turned off", async () => {
    let finish!: (cues: CaptionCue[]) => void;
    mockInvoke.mockRejectedValueOnce({ kind: "Conflict", message: "busy" });
    mockInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        })
    );
    const { result, rerender } = renderHook(
      ({ active }) => useTranslatedCaptions("p", source, active, "fr", playhead),
      {
        initialProps: { active: true },
      }
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(mockInvoke).toHaveBeenCalledTimes(2);
    rerender({ active: false });
    await act(async () => {
      finish([cue]);
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(result.current.cues).toEqual([]);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
  });

  it("does not translate display-only or uncommitted cues", async () => {
    renderHook(() =>
      useTranslatedCaptions(
        "p",
        { ...source, cues: [{ start: 0, end: 1, text: "provisional" }] },
        true,
        "fr",
        playhead
      )
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
