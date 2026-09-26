import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ImportProgress } from "@/types";
import { AddSourceModal } from "./AddSourceModal";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  unlisten: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

type ProgressHandler = (event: { payload: ImportProgress }) => void;

let progressHandler: ProgressHandler | null = null;

function setupProgressListener() {
  const implementation = (_eventName: string, handler: ProgressHandler): Promise<() => void> => {
    progressHandler = handler;
    return Promise.resolve(mocks.unlisten);
  };
  mocks.listen.mockImplementation(implementation);
}

function progress(overrides: Partial<ImportProgress>): ImportProgress {
  return {
    sourceId: 42,
    stage: "receiving",
    parsed: 0,
    inserted: 0,
    liveCount: 0,
    movieCount: 0,
    seriesCount: 0,
    failedCount: 0,
    skippedCount: 0,
    elapsedMs: 0,
    done: false,
    error: null,
    warning: null,
    ...overrides,
  };
}

describe("AddSourceModal", () => {
  beforeEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
    progressHandler = null;
    setupProgressListener();
  });

  it("returns to the form when add_source fails", async () => {
    mocks.invoke.mockRejectedValueOnce({ kind: "Internal", message: "database locked" });

    render(<AddSourceModal profileId="profile-1" onDone={vi.fn()} onCancel={vi.fn()} />);

    await userEvent.type(
      screen.getByPlaceholderText("http://provider.com/playlist.m3u"),
      "http://example.com/list.m3u"
    );
    await userEvent.click(screen.getByRole("button", { name: "Import" }));

    await waitFor(() => expect(screen.getByText("database locked")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Import" })).toBeInTheDocument();
    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });

  it("finishes when the backend emits a terminal success event", async () => {
    const onDone = vi.fn();
    // `add_source` returns the new source id; subsequent invokes (e.g. the
    // Phase 4 `epg_refresh` kicked off on import completion) resolve to undefined.
    mocks.invoke.mockResolvedValue(undefined);
    mocks.invoke.mockResolvedValueOnce(42);

    render(<AddSourceModal profileId="profile-1" onDone={onDone} onCancel={vi.fn()} />);

    await userEvent.type(
      screen.getByPlaceholderText("http://provider.com/playlist.m3u"),
      "http://example.com/list.m3u"
    );
    await userEvent.click(screen.getByRole("button", { name: "Import" }));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledOnce());
    if (!progressHandler) {
      throw new Error("progress listener was not registered");
    }

    act(() => {
      progressHandler?.({
        payload: progress({
          stage: "complete",
          parsed: 1200,
          inserted: 1200,
          liveCount: 1000,
          movieCount: 200,
          done: true,
        }),
      });
    });

    await waitFor(() => expect(screen.getByText("Ready to browse")).toBeInTheDocument());
    expect(onDone).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Browse library" }));
    expect(onDone).toHaveBeenCalledOnce();
    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });
  it("does not claim completion at 50k or 100k when total size is unknown", async () => {
    mocks.invoke.mockResolvedValue(42);
    render(<AddSourceModal profileId="profile-1" onDone={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("M3U URL"), "http://example.com/list.m3u");
    await userEvent.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledOnce());
    for (const parsed of [50000, 100000, 250000]) {
      act(() =>
        progressHandler?.({ payload: progress({ parsed, inserted: parsed, stage: "receiving" }) })
      );
      expect(screen.getByRole("progressbar")).not.toHaveAttribute("aria-valuenow");
      expect(screen.queryByText("Ready to browse")).not.toBeInTheDocument();
      expect(
        screen.getByText(
          `${parsed.toLocaleString()} entries read · ${parsed.toLocaleString()} saved`
        )
      ).toBeInTheDocument();
    }
  });

  it("clears the provider stall watchdog while an import waits for resources", async () => {
    mocks.invoke.mockResolvedValue(42);
    render(<AddSourceModal profileId="profile-1" onDone={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("M3U URL"), "http://example.com/list.m3u");
    await userEvent.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledOnce());

    const clearTimeoutSpy = vi.spyOn(globalThis, "clearTimeout");
    clearTimeoutSpy.mockClear();
    act(() => progressHandler?.({ payload: progress({ stage: "queued" }) }));
    expect(screen.getByText("Waiting for available resources…")).toBeInTheDocument();
    expect(clearTimeoutSpy).toHaveBeenCalled();
    expect(screen.getByRole("progressbar")).toBeInTheDocument();
    clearTimeoutSpy.mockRestore();
  });
});
