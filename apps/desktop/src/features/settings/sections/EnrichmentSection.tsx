//! Settings → Catalog Enrichment: download the on-device embedding model, run
//! the de-dup / genre-fill pass, and configure the opt-in TMDB metadata layer
//! (canonical titles, posters, backdrops, overviews, genres).

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { ProfileSettings, Source } from "@/types";
import { DEFAULT_PROFILE_SETTINGS } from "@/types";
import { SettingRow, SettingsCard, SettingsButton, TextInput, Toggle } from "../SettingsUI";

interface EnrichModelStatus {
  installed: boolean;
  modelId: string;
}
interface EnrichProgress {
  sourceId: number;
  stage: string;
  processed: number;
  total: number;
  done: boolean;
  error?: string | null;
}

export function EnrichmentSection({ profileId }: { profileId: string }) {
  const [model, setModel] = useState<EnrichModelStatus | null>(null);
  const [busy, setBusy] = useState<null | "download" | "run">(null);
  const [progress, setProgress] = useState<EnrichProgress | null>(null);
  const [settings, setSettings] = useState<ProfileSettings>(DEFAULT_PROFILE_SETTINGS);
  const [keyDraft, setKeyDraft] = useState("");
  const [saved, setSaved] = useState(false);

  const refresh = useCallback(async () => {
    const [m, s] = await Promise.all([
      invoke<EnrichModelStatus>("get_enrich_model_status"),
      invoke<ProfileSettings>("get_profile_settings", { profileId }),
    ]);
    setModel(m);
    setSettings(s);
    setKeyDraft(s.tmdbApiKey ?? "");
  }, [profileId]);

  useEffect(() => {
    void refresh();
    const un = listen<EnrichProgress>("catalog:enrich:progress", (e) => {
      setProgress(e.payload);
      if (e.payload.done) {
        setBusy(null);
        // "no-model" means the marker vanished — re-sync so the button flips to Download.
        if (e.payload.stage === "no-model") void refresh();
      }
    });
    return () => {
      void un.then((u) => u());
    };
  }, [refresh]);

  const persist = async (next: ProfileSettings) => {
    setSettings(next);
    await invoke("set_profile_settings", { profileId, settings: next });
    setSaved(true);
    window.setTimeout(() => setSaved(false), 1500);
  };

  const download = async () => {
    setBusy("download");
    try {
      await invoke("download_enrich_models");
      await refresh();
    } finally {
      setBusy(null);
    }
  };

  const run = async () => {
    setBusy("run");
    setProgress(null);
    try {
      const sources = await invoke<Source[]>("list_sources", { profileId });
      for (const s of sources) {
        await invoke("start_enrichment", { profileId, sourceId: s.id });
      }
    } catch {
      setBusy(null);
    }
    // `busy` clears on the terminal `catalog:enrich:progress` event.
  };

  const runLabel = (() => {
    if (!progress || progress.done) return "Running…";
    if (progress.stage === "queued") return "Waiting for available resources…";
    if (progress.stage === "loading-model") return "Loading model…";
    const stageWord =
      progress.stage === "embedding"
        ? "Embedding"
        : progress.stage === "matching"
          ? "Matching"
          : "Working";
    return progress.total > 0
      ? `${stageWord} ${progress.processed}/${progress.total}`
      : `${stageWord}…`;
  })();

  return (
    <div className="flex flex-col gap-6">
      <SettingsCard title="On-device model">
        <SettingRow
          title="Embedding model"
          description={
            model ? `${model.modelId} · ${model.installed ? "installed" : "not installed"}` : "…"
          }
        >
          {model?.installed ? (
            <SettingsButton variant="primary" onClick={run} disabled={busy !== null}>
              {busy === "run" ? runLabel : "Run enrichment"}
            </SettingsButton>
          ) : (
            <SettingsButton variant="primary" onClick={download} disabled={busy !== null}>
              {busy === "download" ? "Downloading…" : "Download (~120MB)"}
            </SettingsButton>
          )}
        </SettingRow>
      </SettingsCard>

      <SettingsCard title="TMDB metadata (optional)">
        <SettingRow
          title="Use TMDB"
          description="Fetch canonical titles, posters, backdrops, overviews and genres for movies and series."
        >
          <Toggle
            checked={settings.tmdbEnabled}
            onChange={(v) =>
              void persist({ ...settings, tmdbEnabled: v, tmdbApiKey: keyDraft || null })
            }
            label="Use TMDB"
          />
        </SettingRow>
        <div className="py-3">
          <p className="mb-1.5 text-sm font-medium text-white">API key</p>
          <div className="flex items-center gap-2">
            <TextInput
              value={keyDraft}
              onChange={setKeyDraft}
              placeholder="TMDB API key (v3 auth)"
              ariaLabel="TMDB API key"
            />
            <SettingsButton
              variant="primary"
              onClick={() => void persist({ ...settings, tmdbApiKey: keyDraft || null })}
            >
              {saved ? "Saved" : "Save"}
            </SettingsButton>
          </div>
          <p className="mt-1.5 text-xs text-white/40">
            Get a free key at themoviedb.org → Settings → API. Stored locally; nothing is sent to
            TMDB unless this is enabled and a key is set.
          </p>
        </div>
      </SettingsCard>

      <p className="text-xs text-white/40">
        Cleans the catalog with on-device AI: collapses duplicate channels/movies into one card and
        fills in missing genres. Runs automatically after an import once the model is installed —
        re-run here anytime. This product uses the TMDB API but is not endorsed or certified by
        TMDB.
      </p>
    </div>
  );
}
