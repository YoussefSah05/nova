import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { BrowseFilters } from "@/features/browse/queries";
import { LibrarySection } from "./LibrarySection";

const state = vi.hoisted(() => ({ source: 1, hasSource: true, grid: vi.fn(), navigate: vi.fn() }));
vi.mock("@/app/session", () => ({
  useSession: () => ({
    profile: { id: "p" },
    activeSourceId: state.source,
    sources: state.hasSource ? [1] : [],
    sourcesLoaded: true,
  }),
}));
vi.mock("@/app/useActivate", () => ({ useActivate: () => vi.fn() }));
vi.mock("@/app/libraryActions", () => ({ useLibraryActions: () => ({}) }));
vi.mock("@/app/navigation/useNav", () => ({ useNavActions: () => ({ switchTo: state.navigate }) }));
vi.mock("@/app/screenContext", () => ({ useScreenScroll: () => ({ current: null }) }));
vi.mock("@/sections/shared/NoPlaylist", () => ({ NoPlaylist: () => <p>Add a playlist</p> }));
vi.mock("./LibraryHistory", () => ({ LibraryHistory: () => <p>History view</p> }));
vi.mock("@/features/browse/BrowseGrid", () => ({
  BrowseGrid: (props: { filters: BrowseFilters; emptyState: ReactNode }) => {
    state.grid(props);
    return <>{props.emptyState}</>;
  },
}));
beforeEach(() => {
  vi.clearAllMocks();
  Element.prototype.scrollIntoView = vi.fn();
  state.source = 1;
  state.hasSource = true;
});

it("switches saved membership, media type and search, then resets controls for a new source", async () => {
  const user = userEvent.setup();
  const { rerender } = render(<LibrarySection />);
  expect(state.grid).toHaveBeenLastCalledWith(
    expect.objectContaining({ filters: expect.objectContaining({ watchlistOnly: true }) })
  );
  await user.click(screen.getByRole("tab", { name: "Favourites" }));
  expect(state.grid).toHaveBeenLastCalledWith(
    expect.objectContaining({
      filters: expect.objectContaining({ favoritesOnly: true, watchlistOnly: false }),
    })
  );
  await user.type(screen.getByRole("textbox", { name: "Search saved titles" }), "Arrival");
  await waitFor(() =>
    expect(state.grid).toHaveBeenLastCalledWith(
      expect.objectContaining({
        filters: expect.objectContaining({ query: "Arrival", favoritesOnly: true }),
      })
    )
  );
  await user.click(screen.getByRole("button", { name: "Clear search" }));
  expect(screen.getByRole("textbox")).toHaveValue("");
  screen.getByRole("combobox", { name: "Saved media type" }).focus();
  await user.keyboard("{ArrowDown}");
  await user.click(screen.getByRole("option", { name: "Shows" }));
  expect(state.grid).toHaveBeenLastCalledWith(expect.objectContaining({ tab: "series" }));
  await user.click(screen.getByRole("button", { name: "Browse shows" }));
  expect(state.navigate).toHaveBeenCalledWith("series");
  screen.getByRole("combobox", { name: "Saved media type" }).focus();
  await user.keyboard("{ArrowDown}");
  await user.click(screen.getByRole("option", { name: "Channels" }));
  expect(screen.queryByRole("combobox", { name: "Saved title order" })).not.toBeInTheDocument();
  expect(screen.getByText("Grouped by channel family")).toBeInTheDocument();
  await user.click(screen.getByRole("tab", { name: "History" }));
  expect(screen.getByText("History view")).toBeInTheDocument();
  state.source = 2;
  rerender(<LibrarySection />);
  expect(screen.getByRole("tab", { name: "Watchlist" })).toHaveAttribute("aria-selected", "true");
  expect(state.grid).toHaveBeenLastCalledWith(
    expect.objectContaining({ sourceId: 2, tab: "movies" })
  );
});
it("offers playlist setup when there is no source", () => {
  state.hasSource = false;
  render(<LibrarySection />);
  expect(screen.getByText("Add a playlist")).toBeInTheDocument();
  expect(state.grid).not.toHaveBeenCalled();
});
