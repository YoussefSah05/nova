import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { useSemanticSearchAvailable } from "./data";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
it.each([true, false])(
  "semantic capability respects enabled=%s for an installed model",
  async (enabled) => {
    mocks.invoke.mockResolvedValue([
      { capability: "embedding", availability: "available", enabled },
    ]);
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
    const { result, unmount } = renderHook(useSemanticSearchAvailable, { wrapper });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toBe(enabled);
    expect(mocks.invoke).toHaveBeenCalledWith("list_model_controls");
    unmount();
  }
);
