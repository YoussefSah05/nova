import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { expect, it, vi } from "vitest";

import type { Channel, Program } from "@/types";
import { useGuidePrograms } from "./data";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

const channel = (id: number) => ({ id, name: `Ch ${id}` }) as Channel;
const show: Program = {
  channelTvgId: "c1",
  title: "Show",
  subTitle: null,
  description: null,
  startUtc: 0,
  stopUtc: 10_000,
  category: null,
  iconUrl: null,
};

it("keeps a row's programmes while the window moves, and reports what failed", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  mocks.invoke.mockResolvedValueOnce([{ channelId: 1, programs: [show] }]);
  const channels = [channel(1)];
  const { result, rerender } = renderHook(
    ({ from }) => useGuidePrograms("p", channels, 0, 0, { from, to: from + 3600 }),
    { wrapper, initialProps: { from: 0 } }
  );
  await waitFor(() => expect(result.current.programs.get(1)).toEqual([show]));

  // The next half hour fails to load: the row keeps what it showed.
  mocks.invoke.mockRejectedValueOnce(new Error("offline"));
  rerender({ from: 1800 });
  await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(2));
  expect(result.current.programs.get(1)).toEqual([show]);
  expect(result.current.failed.size).toBe(0);

  // A channel never loaded whose block fails is reported.
  mocks.invoke.mockRejectedValueOnce(new Error("offline"));
  const { result: other } = renderHook(
    () => useGuidePrograms("p", [channel(2)], 0, 0, { from: 0, to: 3600 }),
    { wrapper }
  );
  await waitFor(() => expect(other.current.failed.has(2)).toBe(true));
  client.clear();
});
