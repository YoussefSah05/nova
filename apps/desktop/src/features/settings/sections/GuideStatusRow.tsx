//! Settings → Sources: a source's programme guide — how fresh it is, whether
//! its feeds failed — with a manual refresh.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { EpgFeedStatus, EpgRefreshDone, EpgRefreshProgress } from "@/types";
import { SettingRow, SettingsButton } from "../SettingsUI";
import { describeGuide } from "./guideStatus";

/// `refreshQueued` changes whenever the caller queued a refresh (a guide URL
/// was just added): the row reloads and shows it as refreshing.
export function GuideStatusRow({
  profileId,
  sourceId,
  refreshQueued = 0,
}: {
  profileId: string;
  sourceId: number;
  refreshQueued?: number;
}) {
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
    if (refreshQueued === 0) return;
    setRefreshing(true);
    void load();
  }, [refreshQueued, load]);

  useEffect(() => {
    let cancelled = false;
    const stops: (() => void)[] = [];
    const keep = (stop: () => void) => {
      if (cancelled) stop();
      else stops.push(stop);
    };
    // Per feed: show progress and pick up each finished feed's counts.
    void listen<EpgRefreshProgress>("epg:refresh:progress", (event) => {
      if (event.payload.sourceId !== sourceId) return;
      if (event.payload.stage === "downloading" || event.payload.stage === "parsing") {
        setRefreshing(true);
      } else {
        void load();
      }
    })
      .then(keep)
      .catch(() => undefined);
    // The job ended, however it ended.
    void listen<EpgRefreshDone>("epg:refresh:done", (event) => {
      if (event.payload.sourceId !== sourceId) return;
      setRefreshing(false);
      void load();
    })
      .then(keep)
      .catch(() => undefined);
    return () => {
      cancelled = true;
      stops.forEach((stop) => stop());
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
