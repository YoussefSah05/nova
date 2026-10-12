import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import type { LiveChannelSummary } from "@/types";
import { LiveBundleDetail } from "./LiveBundleDetail";
import { channelGroupToItem } from "./queries";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("./guide", () => ({ useChannelGuide: () => null }));
vi.mock("./GuideViews", () => ({ GuideLine: () => null }));
afterEach(() => {
  cleanup();
  mocks.invoke.mockReset();
});
export function liveFixture(
  key: string,
  edition: string,
  title = "beIN Sports 1"
): LiveChannelSummary {
  return {
    key,
    sourceId: 1,
    family: "bein sports",
    title,
    edition,
    language: edition,
    country: null,
    number: 1,
    representativeId: key === "fr" ? 1 : 2,
    logoUrl: null,
    category: "Sports",
    region: null,
    isFavorite: false,
    isInWatchlist: false,
    variants: [
      {
        variantKey: key + "-hd",
        channelId: key === "fr" ? 1 : 2,
        rawName: `${edition} - ${title} HD`,
        quality: "HD",
        codecs: [],
        recentlyFailed: false,
        lastSuccessAt: null,
      },
    ],
  };
}
const item = channelGroupToItem({
  group: "bein sports",
  title: "beIN Sports",
  memberCount: 2,
  streamCount: 3,
  editions: ["EN", "FR"],
  representativeId: 1,
  logoUrl: null,
  category: "Sports",
  region: null,
  isFavorite: false,
  isInWatchlist: false,
});
it("offers only present editions, shows qualities, and plays a logical channel", async () => {
  const fr = liveFixture("fr", "FR"),
    en = liveFixture("en", "EN");
  fr.variants.push({
    ...fr.variants[0],
    variantKey: "fr-sd",
    channelId: 3,
    quality: "SD",
    rawName: "FR - BEINSPORT 1 SD",
  });
  mocks.invoke.mockResolvedValue([fr, en]);
  const play = vi.fn();
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <LiveBundleDetail profileId="p" sourceId={1} item={item} onBack={vi.fn()} onPlay={play} />
    </QueryClientProvider>
  );
  await screen.findByRole("button", { name: "EN" });
  expect(screen.queryByRole("button", { name: "AR" })).not.toBeInTheDocument();
  expect(screen.getByText("2 channels · 3 streams")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "FR" }));
  expect(screen.getByText("FR · HD / SD · 2 streams")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: /beIN Sports 1 FR/ }));
  expect(play.mock.calls[0][0].liveChannel.key).toBe("fr");
});
it("shows an actionable retry instead of an empty successful bundle on failure", async () => {
  mocks.invoke.mockRejectedValue(new Error("Probe failed"));
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <LiveBundleDetail profileId="p" sourceId={1} item={item} onBack={vi.fn()} onPlay={vi.fn()} />
    </QueryClientProvider>
  );
  expect(await screen.findByRole("button", { name: "Retry" })).toBeInTheDocument();
});
