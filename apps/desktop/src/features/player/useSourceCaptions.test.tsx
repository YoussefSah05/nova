import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useSourceCaptions } from "./useSourceCaptions";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
beforeEach(() => invoke.mockReset());
afterEach(cleanup);
function useTestCaptions(active = true) {
  return useSourceCaptions(
    "p",
    { kind: "vod", id: 5 },
    active,
    () => 125,
    () => 3600,
    { index: 1, lang: "ar" }
  );
}
it("starts at the established resume position using the audible source ordinal", async () => {
  invoke.mockResolvedValue({ installed: true, modelId: "whisper" });
  const { rerender } = renderHook(({ active }) => useTestCaptions(active), {
    initialProps: { active: false },
  });
  expect(invoke).not.toHaveBeenCalled();
  rerender({ active: true });
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith(
      "start_caption_session",
      expect.objectContaining({
        mediaId: 5,
        fromSec: 125,
        audioTrackIndex: 1,
        trackLang: "ar",
        durationSec: 3600,
      })
    )
  );
});
it("explains a disabled model and allows Retry after enabling without downloading", async () => {
  let enabled = false;
  invoke.mockImplementation(async (command) => {
    if (command === "get_model_status") return { installed: enabled, modelId: "whisper" };
    if (command === "list_model_controls") return [{ modelId: "whisper", enabled: false }];
  });
  const { result } = renderHook(() => useTestCaptions());
  await waitFor(() => expect(result.current.status).toBe("failed"));
  expect(result.current.error).toContain("Enable it in Settings");
  enabled = true;
  act(() => result.current.retry());
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("start_caption_session", expect.anything())
  );
  expect(result.current.error).toBeNull();
  expect(invoke.mock.calls.some(([command]) => command === "download_whisper_model")).toBe(false);
});
it("keeps missing model downloads explicit", async () => {
  invoke.mockImplementation(async (command) => {
    if (command === "get_model_status") return { installed: false, modelId: "whisper" };
    if (command === "list_model_controls") return [{ modelId: "whisper", enabled: true }];
  });
  const { result } = renderHook(() => useTestCaptions());
  await waitFor(() => expect(result.current.status).toBe("needsModel"));
  expect(invoke.mock.calls.some(([command]) => command === "download_whisper_model")).toBe(false);
  act(() => result.current.downloadModel());
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("download_whisper_model"));
});
