import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { PlayerCompanion } from "./PlayerCompanion";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => []) }));
vi.mock("@/lib/motion", () => ({ useReducedMotion: () => true }));
const props = {
  tab: "now" as const,
  onTab: vi.fn(),
  onClose: vi.fn(),
  profileId: "profile",
  media: { kind: "vod" as const, id: 1, title: "Library title" },
  currentTime: 20,
  audioTrackIndex: 0,
  sourceCues: [{ start: 10, end: 30, text: "Available dialogue" }],
  captionNote: null,
  onSeek: vi.fn(),
  onEnableCaptions: vi.fn(),
  onBrowse: vi.fn(),
};
beforeEach(() => {
  vi.clearAllMocks();
  HTMLElement.prototype.scrollIntoView = vi.fn();
});
afterEach(cleanup);

describe("Player companion frontend", () => {
  it("shows real dialogue and future capability states without starting intelligence work", () => {
    render(<PlayerCompanion {...props} />);
    expect(screen.getByText("“Available dialogue”")).toBeInTheDocument();
    expect(screen.getByText("Ask Nova · coming with intelligence")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Enable source captions" }));
    expect(props.onEnableCaptions).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalled();
  });
  it("retains temporary notes while the companion is closed", () => {
    const view = render(<PlayerCompanion {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Save moment" }));
    view.rerender(<PlayerCompanion {...props} tab="saved" />);
    fireEvent.change(screen.getByLabelText("Your note"), { target: { value: "Return here" } });
    view.rerender(<PlayerCompanion {...props} tab={null} />);
    view.rerender(<PlayerCompanion {...props} tab="saved" />);
    expect(screen.getByLabelText("Your note")).toHaveValue("Return here");
    fireEvent.click(screen.getByRole("button", { name: "0:20" }));
    expect(props.onSeek).toHaveBeenCalledWith(20);
  });
  it("filters future dialogue with the spoiler boundary", async () => {
    vi.mocked(invoke).mockResolvedValueOnce([
      { segmentId: 1, startSec: 5, endSec: 10, text: "Earlier line" },
      { segmentId: 2, startSec: 40, endSec: 45, text: "Future reveal" },
    ]);
    render(<PlayerCompanion {...props} tab="find" />);
    await waitFor(() => expect(screen.getByText("Earlier line")).toBeInTheDocument());
    expect(screen.queryByText("Future reveal")).not.toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith(
      "get_transcript",
      expect.objectContaining({ mediaId: 1, audioTrackIndex: 0 })
    );
  });
  it("does not present film transcript or seekable bookmarks for live TV", () => {
    const view = render(
      <PlayerCompanion {...props} media={{ kind: "live", id: 4, title: "Channel" }} tab="find" />
    );
    expect(screen.getByText("Live dialogue is not available yet")).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalled();
    view.rerender(
      <PlayerCompanion {...props} media={{ kind: "live", id: 4, title: "Channel" }} tab="saved" />
    );
    expect(screen.queryByRole("button", { name: /Save 0:/ })).not.toBeInTheDocument();
  });
});
