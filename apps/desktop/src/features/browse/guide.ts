//! Live programme guide on screen: what's on now and next for a channel.
//!
//! Every card, preview and the player ask for one channel with
//! `useChannelGuide`; requests made in the same moment are coalesced into one
//! `epg_channels_now_and_next` call, cached per channel by TanStack Query,
//! refetched when the current programme ends, and invalidated when a guide
//! refresh finishes. Rust picks each channel's guide (its own source's feed).

import { createContext, useContext, useSyncExternalStore } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

import type { ChannelNowAndNext, Program } from "@/types";

/// Requests made within this window share one IPC call.
const BATCH_WINDOW_MS = 16;
/// Re-check a channel with no current programme this often (a finished guide
/// refresh also invalidates it).
const IDLE_REFETCH_MS = 15 * 60_000;
/// Channels per IPC call (a grid page can hold a few hundred cards).
const MAX_BATCH = 100;

export const guideKeys = {
  all: ["guide"] as const,
  channel: (profileId: string, channelId: number) =>
    [...guideKeys.all, profileId, channelId] as const,
};

/// The active profile, for catalog cards (set by `GuideProvider`).
export const GuideProfile = createContext<string | null>(null);

// ── Batched loader ────────────────────────────────────────────────────────────

interface Waiter {
  resolve: (guide: ChannelNowAndNext) => void;
  reject: (error: unknown) => void;
}

const pending = new Map<string, Map<number, Waiter[]>>();

function loadGuide(profileId: string, channelId: number): Promise<ChannelNowAndNext> {
  return new Promise((resolve, reject) => {
    let batch = pending.get(profileId);
    if (!batch) {
      batch = new Map();
      pending.set(profileId, batch);
      setTimeout(() => void flush(profileId), BATCH_WINDOW_MS);
    }
    const waiters = batch.get(channelId) ?? [];
    waiters.push({ resolve, reject });
    batch.set(channelId, waiters);
  });
}

async function flush(profileId: string) {
  const batch = pending.get(profileId);
  pending.delete(profileId);
  if (!batch) return;
  const ids = [...batch.keys()];
  for (let i = 0; i < ids.length; i += MAX_BATCH) {
    const chunk = ids.slice(i, i + MAX_BATCH);
    try {
      const guides = await invoke<ChannelNowAndNext[]>("epg_channels_now_and_next", {
        profileId,
        channelIds: chunk,
      });
      const byId = new Map(guides.map((g) => [g.channelId, g]));
      for (const channelId of chunk) {
        const guide = byId.get(channelId) ?? { channelId, now: null, next: null };
        for (const w of batch.get(channelId) ?? []) w.resolve(guide);
      }
    } catch (error) {
      for (const channelId of chunk) for (const w of batch.get(channelId) ?? []) w.reject(error);
    }
  }
}

/// When the guide shown for a channel next changes: the current programme
/// ends or the next one starts.
export function msUntilGuideChange(
  guide: ChannelNowAndNext | undefined,
  nowMs: number = Date.now()
): number {
  const edge = guide?.now?.stopUtc ?? guide?.next?.startUtc;
  if (edge == null) return IDLE_REFETCH_MS;
  return Math.min(IDLE_REFETCH_MS, Math.max(5_000, edge * 1000 - nowMs + 1_000));
}

/// What's on now and next on a channel row. `profileId` defaults to the
/// surrounding `GuideProvider`'s. With `enabled: false` (not a channel, or a
/// bundle of several) it is always `undefined` — never another item's cached
/// guide under the same id.
export function useChannelGuide(
  channelId: number | null | undefined,
  options: { profileId?: string; enabled?: boolean } = {}
): ChannelNowAndNext | undefined {
  const contextProfile = useContext(GuideProfile);
  const profileId = options.profileId ?? contextProfile;
  const active = profileId != null && channelId != null && (options.enabled ?? true);
  const { data } = useQuery({
    queryKey: guideKeys.channel(profileId ?? "", active ? channelId : -1),
    queryFn: () => loadGuide(profileId!, channelId!),
    enabled: active,
    staleTime: 60_000,
    refetchInterval: (query) => msUntilGuideChange(query.state.data),
    retry: false,
  });
  return active ? data : undefined;
}

// ── Shared minute clock (progress bars) ───────────────────────────────────────

let minute = Math.floor(Date.now() / 60_000);
const clockListeners = new Set<() => void>();
let clockTimer: ReturnType<typeof setInterval> | undefined;

function subscribeClock(listener: () => void) {
  clockListeners.add(listener);
  clockTimer ??= setInterval(() => {
    minute = Math.floor(Date.now() / 60_000);
    for (const l of clockListeners) l();
  }, 15_000);
  return () => {
    clockListeners.delete(listener);
    if (clockListeners.size === 0 && clockTimer !== undefined) {
      clearInterval(clockTimer);
      clockTimer = undefined;
    }
  };
}

/// Re-renders once a minute (shared timer), for elapsed-time bars.
export function useMinute(): number {
  return useSyncExternalStore(subscribeClock, () => minute);
}

export function programmeProgress(p: Program, nowMs: number = Date.now()): number {
  if (p.stopUtc <= p.startUtc) return 0;
  const pct = ((nowMs / 1000 - p.startUtc) / (p.stopUtc - p.startUtc)) * 100;
  return Math.max(0, Math.min(100, pct));
}

export function formatGuideTime(epochSec: number): string {
  return new Date(epochSec * 1000).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
}
