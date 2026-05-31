//! Settings → Transcription & Cache: whisper model + cached-transcript management.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { SettingRow, SettingsCard, SettingsButton } from "../SettingsUI";
import { formatBytes } from "../format";

interface ModelStatus {
  installed: boolean;
  modelId: string;
}
interface CacheStats {
  transcripts: number;
  bytes: number;
}
interface ModelProgress {
  downloadedBytes: number;
  totalBytes?: number | null;
}

export function TranscriptionSection({ profileId }: { profileId: string }) {
  const [model, setModel] = useState<ModelStatus | null>(null);
  const [cache, setCache] = useState<CacheStats | null>(null);
  const [busy, setBusy] = useState<null | "download" | "remove" | "clear">(null);
  const [pct, setPct] = useState<number | null>(null);

  const refresh = useCallback(async () => {
    const [m, c] = await Promise.all([
      invoke<ModelStatus>("get_model_status"),
      invoke<CacheStats>("get_transcription_cache_stats", { profileId }),
    ]);
    setModel(m);
    setCache(c);
  }, [profileId]);

  useEffect(() => {
    void refresh();
    const un = listen<ModelProgress>("model:download:progress", (e) => {
      const p = e.payload;
      setPct(p.totalBytes && p.totalBytes > 0 ? Math.round((p.downloadedBytes / p.totalBytes) * 100) : null);
    });
    return () => {
      void un.then((u) => u());
    };
  }, [refresh]);

  const download = async () => {
    setBusy("download");
    setPct(0);
    try {
      await invoke("download_whisper_model");
      await refresh();
    } finally {
      setBusy(null);
      setPct(null);
    }
  };
  const remove = async () => {
    setBusy("remove");
    try {
      await invoke("remove_whisper_model");
      await refresh();
    } finally {
      setBusy(null);
    }
  };
  const clearCache = async () => {
    if (!window.confirm("Delete all cached transcripts? They'll be re-generated on next watch.")) return;
    setBusy("clear");
    try {
      await invoke("clear_transcription_cache", { profileId });
      await refresh();
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <SettingsCard title="On-device model">
        <SettingRow
          title="Whisper model"
          description={model ? `${model.modelId} · ${model.installed ? "installed" : "not installed"}` : "…"}
        >
          {model?.installed ? (
            <SettingsButton variant="danger" onClick={remove} disabled={busy !== null}>
              {busy === "remove" ? "Removing…" : "Remove"}
            </SettingsButton>
          ) : (
            <SettingsButton variant="primary" onClick={download} disabled={busy !== null}>
              {busy === "download" ? (pct != null ? `Downloading… ${pct}%` : "Downloading…") : "Download (~500MB)"}
            </SettingsButton>
          )}
        </SettingRow>
      </SettingsCard>

      <SettingsCard title="Cache">
        <SettingRow
          title="Cached transcripts"
          description={cache ? `${cache.transcripts} item${cache.transcripts === 1 ? "" : "s"} · ${formatBytes(cache.bytes)}` : "…"}
        >
          <SettingsButton
            variant="danger"
            onClick={clearCache}
            disabled={busy !== null || (cache?.transcripts ?? 0) === 0}
          >
            {busy === "clear" ? "Clearing…" : "Clear cache"}
          </SettingsButton>
        </SettingRow>
      </SettingsCard>
    </div>
  );
}
