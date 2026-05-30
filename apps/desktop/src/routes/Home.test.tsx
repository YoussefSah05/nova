import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Channel, Profile, SeriesSummary, Source, VodItem } from "@/types";
import { Home } from "./Home";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

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
    url: "http://example.com/one.m3u",
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
    logoUrl: null,
    streamUrl: `http://example.com/${id}.m3u8`,
    isFavorite: false,
    isInWatchlist: false,
    lastPlayedAt: null,
    addedAt: null,
  };
}

function setupInvoke(totalChannels: number, firstPage: Channel[], nextPage: Channel[] = []) {
  mocks.invoke.mockImplementation((command: string, args?: InvokeArgs): Promise<unknown> => {
    switch (command) {
      case "list_sources":
        return Promise.resolve(sources);
      case "count_channels":
        return Promise.resolve(totalChannels);
      case "count_vod":
        return Promise.resolve(12);
      case "count_series":
        return Promise.resolve(3);
      case "list_channel_countries":
        return Promise.resolve(["US"]);
      case "list_channel_categories":
        return Promise.resolve(["News"]);
      case "list_vod_categories":
        return Promise.resolve(["Movies"]);
      case "list_series_categories":
        return Promise.resolve(["Series"]);
      case "list_channels":
        if (isShelfRequest(args)) return Promise.resolve([] satisfies Channel[]);
        return Promise.resolve(numberArg(args, "offset") === 0 ? firstPage : nextPage);
      case "list_vod":
        return Promise.resolve([] satisfies VodItem[]);
      case "list_series":
        return Promise.resolve([] satisfies SeriesSummary[]);
      case "list_continue_watching":
        return Promise.resolve([]);
      default:
        throw new Error(`Unexpected command: ${command}`);
    }
  });
}

function numberArg(args: InvokeArgs, key: string): number {
  const value = args?.[key];
  if (typeof value !== "number") {
    throw new Error(`Expected numeric arg: ${key}`);
  }
  return value;
}

function isShelfRequest(args: InvokeArgs): boolean {
  const filters = args?.filters as Record<string, unknown> | undefined;
  if (!filters) return false;
  return (
    filters.sort === "recentlyAdded" ||
    filters.favoritesOnly === true ||
    filters.watchlistOnly === true
  );
}

describe("Home", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Refresh progress listener — resolve to a no-op unlisten fn.
    mocks.listen.mockResolvedValue(() => {});
  });

  it("shows imported counts and the first page for the selected source", async () => {
    setupInvoke(250, [channel(1), channel(2)]);

    render(<Home profile={profile} onSwitchProfile={vi.fn()} />);

    expect(await screen.findByText("Channel 1")).toBeInTheDocument();
    expect(screen.getByLabelText("Playlist")).toHaveValue("1");
    expect(screen.getByRole("button", { name: /live\s+250/i })).toBeInTheDocument();
    expect(screen.getByText("Showing 2 of 250 live")).toBeInTheDocument();
  });

  it("loads the next live page with the current item count as the offset", async () => {
    const firstPage = Array.from({ length: 200 }, (_, index) => channel(index + 1));
    const nextPage = [channel(201)];
    setupInvoke(201, firstPage, nextPage);

    render(<Home profile={profile} onSwitchProfile={vi.fn()} />);

    await screen.findByText("Channel 200");
    await userEvent.click(screen.getByRole("button", { name: /load 1 more/i }));

    expect(await screen.findByText("Channel 201")).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "list_channels",
        expect.objectContaining({ offset: 200 })
      )
    );
  });
});
