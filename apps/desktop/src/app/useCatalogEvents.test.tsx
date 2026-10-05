import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ImportProgress } from "@/types";
import { useCatalogEvents } from "./useCatalogEvents";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  handlers: new Map<string, (e: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    mocks.handlers.set(event, handler);
    return () => mocks.handlers.delete(event);
  }),
}));

function progress(overrides: Partial<ImportProgress & { profileId: string }>) {
  return {
    profileId: "p",
    sourceId: 1,
    stage: "done",
    parsed: 0,
    inserted: 0,
    liveCount: 0,
    movieCount: 0,
    seriesCount: 0,
    failedCount: 0,
    skippedCount: 0,
    elapsedMs: 0,
    done: true,
    error: null,
    warning: null,
    ...overrides,
  } as ImportProgress & { profileId: string };
}

beforeEach(() => {
  mocks.invoke.mockReset().mockResolvedValue(undefined);
  mocks.handlers.clear();
});

function mount() {
  const qc = new QueryClient();
  const invalidate = vi.spyOn(qc, "invalidateQueries");
  const onImportError = vi.fn();
  const onImportDone = vi.fn();
  renderHook(() =>
    useCatalogEvents({ qc, profileId: "p", activeSourceId: 1, onImportError, onImportDone })
  );
  return { invalidate, onImportError, onImportDone };
}

describe("background playlist refresh", () => {
  it("quietly picks up the new catalog without the manual import UI", async () => {
    const { invalidate, onImportError, onImportDone } = mount();
    await waitFor(() => expect(mocks.handlers.has("playlist:import:background")).toBe(true));
    mocks.handlers.get("playlist:import:background")!({ payload: progress({}) });
    expect(invalidate).toHaveBeenCalled();
    expect(mocks.invoke).toHaveBeenCalledWith("start_enrichment", { profileId: "p", sourceId: 1 });
    expect(onImportDone).not.toHaveBeenCalled();
    expect(onImportError).not.toHaveBeenCalled();
  });

  it("never surfaces a failed background refresh", async () => {
    const { invalidate, onImportError } = mount();
    await waitFor(() => expect(mocks.handlers.has("playlist:import:background")).toBe(true));
    mocks.handlers.get("playlist:import:background")!({
      payload: progress({ error: "provider unreachable" }),
    });
    expect(onImportError).not.toHaveBeenCalled();
    expect(invalidate).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("ignores another profile's refresh of a source with the same id", async () => {
    const { invalidate } = mount();
    await waitFor(() => expect(mocks.handlers.has("playlist:import:background")).toBe(true));
    mocks.handlers.get("playlist:import:background")!({
      payload: progress({ profileId: "other" }),
    });
    expect(invalidate).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
