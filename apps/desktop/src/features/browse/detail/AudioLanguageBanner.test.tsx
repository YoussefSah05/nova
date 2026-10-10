import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { AudioLanguageBanner } from "./AudioLanguageBanner";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

function banner(label = "Audio") {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <AudioLanguageBanner
        profileId="p"
        target={{ mediaKind: "episode", mediaId: 5 }}
        label={label}
      />
    </QueryClientProvider>
  );
}

beforeEach(() => mocks.invoke.mockReset());
afterEach(cleanup);

it("shows compact codes with accessible full language names and preserves unknown tracks", async () => {
  mocks.invoke.mockResolvedValue({
    languages: ["fr", "ar", "en"],
    unknownTracks: 1,
    trackCount: 4,
  });
  banner("Next episode audio");
  expect(await screen.findByLabelText("French")).toHaveTextContent("FR");
  expect(screen.getByLabelText("Arabic")).toHaveTextContent("AR");
  expect(screen.getByLabelText("English")).toHaveTextContent("EN");
  expect(screen.getByText("+1 untagged track")).toBeInTheDocument();
  expect(screen.getByLabelText("Next episode audio")).toBeInTheDocument();
});

it("does not invent languages for untagged audio tracks", async () => {
  mocks.invoke.mockResolvedValue({ languages: [], unknownTracks: 2, trackCount: 2 });
  banner();
  expect(await screen.findByText("Languages not provided")).toBeInTheDocument();
  expect(screen.queryByLabelText("Audio languages")).not.toBeInTheDocument();
});

it("distinguishes media with no audio streams from missing language tags", async () => {
  mocks.invoke.mockResolvedValue({ languages: [], unknownTracks: 0, trackCount: 0 });
  banner();
  expect(await screen.findByText("No audio tracks reported")).toBeInTheDocument();
});

it("keeps loading unobtrusive while metadata is being checked", async () => {
  mocks.invoke.mockResolvedValue({ languages: [], unknownTracks: 1, trackCount: 1 });
  banner();
  expect(screen.getByRole("status")).toHaveTextContent("Checking audio…");
  expect(await screen.findByText("Languages not provided")).toBeInTheDocument();
});

it("provides a retry after a nonfatal probe error without showing backend details", async () => {
  mocks.invoke.mockRejectedValueOnce(new Error("private upstream failure"));
  banner();
  const retry = await screen.findByRole("button", { name: "Retry audio information" });
  expect(screen.getByText("Audio information unavailable")).toBeInTheDocument();
  expect(screen.queryByText("private upstream failure")).not.toBeInTheDocument();
  mocks.invoke.mockResolvedValue({ languages: ["ar"], unknownTracks: 0, trackCount: 1 });
  fireEvent.click(retry);
  await waitFor(() => expect(screen.getByLabelText("Arabic")).toHaveTextContent("AR"));
});
