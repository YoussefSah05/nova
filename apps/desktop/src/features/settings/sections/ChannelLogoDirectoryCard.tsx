//! Settings → Catalog Enrichment → Channel logos: the opt-in iptv-org
//! directory that fills channel logos the provider and its EPG did not supply.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { ProfileSettings } from "@/types";
import { SettingRow, SettingsCard, SettingsButton, Toggle } from "../SettingsUI";

interface DirectoryStatus {
  downloaded: boolean;
  entries: number;
  builtAt: number | null;
}
interface ChannelLogosProgress {
  stage: "downloading" | "matching" | "complete" | "failed";
  matched: number;
  candidates: number;
  done: boolean;
  error: string | null;
}

function describe(status: DirectoryStatus | null, progress: ChannelLogosProgress | null): string {
  if (progress && !progress.done) {
    return progress.stage === "downloading"
      ? "Downloading the channel directory…"
      : "Matching channels…";
  }
  if (progress?.stage === "failed") return progress.error ?? "Could not update channel logos.";
  if (progress?.stage === "complete") {
    return `Found logos for ${progress.matched} of ${progress.candidates} channels without one.`;
  }
  if (status?.downloaded && status.builtAt) {
    const date = new Date(status.builtAt * 1000).toLocaleDateString();
    return `${status.entries.toLocaleString()} channels in the directory · updated ${date}`;
  }
  return "Not downloaded yet.";
}

export function ChannelLogoDirectoryCard({
  profileId,
  settings,
  persist,
}: {
  profileId: string;
  settings: ProfileSettings;
  persist: (next: ProfileSettings) => Promise<void>;
}) {
  const [status, setStatus] = useState<DirectoryStatus | null>(null);
  const [progress, setProgress] = useState<ChannelLogosProgress | null>(null);

  const loadStatus = useCallback(async () => {
    setStatus(await invoke<DirectoryStatus>("get_logo_directory_status").catch(() => null));
  }, []);

  useEffect(() => {
    void loadStatus();
    const un = listen<ChannelLogosProgress>("artwork:channel-logos", (e) => {
      setProgress(e.payload);
      if (e.payload.done) void loadStatus();
    });
    return () => {
      void un.then((u) => u());
    };
  }, [loadStatus]);

  const run = async (forceRefresh: boolean) => {
    await invoke("match_channel_logos", { profileId, sourceId: null, forceRefresh });
  };

  const toggle = async (enabled: boolean) => {
    await persist({ ...settings, logoDirectoryEnabled: enabled });
    if (enabled) {
      await run(false);
    } else {
      setProgress(null);
      await invoke("clear_channel_logo_matches", { profileId });
    }
  };

  const running = progress !== null && !progress.done;
  return (
    <SettingsCard title="Channel logos (optional)">
      <SettingRow
        title="Find missing channel logos online"
        description="Uses the public iptv-org channel directory (about 14 MB, refreshed weekly) and matches channels on this Mac by tvg-id or exact name. Channel names are never sent anywhere. Provider and EPG logos always win."
      >
        <Toggle
          checked={settings.logoDirectoryEnabled}
          onChange={(v) => void toggle(v)}
          label="Find missing channel logos online"
        />
      </SettingRow>
      {settings.logoDirectoryEnabled && (
        <SettingRow title="Directory" description={describe(status, progress)}>
          <SettingsButton onClick={() => void run(true)} disabled={running}>
            {running ? "Updating…" : "Update now"}
          </SettingsButton>
        </SettingRow>
      )}
    </SettingsCard>
  );
}
