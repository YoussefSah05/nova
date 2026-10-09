import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen, type Event } from "@tauri-apps/api/event";
import { sourceCaptionNote, useSourceCaptions } from "./useSourceCaptions";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
const mockInvoke = vi.mocked(invoke);
const media = { kind: "vod", id: 1 };
const time = () => 0;
describe("source caption session", () => {
  const download = "download_whisper_model";
  const start = "start_caption_session";
  const event = "transcription:cues";
  function useCaptions(active: boolean) {
    return useSourceCaptions("fixture", media, active, time, time, null);
  }
  beforeEach(() => mockInvoke.mockReset());
  it("does not restart captions when a model download finishes after captions are turned off", async () => {
    let finish!: () => void;
    mockInvoke.mockImplementation((command) =>
      command === download
        ? new Promise<void>((r) => {
            finish = r;
          })
        : Promise.resolve({ installed: false })
    );
    const { result, rerender } = renderHook(({ active }) => useCaptions(active), {
      initialProps: { active: true },
    });
    await act(async () => {});
    act(() => result.current.downloadModel());
    rerender({ active: false });
    await act(async () => {
      finish();
    });
    expect(mockInvoke.mock.calls.some(([command]) => command === start)).toBe(false);
  });
  it("subscribes before starting and ignores cues from a replaced session", async () => {
    const handlers = new Map<string, (event: Event<unknown>) => void>();
    vi.mocked(listen).mockImplementation(async (name, handler) => {
      handlers.set(name, handler as (event: Event<unknown>) => void);
      return () => {};
    });
    mockInvoke.mockImplementation(async (command) => {
      if (command === start) expect(handlers.has(event)).toBe(true);
      return { installed: true };
    });
    const { result, rerender } = renderHook(({ active }) => useCaptions(active), {
      initialProps: { active: true },
    });
    await act(async () => {});
    const first = mockInvoke.mock.calls.find(([command]) => command === start)?.[1] as {
      sessionId: string;
    };
    rerender({ active: false });
    await act(async () => {});
    rerender({ active: true });
    await act(async () => {});
    const emit = (sessionId: string) =>
      handlers.get(event)?.({
        event,
        id: 1,
        payload: {
          sessionId,
          mediaKind: "vod",
          mediaId: 1,
          language: "en",
          transcriptId: 7,
          cues: [{ segmentId: 1, start: 0, end: 1, text: "fixture" }],
        },
      });
    act(() => emit(first.sessionId));
    expect(result.current.cues).toEqual([]);
    const last = mockInvoke.mock.calls.filter(([command]) => command === start).at(-1)?.[1] as {
      sessionId: string;
    };
    act(() => emit(last.sessionId));
    expect(result.current.cues).toHaveLength(1);
    expect(result.current.transcriptId).toBe(7);
    expect(result.current.sessionId).toBe(last.sessionId);
  });
  it("hides old committed dialogue on the first render of a media or track change", async () => {
    let onCues: ((event: Event<unknown>) => void) | undefined;
    vi.mocked(listen).mockImplementation(async (name, handler) => {
      if (name === event) onCues = handler as (event: Event<unknown>) => void;
      return () => {};
    });
    mockInvoke.mockResolvedValue({ installed: true });
    const seen: Array<{ count: number; transcript: number | null }> = [];
    const { rerender } = renderHook(
      ({ id, track }) => {
        const state = useSourceCaptions("fixture", { kind: "vod", id }, true, time, time, {
          index: track,
          lang: "en",
        });
        seen.push({ count: state.cues.length, transcript: state.transcriptId });
        return state;
      },
      { initialProps: { id: 1, track: 0 } }
    );
    await act(async () => {});
    const session = mockInvoke.mock.calls.filter(([command]) => command === start).at(-1)?.[1] as {
      sessionId: string;
    };
    act(() =>
      onCues?.({
        event,
        id: 1,
        payload: {
          sessionId: session.sessionId,
          mediaKind: "vod",
          mediaId: 1,
          transcriptId: 7,
          language: "en",
          cues: [{ segmentId: 1, start: 0, end: 1, text: "old dialogue" }],
        },
      })
    );
    expect(seen.at(-1)?.count).toBe(1);
    const before = seen.length;
    rerender({ id: 2, track: 1 });
    expect(seen[before]).toEqual({ count: 0, transcript: null });
    await act(async () => {});
  });
});

it("reports current-stage time and first-batch data readiness, clearing them on stop", async () => {
  const handlers = new Map<string, (event: Event<unknown>) => void>();
  vi.mocked(listen).mockImplementation(async (name, handler) => {
    handlers.set(name, handler as (event: Event<unknown>) => void);
    return () => {};
  });
  mockInvoke.mockResolvedValue({ installed: true });
  const now = vi.spyOn(performance, "now").mockReturnValue(1000);
  const { result, rerender, unmount } = renderHook(
    ({ active }) => useSourceCaptions("fixture", media, active, time, time, null),
    { initialProps: { active: true } }
  );
  try {
    await act(async () => {});
    const session = mockInvoke.mock.calls
      .filter(([command]) => command === "start_caption_session")
      .at(-1)?.[1] as { sessionId: string };
    const emit = (event: string, extra: Record<string, unknown>) =>
      handlers.get(event)?.({
        event,
        id: 1,
        payload: { sessionId: session.sessionId, mediaKind: "vod", mediaId: 1, ...extra },
      });
    now.mockReturnValue(6000);
    act(() =>
      emit("transcription:status", { state: "streaming", phase: "Recognizing and aligning speech" })
    );
    expect(sourceCaptionNote(result.current)).toBe("Recognizing and aligning speech · 0s");
    act(() =>
      emit("transcription:cues", {
        language: "en",
        transcriptId: 7,
        cues: [{ segmentId: 1, start: 0, end: 1, text: "fixture" }],
      })
    );
    act(() => emit("transcription:status", { state: "streaming", phase: null }));
    expect(sourceCaptionNote(result.current)).toBe("Caption data ready · first batch 5.0s");
    now.mockReturnValue(601000);
    act(() => emit("transcription:status", { state: "streaming", phase: "Extracting audio" }));
    expect(sourceCaptionNote(result.current)).toBe("Extracting audio · 0s");
    rerender({ active: false });
    expect(result.current.phase).toBeNull();
    expect(result.current.firstCuesSec).toBeNull();
  } finally {
    now.mockRestore();
    unmount();
  }
});
