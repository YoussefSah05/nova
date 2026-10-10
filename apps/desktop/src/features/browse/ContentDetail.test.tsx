import { cleanup, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

import { ContentDetail } from "./ContentDetail";
import { vodToItem } from "./queries";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/features/settings/useProfileSettings", () => ({
  useProfileSettings: () => ({ settings: { preferredAudioLang: null }, setSettings: vi.fn() }),
}));
vi.mock("./titleArt", () => ({ useTitleArt: () => null, titleArtSubject: () => null }));
vi.mock("./detail/data", async (importOriginal) => {
  const original = await importOriginal<typeof import("./detail/data")>();
  return {
    ...original,
    useTitleFacts: () => ({ data: undefined }),
    useRelatedTitles: () => ({ data: undefined }),
  };
});
vi.mock("./detail/TitleHero", () => ({
  TitleHero: ({ audioBanner }: { audioBanner: ReactNode }) => <>{audioBanner}</>,
}));

afterEach(cleanup);

it("probes a playable legacy VOD by its provider row without requiring a canonical entity", async () => {
  mocks.invoke.mockResolvedValue({ languages: ["en"], unknownTracks: 0, trackCount: 1 });
  const item = vodToItem({
    id: 27,
    sourceId: 3,
    name: "FR | Example VOSTFR",
    cleanTitle: "Example",
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
  });
  render(
    <QueryClientProvider client={new QueryClient()}>
      <ContentDetail
        profileId="p"
        sourceId={3}
        item={item}
        onBack={vi.fn()}
        onPlay={vi.fn()}
        onToggleFavorite={vi.fn()}
        onToggleWatchlist={vi.fn()}
        onOpenTitle={vi.fn()}
      />
    </QueryClientProvider>
  );
  expect(await screen.findByLabelText("English")).toHaveTextContent("EN");
  expect(screen.queryByLabelText("French")).not.toBeInTheDocument();
  expect(mocks.invoke).toHaveBeenCalledTimes(1);
  expect(mocks.invoke).toHaveBeenCalledWith("get_audio_languages", {
    profileId: "p",
    mediaKind: "vod",
    mediaId: 27,
  });
  expect(screen.queryByText("No playable source available")).not.toBeInTheDocument();
});
