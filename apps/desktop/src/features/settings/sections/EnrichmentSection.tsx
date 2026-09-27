//! Settings → Catalog Enrichment: download the on-device embedding model, run
//! the de-dup / genre-fill pass, and configure the opt-in TMDB metadata layer
//! (canonical titles, posters, backdrops, overviews, genres).

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { ProfileSettings, Source } from "@/types";
import { DEFAULT_PROFILE_SETTINGS } from "@/types";
import {
  SelectInput,
  SettingRow,
  SettingsCard,
  SettingsButton,
  TextInput,
  Toggle,
} from "../SettingsUI";

/// Languages offered for TMDB titles/overviews (BCP-47 tags TMDB accepts).
const METADATA_LANGUAGES = [
  { value: "en-US", label: "English" },
  { value: "fr-FR", label: "Français" },
  { value: "ar", label: "العربية" },
  { value: "es-ES", label: "Español" },
  { value: "de-DE", label: "Deutsch" },
  { value: "it-IT", label: "Italiano" },
  { value: "pt-BR", label: "Português (Brasil)" },
  { value: "tr-TR", label: "Türkçe" },
  { value: "nl-NL", label: "Nederlands" },
];

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
  // The token is write-only: the backend never returns it, so this draft is
  // the only place it exists in the page, and only while being typed.
  const [tokenDraft, setTokenDraft] = useState("");
  const [editingToken, setEditingToken] = useState(false);
  const [tokenError, setTokenError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const [m, s] = await Promise.all([
      invoke<EnrichModelStatus>("get_enrich_model_status"),
      invoke<ProfileSettings>("get_profile_settings", { profileId }),
    ]);
    setModel(m);
    setSettings(s);
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
  };

  const saveToken = async () => {
    setTokenError(null);
    try {
      await invoke("set_tmdb_credential", { profileId, credential: tokenDraft });
      setTokenDraft("");
      setEditingToken(false);
      await refresh();
    } catch (e) {
      setTokenError((e as { message?: string } | null)?.message ?? "The token could not be saved.");
    }
  };

  const removeToken = async () => {
    if (!window.confirm("Remove the saved TMDB token? TMDB matching stops until a new one is set."))
      return;
    await invoke("clear_tmdb_credential", { profileId });
    setTokenDraft("");
    setEditingToken(false);
    await refresh();
  };

  const showTokenInput = !settings.tmdbCredentialConfigured || editingToken;

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
            onChange={(v) => void persist({ ...settings, tmdbEnabled: v })}
            label="Use TMDB"
          />
        </SettingRow>
        <SettingRow
          title="Title language"
          description="Language of TMDB titles and overviews. Original titles stay searchable; changing this refetches titles on the next run."
        >
          <SelectInput
            value={settings.metadataLanguage}
            options={METADATA_LANGUAGES}
            onChange={(v) => void persist({ ...settings, metadataLanguage: v })}
            ariaLabel="TMDB title language"
          />
        </SettingRow>
        <div className="py-3">
          <p className="mb-1.5 text-sm font-medium text-white">API Read Access Token</p>
          {showTokenInput ? (
            <div className="flex items-center gap-2">
              <TextInput
                secret
                value={tokenDraft}
                onChange={setTokenDraft}
                placeholder="Read Access Token (or legacy API key)"
                ariaLabel="TMDB API Read Access Token"
              />
              <SettingsButton
                variant="primary"
                onClick={() => void saveToken()}
                disabled={tokenDraft.trim() === ""}
              >
                Save
              </SettingsButton>
              {settings.tmdbCredentialConfigured && (
                <SettingsButton
                  onClick={() => {
                    setTokenDraft("");
                    setTokenError(null);
                    setEditingToken(false);
                  }}
                >
                  Cancel
                </SettingsButton>
              )}
            </div>
          ) : (
            <div className="flex items-center gap-2">
              <span
                className="flex-1 font-mono text-sm text-white/70"
                aria-label="Saved TMDB token"
              >
                Token saved ••••{settings.tmdbCredentialHint}
              </span>
              <SettingsButton onClick={() => setEditingToken(true)}>Replace</SettingsButton>
              <SettingsButton variant="danger" onClick={() => void removeToken()}>
                Remove
              </SettingsButton>
            </div>
          )}
          {tokenError && <p className="mt-1.5 text-xs text-rose-300">{tokenError}</p>}
          <p className="mt-1.5 text-xs text-white/40">
            Free at themoviedb.org → Settings → API: copy the “API Read Access Token” (a legacy “API
            Key” also works). Stored only in this profile on this Mac and never shown again after
            saving; nothing is sent to TMDB unless this is enabled and a token is set.
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
