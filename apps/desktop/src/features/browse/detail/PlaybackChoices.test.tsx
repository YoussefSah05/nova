import { useAudioTrackChoice } from "./useAudioTrackChoice";
import { act, cleanup, render, renderHook, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AudioTrackPicker, PlayWithCaptions } from "./PlaybackChoices";
import type { AudioLanguages } from "@/types";
import type { AudioTarget } from "./useAudioLanguages";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const en: AudioLanguages = {
  languages: ["en"],
  unknownTracks: 0,
  trackCount: 1,
  tracks: [{ audioIndex: 0, language: "en" }],
};
beforeEach(() => invoke.mockReset().mockResolvedValue(en));
afterEach(cleanup);
function wrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}
function Choices({ onPlay }: { onPlay: ReturnType<typeof vi.fn> }) {
  const choice = useAudioTrackChoice("p", { mediaKind: "vod", mediaId: 1 });
  return (
    <>
      <AudioTrackPicker choice={choice} />
      <PlayWithCaptions onPlay={onPlay} audioTrack={choice.launchTrack} />
    </>
  );
}
it("lists only recorded EN audio while FR and AR remain translation targets", async () => {
  const play = vi.fn();
  const user = userEvent.setup();
  render(<Choices onPlay={play} />, { wrapper: wrapper() });
  await waitFor(() => expect(invoke).toHaveBeenCalled());
  await user.click(screen.getByRole("button", { name: "Audio: Auto" }));
  await user.click(await screen.findByRole("menuitem", { name: "EN" }));
  expect(screen.queryByRole("menuitem", { name: "FR" })).not.toBeInTheDocument();
  expect(screen.queryByRole("menuitem", { name: "AR" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Playback options" }));
  await user.click(screen.getByRole("menuitem", { name: "Translated captions · AR" }));
  expect(play).toHaveBeenCalledWith({
    captionMode: "translation",
    translationTarget: "ar",
    audioTrack: { mediaKind: "vod", mediaId: 1, index: 0, language: "en" },
  });
  // Opening the menu only warms the speech model; nothing is written.
  expect(invoke).toHaveBeenCalledWith("prewarm_captions");
  expect(
    invoke.mock.calls.every(([command]) =>
      ["get_audio_languages", "prewarm_captions"].includes(command)
    )
  ).toBe(true);
});

it("distinguishes duplicate languages and untagged tracks", async () => {
  invoke.mockResolvedValue({
    languages: ["fr"],
    unknownTracks: 1,
    trackCount: 3,
    tracks: [
      { audioIndex: 0, language: "fr" },
      { audioIndex: 1, language: "fr" },
      { audioIndex: 2, language: null },
    ],
  });
  render(<Choices onPlay={vi.fn()} />, { wrapper: wrapper() });
  await userEvent.click(screen.getByRole("button", { name: "Audio: Auto" }));
  expect(await screen.findByRole("menuitem", { name: "FR · Track 1" })).toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: "FR · Track 2" })).toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: "Unknown · Track 3" })).toBeInTheDocument();
});

it("clears a selection across versions and ignores a late response from the old source", async () => {
  let finish!: (tracks: AudioLanguages) => void;
  invoke
    .mockImplementationOnce(
      () =>
        new Promise<AudioLanguages>((resolve) => {
          finish = resolve;
        })
    )
    .mockResolvedValue(en);
  const { result, rerender } = renderHook(({ target }) => useAudioTrackChoice("p", target), {
    wrapper: wrapper(),
    initialProps: { target: { mediaKind: "vod", mediaId: 1 } as AudioTarget },
  });
  rerender({ target: { mediaKind: "vod", mediaId: 2 } });
  await waitFor(() => expect(result.current.query.data).toEqual(en));
  await act(async () =>
    finish({ ...en, languages: ["ar"], tracks: [{ audioIndex: 0, language: "ar" }] })
  );
  expect(result.current.query.data).toEqual(en);
  act(() => result.current.choose(0));
  expect(result.current.launchTrack?.mediaId).toBe(2);
  rerender({ target: { mediaKind: "vod", mediaId: 1 } });
  expect(result.current.launchTrack).toBeUndefined();
});

it("shows a failed probe retry instead of invented languages", async () => {
  invoke.mockRejectedValueOnce(new Error("private source failed")).mockResolvedValue(en);
  render(<Choices onPlay={vi.fn()} />, { wrapper: wrapper() });
  await userEvent.click(screen.getByRole("button", { name: "Audio: Auto" }));
  await userEvent.click(await screen.findByRole("menuitem", { name: "Audio unavailable · Retry" }));
  await userEvent.click(screen.getByRole("button", { name: "Audio: Auto" }));
  expect(await screen.findByRole("menuitem", { name: "EN" })).toBeInTheDocument();
  expect(screen.queryByText("private source failed")).not.toBeInTheDocument();
});
