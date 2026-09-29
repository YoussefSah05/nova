import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import type { Channel, Program } from "@/types";
import { GuideGrid } from "./GuideGrid";
import { guideWindow } from "./timeline";

beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get: () => 800,
  });
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
    configurable: true,
    get: () => 1000,
  });
});
afterEach(() => vi.unstubAllGlobals());

function channel(id: number): Channel {
  return {
    id,
    sourceId: 1,
    tvgId: `c${id}`,
    name: `Channel ${id}`,
    country: null,
    category: "News",
    rawGroup: null,
    logoUrl: null,
    isFavorite: false,
    isInWatchlist: false,
    lastPlayedAt: null,
    addedAt: null,
    dedupGroup: null,
    channelGroup: null,
    region: null,
  };
}

const now = 1_800_000_000;
const onAir: Program = {
  channelTvgId: "c1",
  title: "Evening Bulletin",
  subTitle: null,
  description: null,
  startUtc: now - 600,
  stopUtc: now + 1200,
  category: null,
  iconUrl: null,
};

it("lists channels with what's on, says when there is no guide, and moves with the keys", async () => {
  const onSelect = vi.fn();
  const onPlay = vi.fn();
  const channels = [channel(1), channel(2)];
  render(
    <GuideGrid
      channels={channels}
      programs={
        new Map([
          [1, [onAir]],
          [2, []],
        ])
      }
      window={guideWindow(now)}
      nowSec={now}
      selectedId={1}
      onSelect={onSelect}
      onPlay={onPlay}
      onVisibleRows={() => {}}
      hasMore={false}
      onLoadMore={() => {}}
    />
  );
  const first = screen.getByRole("button", { name: "Channel 1, now Evening Bulletin" });
  expect(screen.getByRole("button", { name: "Channel 2" })).toBeInTheDocument();
  expect(screen.getByText("No guide for this channel")).toBeInTheDocument();

  first.focus();
  await userEvent.keyboard("{ArrowDown}");
  expect(onSelect).toHaveBeenCalledWith(channels[1]);
  await userEvent.keyboard("{Enter}");
  expect(onPlay).toHaveBeenCalledWith(channels[0]);
});
