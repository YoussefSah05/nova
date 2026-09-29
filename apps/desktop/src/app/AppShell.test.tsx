import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { CanonicalMediaSummary, Channel, ChannelGroup, Profile, Source } from "@/types";
import { AppShell } from "./AppShell";

// The browse grid is windowed (@tanstack/react-virtual), rails lazy-mount via
// IntersectionObserver, and the grid sizes columns via ResizeObserver — none of
// which jsdom provides. Stub them so content actually renders: IO reports
// "intersecting" immediately (rails fetch), RO is a no-op, and virtual-core's
// offsetHeight/offsetWidth measurements return a viewport + row height.
const ROW_HEIGHT = 64;
const VIEWPORT_HEIGHT = 800;
const originalOffsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
const originalOffsetWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetWidth");

beforeAll(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      constructor(private cb: IntersectionObserverCallback) {}
      observe(el: Element) {
        this.cb(
          [{ isIntersecting: true, target: el } as IntersectionObserverEntry],
          this as unknown as IntersectionObserver
        );
      }
      unobserve() {}
      disconnect() {}
      takeRecords() {
        return [];
      }
    }
  );
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
    configurable: true,
    get() {
      return 600;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get(this: HTMLElement) {
      return this.dataset?.index !== undefined ? ROW_HEIGHT : VIEWPORT_HEIGHT;
    },
  });
});

afterAll(() => {
  if (originalOffsetHeight) {
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", originalOffsetHeight);
  }
  if (originalOffsetWidth) {
    Object.defineProperty(HTMLElement.prototype, "offsetWidth", originalOffsetWidth);
  }
  vi.unstubAllGlobals();
});

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

type InvokeArgs = Record<string, unknown> | undefined;

const profile: Profile = {
  id: "profile-1",
  name: "Youssef",
  avatarColor: "#4f46e5",
  createdAt: 1,
  lastUsedAt: null,
};

const sources: Source[] = [
  {
    id: 1,
    kind: "m3u",
    name: "Primary",
    displayUrl: "http://example.com",
    lastRefreshedAt: 1,
    autoRefreshHours: 24,
  },
];

function channel(id: number): Channel {
  return {
    id,
    sourceId: 1,
    tvgId: null,
    name: `Channel ${id}`,
    country: "US",
    category: "News",
    rawGroup: "US News",
    logoUrl: `http://example.com/${id}.png`,
    isFavorite: false,
    isInWatchlist: false,
    lastPlayedAt: null,
    addedAt: null,
    dedupGroup: null,
    channelGroup: null,
    region: "MENA",
  };
}

/// Live is grouped server-side now; map a channel to a singleton bundle.
function groupFrom(c: Channel): ChannelGroup {
  return {
    group: c.name.toLowerCase(),
    memberCount: 1,
    representativeId: c.id,
    title: c.name,
    logoUrl: c.logoUrl,
    category: c.category,
    region: c.region,
    isFavorite: c.isFavorite,
    isInWatchlist: c.isInWatchlist,
  };
}

function setupInvoke(
  channelsFor: (args: InvokeArgs) => Channel[],
  movies: CanonicalMediaSummary[] = [],
  sourceList: Source[] = sources
) {
  mocks.invoke.mockImplementation((command: string, args?: InvokeArgs): Promise<unknown> => {
    switch (command) {
      case "list_sources":
        return Promise.resolve(sourceList);
      case "get_trending_in_library":
        return Promise.resolve({ movies: [], series: [] });
      case "get_related_titles":
        return Promise.resolve({ collectionName: null, collection: [], movies: [], series: [] });
      case "get_title_art":
        return Promise.resolve({
          logoUrl: null,
          logoLanguage: null,
          logoWidth: null,
          logoHeight: null,
          backdropUrl: null,
        });
      case "get_title_facts":
        return Promise.resolve(null);
      case "epg_channels_now_and_next":
        return Promise.resolve([]);
      case "count_channels":
        return Promise.resolve(250);
      case "count_movies":
        return Promise.resolve(12);
      case "count_series":
        return Promise.resolve(3);
      case "list_channel_countries":
        return Promise.resolve(["US"]);
      case "list_channel_regions":
        return Promise.resolve(["MENA"]);
      case "list_channel_categories":
        return Promise.resolve(["News"]);
      case "list_vod_categories":
        return Promise.resolve(["Movies"]);
      case "list_series_categories":
        return Promise.resolve(["Series"]);
      case "list_continue_watching":
        return Promise.resolve([]);
      case "list_channel_groups":
        return Promise.resolve(channelsFor(args).map(groupFrom));
      case "semantic_search_channels":
        // Hybrid live search returns brand-family groups; the query is a
        // top-level arg here (not wrapped in `filters`).
        return Promise.resolve((args?.query ? [channel(1)] : []).map(groupFrom));
      case "semantic_search_catalog":
      case "semantic_search_series":
        return Promise.resolve([]);
      case "list_movies":
        return Promise.resolve(movies);
      case "get_movie": {
        const found = movies.find((m) => m.entityId === args?.entityId);
        return found
          ? Promise.resolve({ ...found, variants: [], resume: null })
          : Promise.reject(new Error("movie not found"));
      }
      case "set_movie_favorite":
      case "set_movie_in_watchlist":
        return Promise.resolve();
      case "list_channels":
      case "list_series":
        return Promise.resolve([]);
      default:
        throw new Error(`Unexpected command: ${command}`);
    }
  });
}

function renderShell() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <AppShell profile={profile} onSwitchProfile={vi.fn()} />
    </QueryClientProvider>
  );
}

const reviewMovie: CanonicalMediaSummary = {
  entityId: "mov-7",
  kind: "movie",
  title: "Review Movie",
  sourceTitle: "Review Movie",
  year: 2024,
  category: "Movies",
  posterUrl: null,
  backdropUrl: null,
  overview: null,
  isFavorite: false,
  isInWatchlist: false,
  addedAt: null,
  dedupGroup: null,
};

/// Open Movies, then the movie's page (the first card on the visible screen).
async function openReviewMovie() {
  await userEvent.click(await screen.findByRole("button", { name: "Movies" }));
  const cards = await screen.findAllByRole("button", { name: "Review Movie" });
  await userEvent.click(cards[0]);
  return screen.findByRole("button", { name: "Add Review Movie to favourites" });
}

describe("AppShell", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listen.mockResolvedValue(() => {});
  });

  it("shows each section's count for the selected playlist", async () => {
    setupInvoke(() => [channel(1)]);

    renderShell();

    // Sections live in the top bar; each section's header carries its count.
    await userEvent.click(await screen.findByRole("button", { name: "Live" }));
    expect(await screen.findByText("250 channels")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Movies" }));
    expect(await screen.findByText("12 films")).toBeInTheDocument();
    expect(screen.getByLabelText("Playlist")).toBeInTheDocument();
  });

  it("switches to the search grid and queries with the typed text", async () => {
    setupInvoke((args) => {
      const filters = args?.filters as Record<string, unknown> | undefined;
      // Only the grid query carries a `query`; rails pass sort/favorites/etc.
      return filters?.query ? [channel(1)] : [];
    });

    renderShell();
    await userEvent.click(await screen.findByRole("button", { name: "Live" }));

    // Type a term distinct from the result title so the active-filter chip
    // (which echoes the query) doesn't collide with the card's title text.
    await userEvent.type(screen.getByPlaceholderText("Search live"), "news");

    expect(await screen.findByText("Channel 1")).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "semantic_search_channels",
        expect.objectContaining({ query: "news" })
      )
    );
  });
  it("updates favorite and watchlist controls in an open movie page", async () => {
    setupInvoke(() => [], [reviewMovie]);
    renderShell();
    await userEvent.click(await openReviewMovie());
    expect(
      await screen.findByRole("button", { name: "Remove Review Movie from favourites" })
    ).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: "Add Review Movie to watchlist" }));
    expect(
      await screen.findByRole("button", { name: "Remove Review Movie from watchlist" })
    ).toHaveAttribute("aria-pressed", "true");
    // User state is addressed to the canonical entity, never a provider row.
    expect(mocks.invoke).toHaveBeenCalledWith("set_movie_favorite", {
      profileId: profile.id,
      entityId: "mov-7",
      isFavorite: true,
    });
    expect(mocks.invoke).toHaveBeenCalledWith("set_movie_in_watchlist", {
      profileId: profile.id,
      entityId: "mov-7",
      inWatchlist: true,
    });
    expect(mocks.invoke).toHaveBeenCalledWith("get_movie", {
      profileId: profile.id,
      entityId: "mov-7",
    });
  });

  it("invites a first playlist when there is none", async () => {
    setupInvoke(() => [], [], []);
    renderShell();
    expect(
      await screen.findByRole("heading", { name: "Your library starts with a playlist" })
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add a playlist" })).toBeInTheDocument();
  });

  it("keeps each section's pages while switching, and goes back with Esc", async () => {
    setupInvoke(() => [], [reviewMovie]);
    renderShell();
    await openReviewMovie();

    // Home and back: the movie page is still open in Movies.
    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Add Review Movie to favourites" })).toBeNull()
    );
    await userEvent.click(screen.getByRole("button", { name: "Movies" }));
    expect(
      await screen.findByRole("button", { name: "Add Review Movie to favourites" })
    ).toBeInTheDocument();

    // Esc goes back to the section's root.
    await userEvent.keyboard("{Escape}");
    expect(await screen.findByRole("heading", { name: "Cinémathèque" })).toBeInTheDocument();
    // The page is gone: no Back, neither in the top bar nor on a page.
    expect(screen.queryAllByRole("button", { name: "Back" })).toHaveLength(0);

    // Re-selecting the current section also returns to its root.
    await openReviewMovie();
    expect(screen.getAllByRole("button", { name: "Back" }).length).toBeGreaterThan(0);
    await userEvent.click(screen.getByRole("button", { name: "Movies" }));
    expect(await screen.findByRole("heading", { name: "Cinémathèque" })).toBeInTheDocument();
    expect(screen.queryAllByRole("button", { name: "Back" })).toHaveLength(0);
  });
});
