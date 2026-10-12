import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { GridContextMenu } from "./CardMenu";
import type { BrowseItem } from "./queries";

const item = (key: string, title: string, kind: BrowseItem["kind"] = "movie") =>
  ({ key, kind, id: 1, title, isFavorite: false, isInWatchlist: false }) as BrowseItem;

function mount() {
  const handlers = { onActivate: vi.fn(), onToggleFavorite: vi.fn(), onToggleWatchlist: vi.fn() };
  const items = [item("m-1", "Dune"), item("live-2", "News", "live")];
  render(
    <GridContextMenu items={items} {...handlers}>
      <div data-card-key="m-1">Dune card</div>
      <div data-card-key="live-2">News card</div>
      <div>Gap</div>
    </GridContextMenu>
  );
  return { handlers, items };
}

it("opens one shared menu for the right-clicked card", async () => {
  const user = userEvent.setup();
  const { handlers, items } = mount();
  fireEvent.contextMenu(screen.getByText("News card"));
  await user.click(await screen.findByRole("menuitem", { name: "Play" }));
  expect(handlers.onActivate).toHaveBeenCalledWith(items[1]);
  fireEvent.contextMenu(screen.getByText("Dune card"));
  await user.click(await screen.findByRole("menuitem", { name: "Add to Watchlist" }));
  expect(handlers.onToggleWatchlist).toHaveBeenCalledWith(items[0], true);
});

it("shows no menu outside a card", () => {
  mount();
  fireEvent.contextMenu(screen.getByText("Gap"));
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
});
