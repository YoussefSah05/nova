import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Channel, NowAndNext } from "@/types";

/// Refetch cadence — XMLTV programmes are minutes long, so a 1-minute tick
/// is enough to keep the rolling "Now" line accurate.
const REFRESH_MS = 60_000;

/// Returns a map of `tvgId → NowAndNext` for the given channels. Channels
/// without a `tvgId` are silently skipped (they can never be matched against
/// any XMLTV feed). Refetches on a fixed interval so the "Now" line stays
/// fresh as programmes roll over.
export function useNowAndNext(
  profileId: string | null,
  channels: Channel[]
): Record<string, NowAndNext> {
  const tvgIds = useMemo(() => {
    const seen = new Set<string>();
    const ids: string[] = [];
    for (const c of channels) {
      if (c.tvgId && !seen.has(c.tvgId)) {
        seen.add(c.tvgId);
        ids.push(c.tvgId);
      }
    }
    return ids;
  }, [channels]);

  // When every channel comes from one source, its own EPG feeds win where
  // several feeds cover a channel.
  const sourceId = useMemo(() => {
    const ids = new Set(channels.map((c) => c.sourceId));
    return ids.size === 1 ? [...ids][0] : null;
  }, [channels]);

  // Memoize the key the effect should re-run on — joining ids is cheap
  // compared to firing the IPC on every parent render.
  const tvgKey = JSON.stringify([sourceId, tvgIds]);

  const [data, setData] = useState<Record<string, NowAndNext>>({});

  useEffect(() => {
    if (!profileId || tvgIds.length === 0) {
      setData({});
      return;
    }
    let cancelled = false;

    const fetchNow = async () => {
      try {
        const results = await invoke<NowAndNext[]>("epg_now_and_next", {
          profileId,
          tvgIds,
          sourceId,
        });
        if (cancelled) return;
        const map: Record<string, NowAndNext> = {};
        for (const r of results) map[r.channelTvgId] = r;
        setData(map);
      } catch {
        // Silent — EPG is decorative. The chip just doesn't render.
      }
    };

    void fetchNow();
    const interval = setInterval(fetchNow, REFRESH_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profileId, tvgKey]);

  return data;
}
