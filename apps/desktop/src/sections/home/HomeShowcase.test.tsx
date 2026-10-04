import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ScreenActiveContext } from "@/lib/screenActive";
import { HomeShowcase, SHOWCASE_INTERVAL } from "./HomeShowcase";
import { showcaseSlides } from "./showcase";
import type { BrowseItem } from "@/features/browse/queries";
import type { ContinueWatchingItem, SeriesSummary } from "@/types";

const state = vi.hoisted(() => ({ reduced: false, overlay: false }));
vi.mock("@/lib/motion", () => ({ useReducedMotion: () => state.reduced }));
vi.mock("@/app/shell", () => ({ useShell: () => ({ overlayOpen: state.overlay }) }));
vi.mock("./ResumeHero", () => ({
  ResumeHero: ({ spotlight }: { spotlight: BrowseItem }) => <button>{spotlight?.title}</button>,
}));
const movie = (id: number): BrowseItem => ({
  key: `movie-${id}`,
  kind: "movie",
  id,
  title: `Film ${id}`,
  subtitle: null,
  posterUrl: null,
  tvgId: null,
  isFavorite: false,
  isInWatchlist: false,
});
const slides = showcaseSlides(
  null,
  [movie(1), movie(2)],
  [{ ...movie(3), key: "series-3", kind: "series", title: "Series 3" }]
);
let intersection: IntersectionObserverCallback;

beforeEach(() => {
  vi.useFakeTimers();
  state.reduced = false;
  state.overlay = false;
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      constructor(callback: IntersectionObserverCallback) {
        intersection = callback;
      }
      observe() {
        intersection(
          [{ isIntersecting: true } as IntersectionObserverEntry],
          this as unknown as IntersectionObserver
        );
      }
      disconnect() {}
    }
  );
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
const advance = () => act(() => vi.advanceTimersByTime(SHOWCASE_INTERVAL));

describe("Home showcase", () => {
  it("does not repeat a resumed series when history lacks a canonical ID", () => {
    const series: SeriesSummary = {
      sourceId: 1,
      seriesTitle: "raw series",
      displayTitle: "Series",
      entityId: "ser-1",
      category: null,
      episodeCount: 8,
      seasonCount: 1,
      posterUrl: null,
      isFavorite: false,
      isInWatchlist: false,
      addedAt: null,
      canonicalTitle: null,
      backdropUrl: null,
      overview: null,
    };
    const resume: ContinueWatchingItem = {
      mediaKind: "episode",
      mediaId: 20,
      sourceId: 1,
      title: "Series",
      subtitle: null,
      posterUrl: null,
      entityId: null,
      seriesTitle: "raw series",
      positionSec: 40,
      durationSec: 1200,
      watchedAt: 1,
      season: 1,
      episode: 2,
      numbering: null,
      episodeTitle: null,
      upNext: false,
    };
    const result = showcaseSlides(resume, [movie(1)], [{ ...movie(2), kind: "series", series }]);
    expect(result.map((slide) => slide.kind)).toEqual(["resume", "title"]);
  });
  it("alternates real films and series, caps slides and deduplicates", () => {
    expect(slides.map((slide) => slide.title)).toEqual(["Film 1", "Series 3", "Film 2"]);
    expect(
      showcaseSlides(
        null,
        [movie(1), movie(1), movie(2), movie(3), movie(4), movie(5), movie(6)],
        []
      )
    ).toHaveLength(5);
  });
  it("rotates, pauses on hover and focus, and leaves manual selection paused", () => {
    render(<HomeShowcase slides={slides} alsoInProgress={[]} />);
    advance();
    expect(screen.getByRole("button", { name: "Show Series 3" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    fireEvent.pointerEnter(screen.getByRole("region"));
    advance();
    expect(screen.getByRole("button", { name: "Show Series 3" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    fireEvent.pointerLeave(screen.getByRole("region"));
    fireEvent.focus(screen.getByRole("button", { name: "Series 3" }));
    advance();
    expect(screen.getByRole("button", { name: "Show Series 3" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    fireEvent.blur(screen.getByRole("region"), { relatedTarget: document.body });
    fireEvent.click(screen.getByRole("button", { name: "Next showcase title" }));
    advance();
    expect(screen.getByRole("button", { name: "Show Film 2" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    expect(screen.getByRole("button", { name: "Start showcase rotation" })).toBeInTheDocument();
  });
  it("does no rotation behind overlays, inactive screens or offscreen", () => {
    const view = render(
      <ScreenActiveContext.Provider value={false}>
        <HomeShowcase slides={slides} alsoInProgress={[]} />
      </ScreenActiveContext.Provider>
    );
    advance();
    expect(screen.getByRole("button", { name: "Show Film 1" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    state.overlay = true;
    view.rerender(
      <ScreenActiveContext.Provider value={true}>
        <HomeShowcase slides={slides} alsoInProgress={[]} />
      </ScreenActiveContext.Provider>
    );
    advance();
    expect(screen.getByRole("button", { name: "Show Film 1" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    state.overlay = false;
    act(() =>
      intersection(
        [{ isIntersecting: false } as IntersectionObserverEntry],
        {} as IntersectionObserver
      )
    );
    advance();
    expect(screen.getByRole("button", { name: "Show Film 1" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });
  it("respects reduced motion while retaining manual navigation", () => {
    state.reduced = true;
    render(<HomeShowcase slides={slides} alsoInProgress={[]} />);
    advance();
    expect(screen.queryByRole("button", { name: /showcase rotation/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Next showcase title" }));
    expect(screen.getByRole("button", { name: "Show Series 3" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
  });
  it("suspends the timer while the document is hidden", () => {
    render(<HomeShowcase slides={slides} alsoInProgress={[]} />);
    const hidden = Object.getOwnPropertyDescriptor(document, "hidden");
    Object.defineProperty(document, "hidden", { configurable: true, value: true });
    fireEvent(document, new Event("visibilitychange"));
    advance();
    expect(screen.getByRole("button", { name: "Show Film 1" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    if (hidden) Object.defineProperty(document, "hidden", hidden);
    else Reflect.deleteProperty(document, "hidden");
    fireEvent(document, new Event("visibilitychange"));
    advance();
    expect(screen.getByRole("button", { name: "Show Series 3" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
  });
});
