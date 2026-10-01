import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { expect, it, vi } from "vitest";
import { PlaybackProvider } from "./PlaybackProvider";
import { usePlayback } from "./playback";

const mocks = vi.hoisted(() => ({ flush: Promise.resolve(), read: vi.fn(), setError: vi.fn() }));
vi.mock("./session", () => ({
  useSession: () => ({ profile: { id: "p" }, activeSourceId: 1, setError: mocks.setError }),
  errorMessage: String,
}));
vi.mock("@/features/player/PlayerModal", () => ({
  PlayerModal: ({
    onClose,
    onProgressSaved,
  }: {
    onClose: () => void;
    onProgressSaved: () => void;
  }) => {
    useEffect(
      () => () => {
        void mocks.flush.then(onProgressSaved);
      },
      [onProgressSaved]
    );
    return <button onClick={onClose}>Close player</button>;
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
