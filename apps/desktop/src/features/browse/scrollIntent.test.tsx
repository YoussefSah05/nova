import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { GuideProfile } from "./guide";
import { useHoverPreview } from "./HoverPreview";
import { usePrefetchTitle } from "./prefetch";
import type { BrowseItem } from "./queries";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
const movie: BrowseItem = {
  key: "movie-m1",
  kind: "movie",
  id: 0,
  title: "Heat",
  subtitle: null,
  posterUrl: null,
  tvgId: null,
  isFavorite: false,
  isInWatchlist: false,
  entityId: "m1",
};

function Card() {
  const preview = useHoverPreview();
  const prefetch = usePrefetchTitle(movie);
  return (
    <button
      onMouseEnter={(event) => {
        preview.onEnter(event);
        prefetch.onEnter(event);
      }}
      onMouseLeave={() => {
        preview.onLeave();
        prefetch.onLeave();
      }}
    >
      {preview.open ? "Preview" : "Title"}
    </button>
  );
}
function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <GuideProfile.Provider value="p">
        <section data-testid="screen">
          <Card />
        </section>
      </GuideProfile.Provider>
    </QueryClientProvider>
  );
  return { ...view, container: screen.getByTestId("screen") };
}
beforeEach(() => {
  vi.useFakeTimers();
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue({});
  vi.spyOn(performance, "now").mockReturnValue(10_000);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

it("starts no preview or metadata request for a pointer on a scrolling screen", () => {
  const { container } = setup();
  container.classList.add("nova-scrolling");
  fireEvent.mouseEnter(screen.getByRole("button"));
  act(() => vi.advanceTimersByTime(500));
  expect(screen.getByRole("button")).toHaveTextContent("Title");
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it("cancels pending intent when scrolling starts, then permits intentional hover", () => {
  const { container } = setup();
  fireEvent.mouseEnter(screen.getByRole("button"));
  container.classList.add("nova-scrolling");
  act(() => vi.advanceTimersByTime(500));
  expect(mocks.invoke).not.toHaveBeenCalled();
  expect(screen.getByRole("button")).toHaveTextContent("Title");
  container.classList.remove("nova-scrolling");
  fireEvent.mouseLeave(screen.getByRole("button"));
  fireEvent.mouseEnter(screen.getByRole("button"));
  act(() => vi.advanceTimersByTime(500));
  expect(mocks.invoke).toHaveBeenCalledTimes(3);
  expect(screen.getByRole("button")).toHaveTextContent("Preview");
  fireEvent(container, new Event("scroll", { bubbles: false }));
  expect(screen.getByRole("button")).toHaveTextContent("Title");
});
