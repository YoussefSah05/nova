import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { VodItem } from "@/types";
import {
  browseKeys,
  fetchBrowsePage,
  patchItemEverywhere,
  useBrowseGrid,
  type BrowsePage,
} from "./queries";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
function movie(id: number): VodItem {
  return {
    id,
    sourceId: 1,
    name: "Same Movie FHD",
    cleanTitle: null,
    year: 2024,
    logoUrl: null,
    streamUrl: "http://127.0.0.1:9/fixture",
    category: "Movies",
    isFavorite: false,
    isInWatchlist: false,
    addedAt: null,
    dedupGroup: null,
    canonicalTitle: null,
    posterUrl: null,
    backdropUrl: null,
    overview: null,
  };
}
beforeEach(() => mocks.invoke.mockReset());
it("advances past a full database page even when variants collapse to one card", async () => {
  mocks.invoke.mockResolvedValueOnce(Array.from({ length: 200 }, (_, i) => movie(i)));
  const page = await fetchBrowsePage("movies", "p", 1, {}, 0);
  expect(page.items).toHaveLength(1);
  expect(page.items[0].variants).toHaveLength(199);
  expect(page.nextOffset).toBe(200);
  mocks.invoke.mockResolvedValueOnce([movie(200)]);
  const last = await fetchBrowsePage("movies", "p", 1, {}, page.nextOffset!);
  expect(last.nextOffset).toBeNull();
  expect(mocks.invoke).toHaveBeenLastCalledWith(
    "list_vod",
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
  const undo = patchItemEverywhere(client, "p", 1, "vod-1", { isFavorite: true });
  expect(client.getQueryData<BrowsePage>(key)?.items[0].isFavorite).toBe(true);
  undo();
  expect(client.getQueryData<BrowsePage>(key)?.items[0].isFavorite).toBe(false);
  client.clear();
});
