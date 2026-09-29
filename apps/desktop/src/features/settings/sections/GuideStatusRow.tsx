//! Settings → Sources: a source's programme guide — how fresh it is, whether
//! its feeds failed — with a manual refresh.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { EpgFeedStatus, EpgRefreshProgress } from "@/types";
import { SettingRow, SettingsButton } from "../SettingsUI";
import { describeGuide } from "./guideStatus";

export function GuideStatusRow({ profileId, sourceId }: { profileId: string; sourceId: number }) {
  const [feeds, setFeeds] = useState<EpgFeedStatus[] | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const load = useCallback(async () => {
    try {
      setFeeds(await invoke<EpgFeedStatus[]>("list_epg_feeds", { profileId, sourceId }));
    } catch {
      setFeeds([]);
    }
  }, [profileId, sourceId]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<EpgRefreshProgress>("epg:refresh:progress", (event) => {
      if (event.payload.sourceId !== sourceId) return;
      const running = event.payload.stage === "downloading" || event.payload.stage === "parsing";
      setRefreshing(running);
      if (!running) void load();
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [sourceId, load]);

  const refresh = async () => {
    setRefreshing(true);
    try {
      await invoke("epg_refresh", { profileId, sourceId });
    } catch {
      setRefreshing(false);
    }
  };

  const description = refreshing
    ? "Refreshing the guide…"
    : feeds === null
      ? "Loading…"
      : describeGuide(feeds);

  return (
    <SettingRow title="Programme guide" description={description}>
      <SettingsButton onClick={refresh} disabled={refreshing || feeds?.length === 0}>
        {refreshing ? "Refreshing…" : "Refresh guide"}
      </SettingsButton>
    </SettingRow>
  );
}
