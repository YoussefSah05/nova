import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useAirPlay } from "./useAirPlay";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

describe("AirPlay lifetime", () => {
  it("releases a handoff that finishes after the player closes", async () => {
    let picker!: () => void;
    vi.mocked(listen).mockImplementation(async (event, handler) => {
      if (event === "airplay:picker-closed")
        picker = () => handler({ event, id: 1, payload: undefined });
      return () => {};
    });
    let finish!: (result: { proxyToken: string }) => void;
    vi.mocked(invoke).mockImplementation((command) =>
      command === "airplay_engage"
        ? new Promise((resolve) => {
            finish = resolve;
          })
        : Promise.resolve()
    );
    const onEngage = vi.fn();
    const { unmount } = renderHook(() =>
      useAirPlay({
        enabled: true,
        profileId: "fixture",
        mediaKind: "vod",
        mediaId: 1,
        layoutKey: "normal",
        controlsVisible: true,
        getPosition: () => 0,
        getPlaying: () => true,
        onEngage,
        onDisengage: vi.fn(),
      })
    );
    await act(async () => {});
    await act(async () => {
      picker();
    });
    unmount();
    await act(async () => {
      finish({ proxyToken: "late-fixture-session" });
    });
    expect(invoke).toHaveBeenCalledWith("stop_playback", { token: "late-fixture-session" });
    expect(onEngage).not.toHaveBeenCalled();
  });
});
