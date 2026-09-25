import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { useProfileSettings } from "./useProfileSettings";
import { useCaptionStyle } from "../player/useCaptionStyle";
import { DEFAULT_PROFILE_SETTINGS, DEFAULT_CAPTION_STYLE } from "@/types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

const cases = [
  {
    name: "profile settings",
    get: "get_profile_settings",
    set: "set_profile_settings",
    field: "settings",
    defaults: DEFAULT_PROFILE_SETTINGS,
    patch: { accentColor: "blue" },
    usePreference: (id: string) => {
      const s = useProfileSettings(id);
      return { value: s.settings, update: () => s.setSettings({ accentColor: "blue" }) };
    },
  },
  {
    name: "caption style",
    get: "get_caption_style",
    set: "set_caption_style",
    field: "style",
    defaults: DEFAULT_CAPTION_STYLE,
    patch: { fontScale: 1.5 },
    usePreference: (id: string) => {
      const s = useCaptionStyle(id);
      return { value: s.style, update: () => s.setStyle({ fontScale: 1.5 }) };
    },
  },
];

for (const c of cases)
  describe(c.name, () => {
    beforeEach(() => {
      vi.useFakeTimers();
      mockInvoke.mockReset();
      mockInvoke.mockImplementation(async () => c.defaults);
    });
    afterEach(() => vi.useRealTimers());

    it("flushes the pending edit when the panel closes", async () => {
      const { result, unmount } = renderHook(() => c.usePreference("first"));
      await act(async () => {});
      act(() => result.current.update());
      unmount();
      await act(async () => {});
      expect(mockInvoke).toHaveBeenCalledWith(c.set, {
        profileId: "first",
        [c.field]: { ...c.defaults, ...c.patch },
      });
    });

    it("keeps a pending edit attached to its original profile", async () => {
      const { result, rerender } = renderHook(({ id }) => c.usePreference(id), {
        initialProps: { id: "first" },
      });
      await act(async () => {});
      act(() => result.current.update());
      rerender({ id: "second" });
      await act(async () => {});
      await act(async () => {
        vi.advanceTimersByTime(400);
      });
      expect(mockInvoke).toHaveBeenCalledWith(c.set, {
        profileId: "first",
        [c.field]: { ...c.defaults, ...c.patch },
      });
      expect(result.current.value).toEqual(c.defaults);
    });

    it("preserves edits made before the saved value arrives", async () => {
      let resolve!: (value: unknown) => void;
      mockInvoke.mockImplementation((command) =>
        command === c.get
          ? new Promise((r) => {
              resolve = r;
            })
          : Promise.resolve()
      );
      const { result } = renderHook(() => c.usePreference("first"));
      act(() => result.current.update());
      await act(async () => {});
      await act(async () => {
        resolve(c.defaults);
      });
      expect(result.current.value).toEqual({ ...c.defaults, ...c.patch });
    });
  });

describe("concurrent settings panels", () => {
  it("merges edits with the latest persisted settings instead of a stale mount snapshot", async () => {
    vi.useFakeTimers();
    let saved = { ...DEFAULT_PROFILE_SETTINGS };
    mockInvoke.mockImplementation(async (command, args) => {
      if (command === "get_profile_settings") return { ...saved };
      if (command === "set_profile_settings") saved = (args as { settings: typeof saved }).settings;
    });
    const first = renderHook(() => useProfileSettings("concurrent"));
    const second = renderHook(() => useProfileSettings("concurrent"));
    await act(async () => {});
    act(() => first.result.current.setSettings({ accentColor: "blue" }));
    await act(async () => {
      vi.advanceTimersByTime(400);
    });
    act(() => second.result.current.setSettings({ autoplayNext: false }));
    await act(async () => {
      vi.advanceTimersByTime(400);
    });
    expect(saved.accentColor).toBe("blue");
    expect(saved.autoplayNext).toBe(false);
    first.unmount();
    second.unmount();
    vi.useRealTimers();
  });
});
