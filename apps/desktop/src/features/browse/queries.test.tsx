import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { CanonicalMediaSummary } from "@/types";
import {
  browseKeys,
  fetchBrowsePage,
  patchItemEverywhere,
  useBrowseGrid,
  type BrowsePage,
} from "./queries";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
function movie(id: number): CanonicalMediaSummary {
  return {
    entityId: `mov-${id}`,
    kind: "movie",
    title: "Same Movie",
    sourceTitle: "Same Movie FHD",
    year: 2024,
    category: "Movies",
    posterUrl: null,
    backdropUrl: null,
    overview: null,
    isFavorite: false,
    isInWatchlist: false,
    addedAt: null,
    dedupGroup: null,
    genres: [],
    runtimeMin: null,
    rating: null,
    ratingSource: null,
    badges: [],
    versionCount: 1,
  };
}
beforeEach(() => mocks.invoke.mockReset());
it("advances by full database pages; canonical movies are never grouped client-side", async () => {
  // Entity resolution decides which copies are one film; the client shows one
  // card per canonical entity, even when raw titles look alike.
  mocks.invoke.mockResolvedValueOnce(Array.from({ length: 200 }, (_, i) => movie(i)));
  const page = await fetchBrowsePage("movies", "p", 1, {}, 0);
  expect(page.items).toHaveLength(200);
  expect(page.items.every((item) => !item.variants?.length)).toBe(true);
  expect(page.nextOffset).toBe(200);
  mocks.invoke.mockResolvedValueOnce([movie(200)]);
  const last = await fetchBrowsePage("movies", "p", 1, {}, page.nextOffset!);
  expect(last.nextOffset).toBeNull();
  expect(mocks.invoke).toHaveBeenLastCalledWith(
    "list_movies",
    expect.objectContaining({ limit: 200, offset: 200 })
  );
});
it("releases inactive grid pages instead of accumulating catalog objects", async () => {
  mocks.invoke.mockResolvedValue([movie(1)]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const { result, rerender, unmount } = renderHook(
    ({ offset }) => useBrowseGrid("movies", "p", 1, {}, offset),
    { initialProps: { offset: 0 }, wrapper }
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  for (const offset of [200, 400, 600, 800]) {
    rerender({ offset });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    await waitFor(() => expect(client.getQueryCache().getAll()).toHaveLength(1));
  }
  unmount();
  client.clear();
});
it("favorite updates and rollback work with bounded page payloads", async () => {
  mocks.invoke.mockResolvedValue([movie(1)]);
  const page = await fetchBrowsePage("movies", "p", 1, {}, 0);
  const client = new QueryClient();
  const key = [...browseKeys.grid("p", 1, "movies", {}), 0];
  client.setQueryData(key, page);
  const undo = patchItemEverywhere(client, "p", 1, "movie-mov-1", { isFavorite: true });
  expect(client.getQueryData<BrowsePage>(key)?.items[0].isFavorite).toBe(true);
  undo();
  expect(client.getQueryData<BrowsePage>(key)?.items[0].isFavorite).toBe(false);
  client.clear();
});
it("movies are canonical entities addressed by entity id, not provider rows", async () => {
  mocks.invoke.mockResolvedValueOnce([movie(1), { ...movie(2), sourceTitle: "Other Film" }]);
  const page = await fetchBrowsePage("movies", "p", 1, {}, 0);
  expect(mocks.invoke).toHaveBeenCalledWith(
    "list_movies",
    expect.objectContaining({ profileId: "p", sourceId: 1 })
  );
  expect(page.items.map((item) => [item.kind, item.key, item.entityId])).toEqual([
    ["movie", "movie-mov-1", "mov-1"],
    ["movie", "movie-mov-2", "mov-2"],
  ]);
  expect(JSON.stringify(page.items)).not.toMatch(/streamUrl|http/);
});
