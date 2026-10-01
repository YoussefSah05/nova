import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import { LibraryHistory } from "./LibraryHistory";
import { historyDay } from "./history";
import type { LibraryHistoryItem } from "@/types/generated/LibraryHistoryItem";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  play: vi.fn(),
  navigate: vi.fn(),
  scroll: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/app/playback", () => ({ usePlayback: () => ({ playMedia: mocks.play }) }));
vi.mock("@/app/navigation/useNav", () => ({ useNavActions: () => ({ switchTo: mocks.navigate }) }));
vi.mock("@/app/screenContext", () => ({
  useScreenScroll: () => ({ current: { scrollTo: mocks.scroll } }),
}));

function row(patch: Partial<LibraryHistoryItem> = {}): LibraryHistoryItem {
  return {
    key: "film:1",
    mediaKind: "vod",
    mediaId: 1,
    entityId: "film1",
    title: "Arrival",
    posterUrl: null,
    season: null,
    episode: null,
    episodeTitle: null,
    numbered: false,
    positionSec: 120,
    durationSec: 600,
    watchedAt: 1700000000,
    completed: false,
    ...patch,
  };
}
function mount() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <LibraryHistory profileId="profile" sourceId={7} />
    </QueryClientProvider>
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  Element.prototype.scrollIntoView = vi.fn();
});

it("groups local dates and resumes, replays or tunes the selected media", async () => {
  const user = userEvent.setup();
  mocks.invoke.mockResolvedValue([
    row(),
    row({
      key: "episode:1",
      title: "Dark",
      mediaKind: "episode",
      mediaId: 2,
      completed: true,
      season: 1,
      episode: 2,
      numbered: true,
    }),
    row({ key: "live:1", title: "News", mediaKind: "live", mediaId: 3, watchedAt: 1600000000 }),
    row({ key: "missing", title: "Missing", mediaId: null }),
  ]);
  mount();
  expect(await screen.findByRole("region", { name: historyDay(1700000000) })).toBeInTheDocument();
  expect(screen.getByRole("region", { name: historyDay(1600000000) })).toBeInTheDocument();
  expect(screen.getByText("Completed")).toBeInTheDocument();
  expect(screen.getByText("Unavailable in this playlist")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Missing/ })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /Resume Arrival/ }));
  expect(mocks.play).toHaveBeenLastCalledWith(
    expect.objectContaining({ kind: "vod", id: 1, startSec: 120 })
  );
  await user.click(screen.getByRole("button", { name: /Watch again Dark/ }));
  expect(mocks.play).toHaveBeenLastCalledWith(
    expect.objectContaining({ kind: "episode", id: 2, startSec: 0 })
  );
  await user.click(screen.getByRole("button", { name: /Watch live News/ }));
  expect(mocks.play).toHaveBeenLastCalledWith(
    expect.objectContaining({ kind: "live", id: 3, startSec: undefined })
  );
  expect(mocks.invoke).toHaveBeenCalledWith("list_library_history", {
    profileId: "profile",
    sourceId: 7,
    kind: null,
    limit: 51,
    offset: 0,
  });
});

it("bounds pages with lookahead and resets paging when the kind changes", async () => {
  const user = userEvent.setup();
  mocks.invoke.mockImplementation((_command, args: { offset: number; kind: string | null }) =>
    Promise.resolve(
      args.offset || args.kind
        ? [row({ title: "Last" })]
        : Array.from({ length: 51 }, (_, i) => row({ key: String(i), title: `Film ${i}` }))
    )
  );
  mount();
  await screen.findByText("Film 49");
  expect(screen.queryByText("Film 50")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Next page" }));
  await screen.findByText("Last");
  expect(mocks.invoke).toHaveBeenLastCalledWith(
    "list_library_history",
    expect.objectContaining({ offset: 50 })
  );
  expect(screen.getByRole("button", { name: "Next page" })).toBeDisabled();
  screen.getByRole("combobox", { name: "History media type" }).focus();
  await user.keyboard("{ArrowDown}");
  await user.click(screen.getByRole("option", { name: "Episodes" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "list_library_history",
      expect.objectContaining({ offset: 0, kind: "episode" })
    )
  );
});

it("offers a retry on errors and a browse action for empty history", async () => {
  const user = userEvent.setup();
  mocks.invoke.mockRejectedValueOnce(new Error("failed")).mockResolvedValue([]);
  mount();
  await screen.findByRole("alert");
  await user.click(screen.getByRole("button", { name: "Try again" }));
  await user.click(await screen.findByRole("button", { name: "Find something to watch" }));
  expect(mocks.navigate).toHaveBeenCalledWith("movies");
});
