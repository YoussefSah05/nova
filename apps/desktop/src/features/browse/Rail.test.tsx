import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { Rail } from "./Rail";

const state = vi.hoisted(() => ({ reduced: false }));
vi.mock("@/lib/motion", () => ({ useReducedMotion: () => state.reduced }));
vi.mock("./queries", () => ({
  useRail: () => ({ data: [{ key: "film" }], isLoading: false }),
}));
vi.mock("./PosterCard", () => ({
  PosterCard: () => <span>A film</span>,
  PosterCardSkeleton: () => null,
}));
afterEach(() => {
  cleanup();
  state.reduced = false;
});

it.each([false, true])("retains rail controls with reduced motion set to %s", (reduced) => {
  state.reduced = reduced;
  const view = render(
    <Rail
      tab="movies"
      profileId="p"
      sourceId={1}
      eager
      spec={{ id: "new", title: "New", filters: {} }}
      onActivate={() => {}}
      onToggleFavorite={() => {}}
      onToggleWatchlist={() => {}}
    />
  );
  const track = view.container.querySelector(".overflow-x-auto") as HTMLElement;
  Object.defineProperty(track, "clientWidth", { value: 1000 });
  const scrollBy = vi.fn();
  track.scrollBy = scrollBy;
  fireEvent.click(screen.getByRole("button", { name: "Scroll right" }));
  fireEvent.click(screen.getByRole("button", { name: "Scroll left" }));
  expect(scrollBy.mock.calls).toEqual([
    [{ left: 850, behavior: reduced ? "auto" : "smooth" }],
    [{ left: -850, behavior: reduced ? "auto" : "smooth" }],
  ]);
});
