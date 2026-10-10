import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PlaybackProvider } from "./PlaybackProvider";
import { usePlayback } from "./playback";
import type { PlaybackLaunchOptions } from "@/features/player/playbackLaunch";
import type { PlayMedia } from "@/features/player/PlayerModal";
import { vodToItem } from "@/features/browse/queries";

const mocks = vi.hoisted(() => ({
  flush: Promise.resolve(),
  read: vi.fn(),
  setError: vi.fn(),
  invoke: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
beforeEach(() => {
  mocks.flush = Promise.resolve();
  mocks.invoke.mockReset();
  mocks.read.mockReset();
});
afterEach(cleanup);
vi.mock("./session", () => ({
  useSession: () => ({ profile: { id: "p" }, activeSourceId: 1, setError: mocks.setError }),
  errorMessage: String,
}));
vi.mock("@/features/player/PlayerModal", () => ({
  PlayerModal: ({
    onClose,
    onProgressSaved,
    media,
    launchOptions,
    onPlayMedia,
  }: {
    onClose: () => void;
    onProgressSaved: () => void;
    media: PlayMedia;
    launchOptions?: PlaybackLaunchOptions;
    onPlayMedia: (media: PlayMedia) => void;
  }) => {
    const [mode, setMode] = useState(launchOptions?.captionMode ?? "off");
    useEffect(
      () => () => {
        void mocks.flush.then(onProgressSaved);
      },
      [onProgressSaved]
    );
    return (
      <>
        <button onClick={onClose}>Close player</button>
        <p data-testid="launch">{JSON.stringify(launchOptions)}</p>
        <p data-testid="media">{media.id}</p>
        <p data-testid="mode">{mode}</p>
        <button onClick={() => setMode("source")}>Change captions</button>
        <button onClick={() => onPlayMedia({ kind: "episode", id: 2, title: "Next" })}>
          Next episode
        </button>
      </>
    );
  },
}));
function History() {
  const { playMedia } = usePlayback();
  const history = useQuery({
    queryKey: ["browse", "p", 1, "history", "all", 0],
    queryFn: mocks.read,
    staleTime: 300000,
  });
  return (
    <>
      <p>{history.data}</p>
      <button onClick={() => playMedia({ kind: "vod", id: 1, title: "Film" })}>Play</button>
    </>
  );
}
it("refreshes an active history view after the delayed final progress write", async () => {
  let finish!: () => void;
  mocks.flush = new Promise<void>((resolve) => {
    finish = resolve;
  });
  let saved = "Old progress";
  mocks.read.mockImplementation(async () => saved);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const user = userEvent.setup();
  render(
    <QueryClientProvider client={client}>
      <PlaybackProvider>
        <History />
      </PlaybackProvider>
    </QueryClientProvider>
  );
  await screen.findByText("Old progress");
  await user.click(screen.getByRole("button", { name: "Play" }));
  await user.click(screen.getByRole("button", { name: "Close player" }));
  await waitFor(() => expect(mocks.read).toHaveBeenCalledTimes(2));
  expect(screen.getByText("Old progress")).toBeInTheDocument();
  await act(async () => {
    saved = "Final progress";
    finish();
    await mocks.flush;
  });
  expect(await screen.findByText("Final progress")).toBeInTheDocument();
  expect(mocks.read).toHaveBeenCalledTimes(3);
});

function Launcher() {
  const { playMedia, playItem } = usePlayback();
  const media: PlayMedia = { kind: "episode", id: 1, title: "Film" };
  const translated: PlaybackLaunchOptions = {
    captionMode: "translation",
    translationTarget: "ar",
    audioTrack: { mediaKind: "episode", mediaId: 1, index: 1, language: "en" },
  };
  return (
    <>
      <button onClick={() => playMedia(media, translated)}>Launch translated</button>
      <button onClick={() => playMedia(media, { captionMode: "off" })}>Launch normal</button>
      <button
        onClick={() =>
          void playItem(
            {
              ...vodToItem({
                id: 9,
                sourceId: 1,
                name: "Film",
                cleanTitle: "Film",
                year: null,
                logoUrl: null,
                category: null,
                isFavorite: false,
                isInWatchlist: false,
                addedAt: null,
                dedupGroup: null,
                canonicalTitle: null,
                posterUrl: null,
                backdropUrl: null,
                overview: null,
              }),
              entityId: "film",
              kind: "movie",
            },
            translated
          )
        }
      >
        Resolve movie
      </button>
    </>
  );
}
function launchers() {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <PlaybackProvider>
        <Launcher />
      </PlaybackProvider>
    </QueryClientProvider>
  );
}
it("applies fresh options when explicitly reopening the same title", async () => {
  launchers();
  await userEvent.click(screen.getByRole("button", { name: "Launch translated" }));
  expect(screen.getByTestId("mode")).toHaveTextContent("translation");
  expect(screen.getByTestId("launch")).toHaveTextContent('"translationTarget":"ar"');
  await userEvent.click(screen.getByRole("button", { name: "Launch normal" }));
  expect(screen.getByTestId("mode")).toHaveTextContent("off");
  expect(screen.getByTestId("launch")).not.toHaveTextContent("audioTrack");
});
it("preserves manual caption changes on next episode while discarding the previous audio ordinal", async () => {
  launchers();
  await userEvent.click(screen.getByRole("button", { name: "Launch translated" }));
  await userEvent.click(screen.getByRole("button", { name: "Change captions" }));
  await userEvent.click(screen.getByRole("button", { name: "Next episode" }));
  expect(screen.getByTestId("media")).toHaveTextContent("2");
  expect(screen.getByTestId("mode")).toHaveTextContent("source");
  expect(screen.getByTestId("launch")).not.toHaveTextContent("audioTrack");
});
it("does not let a delayed canonical resolution replace a newer explicit launch", async () => {
  let finish!: (value: unknown) => void;
  mocks.invoke.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  launchers();
  await userEvent.click(screen.getByRole("button", { name: "Resolve movie" }));
  await userEvent.click(screen.getByRole("button", { name: "Launch normal" }));
  await act(async () => finish({ mediaKind: "vod", mediaId: 99 }));
  expect(screen.getByTestId("media")).toHaveTextContent("1");
  expect(screen.getByTestId("mode")).toHaveTextContent("off");
});

it("carries caption launch options through canonical playback resolution", async () => {
  mocks.invoke.mockResolvedValue({ mediaKind: "vod", mediaId: 99 });
  launchers();
  await userEvent.click(screen.getByRole("button", { name: "Resolve movie" }));
  await waitFor(() => expect(screen.getByTestId("media")).toHaveTextContent("99"));
  expect(mocks.invoke).toHaveBeenCalledWith("resolve_movie_playback", {
    profileId: "p",
    entityId: "film",
    variantId: null,
  });
  expect(screen.getByTestId("mode")).toHaveTextContent("translation");
  expect(screen.getByTestId("launch")).toHaveTextContent('"translationTarget":"ar"');
});
