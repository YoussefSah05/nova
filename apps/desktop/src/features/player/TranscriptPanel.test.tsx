import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TranscriptPanel } from "./TranscriptPanel";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@/lib/motion", () => ({ useReducedMotion: () => true }));
beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(invoke).mockReset();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("does not display an older search response after the query changes", async () => {
  let completeOld: (hits: unknown) => void = () => {};
  vi.mocked(invoke).mockImplementation((command, args) => {
    if (command === "get_transcript") return Promise.resolve([]);
    if (args && "query" in args && args.query === "old")
      return new Promise((resolve) => {
        completeOld = resolve;
      });
    return Promise.resolve([
      { segmentId: 2, startSec: 20, endSec: 30, text: "New matching line", kind: "source" },
    ]);
  });
  render(
    <TranscriptPanel
      embedded
      profileId="p"
      media={{ kind: "vod", id: 1 }}
      audioTrackIndex={0}
      currentTime={40}
      sourceCues={[]}
      onSeek={vi.fn()}
      onClose={vi.fn()}
    />
  );
  await act(async () => {});
  fireEvent.change(screen.getByLabelText("Search dialogue"), { target: { value: "old" } });
  await act(async () => {
    vi.advanceTimersByTime(250);
  });
  fireEvent.change(screen.getByLabelText("Search dialogue"), { target: { value: "new" } });
  await act(async () => {
    vi.advanceTimersByTime(250);
  });
  await act(async () => {
    completeOld([{ segmentId: 1, startSec: 10, endSec: 20, text: "Stale result", kind: "source" }]);
  });
  expect(screen.queryByText("Stale result")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: /New matching line/i })).toBeInTheDocument();
});
