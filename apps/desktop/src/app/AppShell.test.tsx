import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  CanonicalMediaSummary,
  Channel,
  ChannelGroup,
  ContinueWatchingItem,
  Profile,
  SeriesSummary,
  Source,
} from "@/types";
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

/// Set by a test to make the next favourite toggles fail in Rust.
let favoriteFails = false;
/// Set by a test: a show in progress with a new episode airing today.
let showsFixture = false;

function today(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

const breakingBad: SeriesSummary = {
  seriesTitle: "BREAKING BAD",
  sourceId: 1,
  category: "Drama",
  episodeCount: 10,
  seasonCount: 2,
  posterUrl: null,
  isFavorite: false,
  isInWatchlist: false,
  addedAt: 1,
  canonicalTitle: "Breaking Bad",
  backdropUrl: null,
  overview: null,
  displayTitle: "Breaking Bad",
  entityId: "ser-1",
  genres: ["Drama", "Crime"],
  status: "Returning Series",
  firstYear: 2008,
  nextEpisode: { season: 3, episode: 1, name: null, airDate: today() },
  watchedCount: 3,
  lastWatchedAt: 1,
};

const bbInProgress: ContinueWatchingItem = {
  mediaKind: "episode",
  mediaId: 41,
  sourceId: 1,
  title: "Breaking Bad",
  subtitle: null,
  posterUrl: null,
  positionSec: 0,
  durationSec: null,
  watchedAt: 1,
  entityId: "ser-1",
  seriesTitle: "BREAKING BAD",
  season: 2,
  episode: 1,
  numbering: "seasonEpisode",
  episodeTitle: null,
  upNext: true,
};

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
      case "list_movie_genres":
        return Promise.resolve([{ name: "Drama", count: 12 }]);
      case "list_movie_decades":
        return Promise.resolve([{ decade: 2020, count: 5, posterUrls: [] }]);
      case "list_library_collections":
        return Promise.resolve([]);
      case "count_channels":
        return Promise.resolve(250);
      case "list_live_overview":
        return Promise.resolve({
          channels: 250,
          withGuide: 180,
          categories: [{ name: "News", count: 250 }],
        });
      case "search_movies":
        return Promise.resolve(movies);
      case "search_moments":
        return Promise.resolve([
          {
            segmentId: 9,
            startSec: 3723,
            endSec: 3726,
            text: "Don't let yourself get attached",
            kind: "source",
            mediaKind: "vod",
            mediaId: 7,
            title: "Review Movie",
            season: null,
            episode: null,
            episodeTitle: null,
            numbered: false,
            posterUrl: null,
            entityId: "mov-7",
          },
        ]);
      case "get_enrich_model_status":
        return Promise.resolve({ installed: false, modelId: "e5", availability: "missing" });
      case "epg_refresh_if_stale":
        return Promise.resolve(false);
      case "epg_guide": {
        const now = Math.floor(Date.now() / 1000);
        return Promise.resolve(
          (args?.channelIds as number[]).map((channelId) => ({
            channelId,
            programs:
              channelId === 1
                ? [
                    {
                      channelTvgId: "c1",
                      title: "Evening Bulletin",
                      subTitle: null,
                      description: "The day's news.",
                      startUtc: now - 600,
                      stopUtc: now + 1200,
                      category: null,
                      iconUrl: null,
                    },
                  ]
                : [],
          }))
        );
      }
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
        return Promise.resolve(showsFixture && args?.section === "series" ? [bbInProgress] : []);
      case "list_series_genres":
        return Promise.resolve([{ name: "Drama", count: 1 }]);
      case "list_series_schedule":
      case "list_series_cards":
        return Promise.resolve(showsFixture ? [breakingBad] : []);
      case "list_series_episodes":
      case "get_season_episodes":
        return Promise.resolve([]);
      case "get_series_resume":
        return Promise.resolve(null);
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
          ? Promise.resolve({
              ...found,
              variants: [
                {
                  variantId: "var-fhd",
                  sourceId: 1,
                  sourceName: "Primary",
                  label: "Review Movie FHD",
                  available: true,
                  badges: ["FHD"],
                  recentlyFailed: false,
                  picked: true,
                },
                {
                  variantId: "var-4k",
                  sourceId: 1,
                  sourceName: "Backup",
                  label: "Review Movie 4K",
                  available: true,
                  badges: ["4K"],
                  recentlyFailed: false,
                  picked: false,
                },
              ],
              resume: null,
              originalTitle: null,
              releaseDate: null,
              voteCount: null,
              keywords: [],
            })
          : Promise.reject(new Error("movie not found"));
      }
      case "resolve_movie_playback":
        // The player itself is out of scope here: stop at the resolution.
        return Promise.reject(new Error("no stream in tests"));
      case "get_profile_settings":
        return Promise.resolve({});
      case "set_movie_favorite":
        return favoriteFails ? Promise.reject(new Error("database is locked")) : Promise.resolve();
      case "set_movie_in_watchlist":
        return Promise.resolve();
      case "list_series":
        return Promise.resolve(showsFixture ? [breakingBad] : []);
      case "list_channels":
        return Promise.resolve(channelsFor(args));
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
  genres: [],
  runtimeMin: null,
  rating: null,
  ratingSource: null,
  badges: [],
  versionCount: 1,
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
    favoriteFails = false;
    showsFixture = false;
    mocks.listen.mockResolvedValue(() => {});
  });

  it("shows each section's count for the selected playlist", async () => {
    setupInvoke(() => [channel(1)]);

    renderShell();

    // Sections live in the top bar; each section's header carries its count.
    await userEvent.click(await screen.findByRole("button", { name: "Live" }));
    expect(await screen.findByText("250 channels · guide data for 180")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Movies" }));
    expect(await screen.findByText("12 films")).toBeInTheDocument();
    expect(screen.getByLabelText("Playlist")).toBeInTheDocument();
  });

  it("searches the live guide and shows the chosen channel's programme", async () => {
    setupInvoke((args) => {
      const filters = args?.filters as Record<string, unknown> | undefined;
      return filters?.query ? [channel(1)] : [channel(2)];
    });

    renderShell();
    await userEvent.click(await screen.findByRole("button", { name: "Live" }));
    await userEvent.type(screen.getByPlaceholderText("Search channels"), "news");

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "list_channels",
        expect.objectContaining({ filters: expect.objectContaining({ query: "news" }) })
      )
    );
    // The first channel is chosen: its current programme shows beside the guide.
    expect(await screen.findByRole("heading", { name: "Evening Bulletin" })).toBeInTheDocument();
    expect(screen.getByText("The day's news.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Watch" })).toBeInTheDocument();

    // The wall shows the same slice as brand bundles.
    await userEvent.click(screen.getByRole("button", { name: "Wall" }));
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

  it("keeps a section's screen, scroll included, when another is first visited later", async () => {
    setupInvoke(() => [], [reviewMovie]);
    renderShell();
    await userEvent.click(await screen.findByRole("button", { name: "Library" }));
    const library = await screen.findByRole("region", { name: "Library" });
    library.scrollTop = 321;
    library.dispatchEvent(new Event("scroll"));

    // Movies comes before Library in the section order: visiting it later
    // must not shift (and so remount) Library's screen.
    await userEvent.click(screen.getByRole("button", { name: "Movies" }));
    await screen.findByRole("heading", { name: "Cinémathèque" });
    await userEvent.click(screen.getByRole("button", { name: "Library" }));
    const again = await screen.findByRole("region", { name: "Library" });
    expect(again).toBe(library);
    expect(again.scrollTop).toBe(321);
  });

  it("rolls a failed favourite back on the open page and says why", async () => {
    setupInvoke(() => [], [reviewMovie]);
    favoriteFails = true;
    renderShell();
    await userEvent.click(await openReviewMovie());
    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");
    expect(
      await screen.findByRole("button", { name: "Add Review Movie to favourites" })
    ).toHaveAttribute("aria-pressed", "false");
  });

  it("narrows the wall by decade and plays the source chosen on a film's page", async () => {
    setupInvoke(() => [], [reviewMovie]);
    renderShell();
    await userEvent.click(await screen.findByRole("button", { name: "Movies" }));
    await userEvent.click(await screen.findByRole("button", { name: /2020s/ }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "list_movies",
        expect.objectContaining({
          filters: expect.objectContaining({ yearFrom: 2020, yearTo: 2029 }),
        })
      )
    );
    await userEvent.click(screen.getByRole("button", { name: "Clear Decade filter" }));
    expect(await screen.findByRole("heading", { name: "Every film" })).toBeInTheDocument();

    const cards = await screen.findAllByRole("button", { name: "Review Movie" });
    await userEvent.click(cards[0]);
    const backup = await screen.findByRole("button", { name: /^Play Backup 4K/ });
    expect(
      screen.getByRole("button", { name: /^Play Primary FHD \(Nova's pick\)/ })
    ).toBeInTheDocument();
    await userEvent.click(backup);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("resolve_movie_playback", {
        profileId: profile.id,
        entityId: "mov-7",
        variantId: "var-4k",
      })
    );
  });

  it("leads Shows with the shows in progress and the week's new episodes", async () => {
    showsFixture = true;
    setupInvoke(() => []);
    renderShell();
    await userEvent.click(await screen.findByRole("button", { name: "Series" }));

    expect(await screen.findByRole("heading", { name: "Continue your shows" })).toBeInTheDocument();
    const cont = await screen.findByRole("button", {
      name: "Breaking Bad, S2 · E1 next, 3 of 10 watched",
    });
    expect(screen.getByRole("heading", { name: "This week" })).toBeInTheDocument();
    expect(screen.getByRole("listitem", { name: /^Today/ })).toHaveTextContent("S3 · E1");
    expect(await screen.findByText(/1 with an episode this week/)).toBeInTheDocument();
    // The wall's card names its status from TMDB.
    expect(
      await screen.findByRole("button", { name: /^Breaking Bad, Returning, 2 seasons · 2008–/ })
    ).toBeInTheDocument();

    await userEvent.click(cont);
    // The show's page (the section stays mounted, hidden, underneath).
    await waitFor(() => expect(screen.getAllByText("3 of 10 watched")).toHaveLength(2));
    expect(screen.getByRole("heading", { level: 1, name: "Breaking Bad" })).toBeInTheDocument();
  });

  it("searches everything from the palette, reading a decade and a genre it can undo", async () => {
    // cmdk scrolls the active option into view; jsdom has no layout.
    Element.prototype.scrollIntoView = vi.fn();
    setupInvoke(() => [], [reviewMovie]);
    renderShell();
    await screen.findByRole("button", { name: "Live" });
    await userEvent.keyboard("{Meta>}k{/Meta}");
    await userEvent.type(
      await screen.findByPlaceholderText("Search content or jump to…"),
      "drama 90s"
    );
    await userEvent.click(
      await screen.findByRole("option", { name: "Search everything for “drama 90s”" })
    );

    const understood = await screen.findByRole("group", { name: "Understood as" });
    expect(understood).toHaveTextContent("GenreDrama");
    expect(understood).toHaveTextContent("Decade1990s");
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "list_movies",
        expect.objectContaining({
          filters: expect.objectContaining({ genre: "Drama", yearFrom: 1990, yearTo: 1999 }),
        })
      )
    );
    expect(screen.getByText("Meaning-based search is off")).toBeInTheDocument();

    // Undo the decade: "90s" is searched as words, in films and dialogue.
    await userEvent.click(screen.getByRole("button", { name: "Search “1990s” as words instead" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "search_moments",
        expect.objectContaining({ query: "90s" })
      )
    );
    expect(
      await screen.findByRole("button", {
        name: "Play Review Movie · 1:02:03: “Don't let yourself get attached”",
      })
    ).toBeInTheDocument();
  });
});
