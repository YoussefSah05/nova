import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { closeNativePlayback, openNativePlayback, type NativeOpenResult } from "./nativePlayback";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);
const options = { profileId: "fixture", mediaKind: "vod", mediaId: 1, title: "fixture" };

describe("native playback lifetime", () => {
  it("passes an explicit moment to the coordinator instead of issuing a separate seek", async () => {
    mockInvoke.mockReset();
    mockInvoke.mockResolvedValue({ token: "moment", embedded: true });
    await openNativePlayback({ ...options, startSec: 42 }, () => false);
    expect(mockInvoke.mock.calls).toEqual([["mpv_open_stream", { ...options, startSec: 42 }]]);
  });

  it("closes a late open before starting the next requested media", async () => {
    let finish!: (value: NativeOpenResult) => void;
    let cancelled = false;
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command, args) => {
      if (command === "mpv_open_stream" && (args as typeof options).mediaId === 1) {
        return new Promise((resolve) => {
          finish = resolve;
        });
      }
      return Promise.resolve(
        command === "mpv_open_stream" ? { token: "new", embedded: true } : undefined
      );
    });
    const first = openNativePlayback(options, () => cancelled);
    await Promise.resolve();
    cancelled = true;
    const next = openNativePlayback({ ...options, mediaId: 2 }, () => false);
    finish({ token: "old", embedded: true });
    expect(await first).toBeNull();
    expect(await next).toEqual({ token: "new", embedded: true });
    expect(mockInvoke.mock.calls.map(([command]) => command)).toEqual([
      "mpv_open_stream",
      "mpv_close",
      "mpv_open_stream",
    ]);
    expect(mockInvoke).toHaveBeenCalledWith("mpv_close", { token: "old" });
    await closeNativePlayback("new");
  });

  it("does not open a cancelled queued request and recovers after an open failure", async () => {
    mockInvoke.mockReset();
    mockInvoke.mockRejectedValueOnce(new Error("fixture failure"));
    await expect(openNativePlayback(options, () => false)).rejects.toThrow("fixture failure");
    expect(await openNativePlayback(options, () => true)).toBeNull();
    mockInvoke.mockResolvedValueOnce({ token: "next", embedded: true });
    expect((await openNativePlayback(options, () => false))?.token).toBe("next");
    expect(mockInvoke).toHaveBeenCalledTimes(2);
  });
});
