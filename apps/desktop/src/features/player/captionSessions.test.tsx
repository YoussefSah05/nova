import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen, type Event } from "@tauri-apps/api/event";
import { useSourceCaptions } from "./useSourceCaptions";
import { useTranslatedCaptions } from "./useTranslatedCaptions";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
const mockInvoke = vi.mocked(invoke);
const media = { kind: "vod", id: 1 };
const time = () => 0;
for (const kind of ["source", "translation"] as const)
  describe(`${kind} session`, () => {
    const download = kind === "source" ? "download_whisper_model" : "download_nllb_model";
    const start = kind === "source" ? "start_caption_session" : "start_translation_session";
    const useCaptions =
      kind === "source"
        ? function useSource(active: boolean) {
            return useSourceCaptions("fixture", media, active, time, time, null);
          }
        : function useTranslation(active: boolean) {
            return useTranslatedCaptions("fixture", media, active, time, time, null, "fr");
          };
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
        if (command === start)
          expect(handlers.has(kind === "source" ? "transcription:cues" : "translation:cues")).toBe(
            true
          );
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
      const event = kind === "source" ? "transcription:cues" : "translation:cues";
      const emit = (sessionId: string) =>
        handlers.get(event)?.({
          event,
          id: 1,
          payload: {
            sessionId,
            mediaKind: "vod",
            mediaId: 1,
            language: "en",
            cues: [{ start: 0, end: 1, text: "fixture" }],
          },
        });
      act(() => emit(first.sessionId));
      expect(result.current.cues).toEqual([]);
      const last = mockInvoke.mock.calls.filter(([command]) => command === start).at(-1)?.[1] as {
        sessionId: string;
      };
      act(() => emit(last.sessionId));
      expect(result.current.cues).toHaveLength(1);
    });
  });
