import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { ChannelNowAndNext, EpgFeedStatus, Program } from "@/types";
import { describeGuide } from "@/features/settings/sections/guideStatus";
import { msUntilGuideChange, useChannelGuide } from "./guide";
import { GuideProvider } from "./GuideViews";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  handlers: [] as ((event: { payload: unknown }) => void)[],
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_name: string, handler: (event: { payload: unknown }) => void) => {
    mocks.handlers.push(handler);
    return () => undefined;
  }),
}));

function programme(title: string, startUtc: number, stopUtc: number): Program {
  return {
    channelTvgId: "cnn.us",
    title,
    subTitle: null,
    description: null,
    startUtc,
    stopUtc,
    category: null,
    iconUrl: null,
  };
}

function wrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>
      <GuideProvider profileId="p">{children}</GuideProvider>
    </QueryClientProvider>
  );
}

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.handlers.length = 0;
});

it("coalesces the channels on screen into one guide request", async () => {
  const now = Math.floor(Date.now() / 1000);
  mocks.invoke.mockImplementation(async (_cmd: string, args: { channelIds: number[] }) =>
    args.channelIds
      .filter((id) => id !== 3)
      .map(
        (id): ChannelNowAndNext => ({
          channelId: id,
          now: programme(`show ${id}`, now - 60, now + 600),
          next: null,
        })
      )
  );
  const { result } = renderHook(() => [1, 2, 3].map((id) => useChannelGuide(id)), {
    wrapper: wrapper(),
  });
  await waitFor(() => expect(result.current.every((g) => g !== undefined)).toBe(true));
  expect(mocks.invoke).toHaveBeenCalledTimes(1);
  expect(mocks.invoke).toHaveBeenCalledWith("epg_channels_now_and_next", {
    profileId: "p",
    channelIds: [1, 2, 3],
  });
  expect(result.current.map((g) => g?.now?.title ?? null)).toEqual(["show 1", "show 2", null]);
});

it("refetches a channel's guide when a guide refresh job finishes", async () => {
  mocks.invoke.mockResolvedValueOnce([{ channelId: 7, now: null, next: null }]);
  const { result } = renderHook(() => useChannelGuide(7), { wrapper: wrapper() });
  await waitFor(() => expect(result.current).toBeDefined());
  expect(result.current?.now).toBeNull();

  const now = Math.floor(Date.now() / 1000);
  mocks.invoke.mockResolvedValueOnce([
    { channelId: 7, now: programme("News", now - 60, now + 600), next: null },
  ]);
  act(() => mocks.handlers.forEach((h) => h({ payload: { sourceId: 1, ok: true } })));
  await waitFor(() => expect(result.current?.now?.title).toBe("News"));
  expect(mocks.invoke).toHaveBeenCalledTimes(2);
});

it("never shows a channel's guide on another item with the same id", async () => {
  const now = Math.floor(Date.now() / 1000);
  mocks.invoke.mockResolvedValue([
    { channelId: 1, now: programme("Channel 1 show", now - 60, now + 600), next: null },
  ]);
  const { result } = renderHook(
    () => ({
      channel: useChannelGuide(1),
      // A series card from source 1, or a bundle whose representative is 1.
      other: useChannelGuide(1, { enabled: false }),
    }),
    { wrapper: wrapper() }
  );
  await waitFor(() => expect(result.current.channel?.now?.title).toBe("Channel 1 show"));
  expect(result.current.other).toBeUndefined();
  expect(mocks.invoke).toHaveBeenCalledTimes(1);
});

it("a failed guide request fails every channel waiting on it", async () => {
  mocks.invoke.mockRejectedValueOnce(new Error("profile closed"));
  const { result } = renderHook(() => [4, 5].map((id) => useChannelGuide(id)), {
    wrapper: wrapper(),
  });
  await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(1));
  // Neither query hangs: both settle (as errors) and show no guide.
  await new Promise((r) => setTimeout(r, 50));
  expect(result.current).toEqual([undefined, undefined]);
});

it("checks again when the current programme ends", () => {
  const nowMs = 1_000_000_000;
  const at = nowMs / 1000;
  const guide = (now: Program | null, next: Program | null): ChannelNowAndNext => ({
    channelId: 1,
    now,
    next,
  });
  expect(msUntilGuideChange(guide(programme("a", at - 60, at + 120), null), nowMs)).toBe(121_000);
  expect(msUntilGuideChange(guide(null, programme("b", at + 30, at + 90)), nowMs)).toBe(31_000);
  expect(msUntilGuideChange(guide(programme("a", at - 60, at), null), nowMs)).toBe(5_000);
  expect(msUntilGuideChange(guide(null, null), nowMs)).toBe(900_000);
  expect(msUntilGuideChange(guide(programme("a", at, at + 86_400), null), nowMs)).toBe(900_000);
});

it("summarises a source's guide feeds in plain words", () => {
  const nowMs = 1_000_000_000_000;
  const feed = (over: Partial<EpgFeedStatus>): EpgFeedStatus => ({
    id: 1,
    kind: "xtream",
    lastRefreshUtc: null,
    lastError: null,
    programmeCount: 0,
    ...over,
  });
  expect(describeGuide([], nowMs)).toMatch(/No guide feed/);
  expect(describeGuide([feed({})], nowMs)).toBe("No programmes yet");
  expect(
    describeGuide([feed({ programmeCount: 1200, lastRefreshUtc: nowMs / 1000 - 7200 })], nowMs)
  ).toBe("1,200 programmes from 1 feed · updated 2h ago");
  expect(
    describeGuide(
      [
        feed({ programmeCount: 10, lastRefreshUtc: nowMs / 1000 - 30 }),
        feed({ id: 2, kind: "community", lastError: "EPG feed contained no programmes" }),
      ],
      nowMs
    )
  ).toBe(
    "10 programmes from 2 feeds · updated just now · 1 of 2 feeds failed: EPG feed contained no programmes"
  );
});
