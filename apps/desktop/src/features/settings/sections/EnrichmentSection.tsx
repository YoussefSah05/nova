//! Catalog preferences and explicit enrichment. Installation lives in Models.
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";
import type { ProfileSettings, Source } from "@/types";
import type { EnrichModelStatus } from "@/types/generated/EnrichModelStatus";
import { Skeleton } from "@/components/ui/skeleton";
import { useProfileSettings } from "../useProfileSettings";
import { modelKeys } from "../modelData";
import {
  SelectInput,
  SettingRow,
  SettingsCard,
  SettingsButton,
  SettingsError,
  TextInput,
  Toggle,
} from "../SettingsUI";
import { ChannelLogoDirectoryCard } from "./ChannelLogoDirectoryCard";
import { useConfirm } from "../confirm";

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

export function EnrichmentSection({
  profileId,
  onOpenModels,
}: {
  profileId: string;
  onOpenModels?: () => void;
}) {
  const confirm = useConfirm();
  const { settings, saveSettings, loaded } = useProfileSettings(profileId);
  const credential = useQuery({
    queryKey: ["settings-credential", profileId],
    staleTime: 0,
    queryFn: () => invoke<ProfileSettings>("get_profile_settings", { profileId }),
  });
  const model = useQuery({
    queryKey: modelKeys.embedding,
    staleTime: 0,
    queryFn: () => invoke<EnrichModelStatus>("get_enrich_model_status"),
  });
  const [tokenDraft, setTokenDraft] = useState("");
  const [editingToken, setEditingToken] = useState(false);
  const [tokenBusy, setTokenBusy] = useState(false);
  const [tokenError, setTokenError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [runError, setRunError] = useState<string | null>(null);
  const saveToken = async () => {
    setTokenBusy(true);
    setTokenError(null);
    try {
      await invoke("set_tmdb_credential", { profileId, credential: tokenDraft });
      setTokenDraft("");
      setEditingToken(false);
      await credential.refetch();
    } catch {
      setTokenError("The token could not be saved. Try again.");
    } finally {
      setTokenBusy(false);
    }
  };
  const removeToken = async () => {
    const yes = await confirm({
      title: "Remove the TMDB token?",
      description: "TMDB matching, artwork and facts stop updating until a new token is set.",
      confirmLabel: "Remove token",
      destructive: true,
    });
    if (!yes) return;
    setTokenBusy(true);
    setTokenError(null);
    try {
      await invoke("clear_tmdb_credential", { profileId });
      setTokenDraft("");
      setEditingToken(false);
      await credential.refetch();
    } catch {
      setTokenError("The token could not be removed. Try again.");
    } finally {
      setTokenBusy(false);
    }
  };
  const run = async () => {
    setRunning(true);
    setMessage(null);
    setRunError(null);
    try {
      const current = await model.refetch();
      if (!current.data?.installed) {
        setRunError("Download the catalog model in Models first.");
        return;
      }
      const sources = await invoke<Source[]>("list_sources", { profileId });
      for (const source of sources)
        await invoke("start_enrichment", { profileId, sourceId: source.id });
      setMessage(
        sources.length
          ? `Enrichment queued for ${sources.length} playlist${sources.length === 1 ? "" : "s"}. You can keep browsing.`
          : "Add a playlist before running enrichment."
      );
    } catch {
      setRunError(
        "Couldn’t queue all playlists. Existing catalog content is still available; try again."
      );
    } finally {
      setRunning(false);
    }
  };
  const showTokenInput = !credential.data?.tmdbCredentialConfigured || editingToken;
  if (credential.isError)
    return (
      <SettingsError
        message="Couldn’t load catalog preferences."
        retry={() => void credential.refetch()}
      />
    );
  if (!loaded || credential.isPending)
    return <Skeleton className="h-48" aria-label="Loading catalog preferences" />;
  return (
    <div className="flex flex-col gap-6">
      <SettingsCard title="Catalog enrichment">
        <SettingRow
          title="Refresh catalog intelligence"
          description={
            model.data?.installed
              ? "Fill missing genres and update search for your playlists."
              : "The catalog model is managed in Models."
          }
        >
          {model.data?.installed ? (
            <SettingsButton variant="primary" onClick={() => void run()} disabled={running}>
              {running ? "Queuing…" : "Run enrichment"}
            </SettingsButton>
          ) : (
            <SettingsButton onClick={() => onOpenModels?.()}>Open Models</SettingsButton>
          )}
        </SettingRow>
        {model.isError && (
          <div className="pb-4">
            <SettingsError
              message="Couldn’t check the catalog model."
              retry={() => void model.refetch()}
            />
          </div>
        )}
        {message && (
          <p role="status" className="pb-4 text-xs text-muted-foreground">
            {message}
          </p>
        )}
        {runError && (
          <div className="pb-4">
            <SettingsError message={runError} />
          </div>
        )}
      </SettingsCard>
      <SettingsCard title="TMDB metadata (optional)">
        <SettingRow
          title="Use TMDB"
          description="Fetch canonical titles, posters, backdrops, overviews and genres for movies and series."
        >
          <Toggle
            checked={settings.tmdbEnabled}
            onChange={(v) => void saveSettings({ tmdbEnabled: v })}
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
            onChange={(v) => void saveSettings({ metadataLanguage: v })}
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
                disabled={tokenDraft.trim() === "" || tokenBusy}
              >
                Save
              </SettingsButton>
              {credential.data?.tmdbCredentialConfigured && (
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
                Token saved ••••{credential.data?.tmdbCredentialHint}
              </span>
              <SettingsButton disabled={tokenBusy} onClick={() => setEditingToken(true)}>
                Replace
              </SettingsButton>
              <SettingsButton
                variant="danger"
                disabled={tokenBusy}
                onClick={() => void removeToken()}
              >
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

      <ChannelLogoDirectoryCard
        profileId={profileId}
        settings={settings}
        persist={async (next) => {
          await saveSettings({ logoDirectoryEnabled: next.logoDirectoryEnabled });
        }}
      />

      <p className="text-xs text-white/40">
        Catalog enrichment fills missing genres and improves search with on-device AI. Runs
        automatically after an import once the model is installed. This product uses the TMDB API
        but is not endorsed or certified by TMDB.
      </p>
    </div>
  );
}
