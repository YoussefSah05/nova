import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

import { interpret } from "@/ui/interpret";
import { planFrom, useSearchChannels, useSearchMovies, useSearchSeries } from "./data";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("type-only search", () => {
  it.each([
    ["movies", "list_movies", useSearchMovies],
    ["shows", "list_series", useSearchSeries],
    ["channels", "list_channel_groups", useSearchChannels],
  ] as const)("browses a bounded list for %s", async (query, command, useSearch) => {
    vi.mocked(invoke).mockReset().mockResolvedValue([]);
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const parsed = interpret(query, []);
    const { result, unmount } = renderHook(
      () => useSearch("fixture", 7, planFrom(parsed.text, parsed.facets)),
      {
        wrapper: ({ children }: { children: ReactNode }) => (
          <QueryClientProvider client={client}>{children}</QueryClientProvider>
        ),
      }
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(invoke).toHaveBeenCalledWith(
      command,
      expect.objectContaining({
        profileId: "fixture",
        sourceId: 7,
        offset: 0,
        limit: expect.any(Number),
      })
    );
    expect(invoke).toHaveBeenCalledTimes(1);
    unmount();
    client.clear();
  });
});
