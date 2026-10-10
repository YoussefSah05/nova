import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

import { ScreenActiveContext } from "@/lib/screenActive";
import { useAudioLanguages, useMovieAudioTarget, type AudioTarget } from "./useAudioLanguages";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

function wrapper(active = true, client = new QueryClient()) {
  return ({ children }: { children: ReactNode }) => (
    <ScreenActiveContext.Provider value={active}>
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    </ScreenActiveContext.Provider>
  );
}

beforeEach(() => mocks.invoke.mockReset());
afterEach(cleanup);

it("probes only the addressed source, then switches when its media target changes", async () => {
  mocks.invoke.mockResolvedValue({ languages: ["fr"], unknownTracks: 0, trackCount: 1 });
  const { result, rerender } = renderHook(
    ({ target }: { target: AudioTarget }) => useAudioLanguages("p", target),
    { wrapper: wrapper(), initialProps: { target: { mediaKind: "vod", mediaId: 4 } } }
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(mocks.invoke).toHaveBeenCalledWith("get_audio_languages", {
    profileId: "p",
    mediaKind: "vod",
    mediaId: 4,
  });
  rerender({ target: { mediaKind: "episode", mediaId: 6 } });
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("get_audio_languages", {
      profileId: "p",
      mediaKind: "episode",
      mediaId: 6,
    })
  );
});

it("does not probe a hidden detail or a missing source", async () => {
  const hidden = renderHook(() => useAudioLanguages("p", { mediaKind: "vod", mediaId: 4 }), {
    wrapper: wrapper(false),
  });
  const absent = renderHook(() => useAudioLanguages("p", null), { wrapper: wrapper() });
  await waitFor(() => expect(hidden.result.current.fetchStatus).toBe("idle"));
  expect(absent.result.current.fetchStatus).toBe("idle");
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it("reuses recently verified audio tags without a second provider request", async () => {
  const client = new QueryClient();
  const data = { languages: ["ar"], unknownTracks: 1, trackCount: 2 };
  client.setQueryData(["audio-languages", "p", "vod", 4], data);
  const { result } = renderHook(() => useAudioLanguages("p", { mediaKind: "vod", mediaId: 4 }), {
    wrapper: wrapper(true, client),
  });
  await waitFor(() => expect(result.current.data).toEqual(data));
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it("keeps a failed probe nonfatal and allows an explicit retry", async () => {
  mocks.invoke.mockRejectedValueOnce(new Error("probe unavailable"));
  const { result } = renderHook(() => useAudioLanguages("p", { mediaKind: "vod", mediaId: 4 }), {
    wrapper: wrapper(),
  });
  await waitFor(() => expect(result.current.isError).toBe(true));
  expect(mocks.invoke).toHaveBeenCalledTimes(1);
  mocks.invoke.mockResolvedValue({ languages: ["en"], unknownTracks: 0, trackCount: 1 });
  await result.current.refetch();
  await waitFor(() => expect(result.current.data?.languages).toEqual(["en"]));
});

it("uses the exact picked movie variant without starting playback", async () => {
  mocks.invoke.mockResolvedValue({ mediaKind: "vod", mediaId: 42, variantId: "v2" });
  const { result } = renderHook(() => useMovieAudioTarget("p", "m1", "v2", true), {
    wrapper: wrapper(),
  });
  await waitFor(() => expect(result.current.data).toEqual({ mediaKind: "vod", mediaId: 42 }));
  expect(mocks.invoke).toHaveBeenCalledTimes(1);
  expect(mocks.invoke).toHaveBeenCalledWith("resolve_movie_playback", {
    profileId: "p",
    entityId: "m1",
    variantId: "v2",
  });
});

it("waits for canonical detail and does not resolve a movie in a hidden screen", () => {
  renderHook(() => useMovieAudioTarget("p", "m1", null, false), { wrapper: wrapper() });
  renderHook(() => useMovieAudioTarget("p", "m1", "v2", true), { wrapper: wrapper(false) });
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it("resolves the new picked variant when the profile's preferred version changes", async () => {
  mocks.invoke.mockResolvedValueOnce({ mediaKind: "vod", mediaId: 42, variantId: "v2" });
  const { result, rerender } = renderHook(
    ({ variantId }: { variantId: string }) => useMovieAudioTarget("p", "m1", variantId, true),
    { wrapper: wrapper(), initialProps: { variantId: "v2" } }
  );
  await waitFor(() => expect(result.current.data?.mediaId).toBe(42));
  mocks.invoke.mockResolvedValueOnce({ mediaKind: "vod", mediaId: 43, variantId: "v3" });
  rerender({ variantId: "v3" });
  await waitFor(() => expect(result.current.data?.mediaId).toBe(43));
  expect(mocks.invoke).toHaveBeenLastCalledWith("resolve_movie_playback", {
    profileId: "p",
    entityId: "m1",
    variantId: "v3",
  });
});
