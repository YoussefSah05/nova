import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

import { prefetchTitle } from "./prefetch";
import { canonicalMovieQuery, useTitleFacts } from "./detail/data";
import { useTitleArtFor } from "./titleArt";
import type { BrowseItem } from "./queries";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

const movie: BrowseItem = {
  key: "movie-m1",
  kind: "movie",
  id: 0,
  title: "Heat",
  subtitle: null,
  posterUrl: null,
  tvgId: null,
  isFavorite: false,
  isInWatchlist: false,
  entityId: "m1",
};

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "get_title_art":
        return {
          logoUrl: "logo",
          logoLanguage: null,
          logoWidth: 1,
          logoHeight: 1,
          backdropUrl: "bd",
        };
      case "get_title_facts":
        return { tagline: "t" };
      case "get_movie":
        return { entityId: "m1", title: "Heat", variants: [], resume: null };
      default:
        throw new Error(cmd);
    }
  });
});

it("a hover prefetch fills exactly what the page reads, so opening it sends nothing", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  prefetchTitle(client, "p", movie);
  await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(3));

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const subject = { kind: "movie", entityId: "m1" } as const;
  const { result } = renderHook(
    () => ({
      art: useTitleArtFor("p", subject),
      facts: useTitleFacts("p", subject).data,
      detail: useQuery({ ...canonicalMovieQuery("p", "m1"), staleTime: 60_000 }).data,
    }),
    { wrapper }
  );
  expect(result.current.art?.logoUrl).toBe("logo");
  expect(result.current.facts).toEqual({ tagline: "t" });
  expect(result.current.detail?.title).toBe("Heat");
  expect(mocks.invoke).toHaveBeenCalledTimes(3);
});

it("prefetches nothing for a card without a canonical title", () => {
  const client = new QueryClient();
  prefetchTitle(client, "p", { ...movie, kind: "live", entityId: undefined });
  expect(mocks.invoke).not.toHaveBeenCalled();
});
