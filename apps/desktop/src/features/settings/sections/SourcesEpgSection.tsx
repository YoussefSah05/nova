//! Settings → Sources & EPG: manage playlists (refresh / auto-refresh / remove)
//! and register community EPG (XMLTV) URLs.

import { useCallback, useEffect, useState } from "react";
import { Skeleton } from "@/components/ui/skeleton";
import { invoke } from "@tauri-apps/api/core";

import type { Source } from "@/types";
import {
  SettingRow,
  SettingsCard,
  SettingsButton,
  SelectInput,
  TextInput,
  SettingsError,
} from "../SettingsUI";
import { GuideStatusRow } from "./GuideStatusRow";

const REFRESH_OPTIONS = [
  { value: "0", label: "Manual" },
  { value: "6", label: "Every 6h" },
  { value: "12", label: "Every 12h" },
  { value: "24", label: "Daily" },
  { value: "168", label: "Weekly" },
];

export function SourcesEpgSection({
  profileId,
  onSourcesChanged,
  onAddSource,
}: {
  profileId: string;
  onSourcesChanged: () => void;
  onAddSource: () => void;
}) {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sources, setSources] = useState<Source[]>([]);
  const [epgUrl, setEpgUrl] = useState<Record<number, string>>({});
  const [busy, setBusy] = useState<number | null>(null);
  const [epgError, setEpgError] = useState<Record<number, string>>({});
  /// Bumped per source when adding a feed queued its guide refresh.
  const [guideQueued, setGuideQueued] = useState<Record<number, number>>({});

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setError(null);
      setSources(await invoke<Source[]>("list_sources", { profileId }));
    } catch {
      setError("Couldn’t load playlists. Try again.");
    } finally {
      setLoading(false);
    }
  }, [profileId]);

  useEffect(() => {
    void load();
  }, [load]);

  const refresh = async (id: number) => {
    setBusy(id);
    try {
      await invoke("refresh_source", { profileId, sourceId: id });
      setError(null);
    } catch {
      setError("Couldn’t refresh the playlist. Your existing catalog is still available.");
    } finally {
      setBusy(null);
    }
  };
  const setAutoRefresh = async (id: number, hours: number) => {
    setBusy(id);
    try {
      await invoke("set_source_auto_refresh", { profileId, sourceId: id, hours });
      setError(null);
      await load();
    } catch {
      setError("Couldn’t save the refresh interval. The previous value is unchanged.");
    } finally {
      setBusy(null);
    }
  };
  const remove = async (s: Source) => {
    if (!window.confirm(`Remove “${s.name}” and all its imported content?`)) return;
    setBusy(s.id);
    try {
      await invoke("delete_source", { profileId, sourceId: s.id });
      await load();
      onSourcesChanged();
    } catch {
      setError("Couldn’t remove the playlist. Try again.");
    } finally {
      setBusy(null);
    }
  };
  const addEpg = async (id: number) => {
    const url = (epgUrl[id] ?? "").trim();
    if (!url) return;
    setBusy(id);
    try {
      await invoke("add_epg_url", { profileId, sourceId: id, url });
    } catch {
      setEpgError((prev) => ({
        ...prev,
        [id]: "Couldn’t add the guide feed. Check the URL and try again.",
      }));
      return;
    } finally {
      setBusy(null);
    }
    setEpgError((prev) => ({ ...prev, [id]: "" }));
    setEpgUrl((prev) => ({ ...prev, [id]: "" }));
    setGuideQueued((prev) => ({ ...prev, [id]: (prev[id] ?? 0) + 1 }));
  };

  if (loading) return <Skeleton className="h-48" aria-label="Loading playlists" />;
  if (error && sources.length === 0)
    return <SettingsError message={error} retry={() => void load()} />;
  if (sources.length === 0)
    return (
      <SettingsCard title="Your playlists">
        <SettingRow
          title="No playlists yet"
          description="Add an M3U or Xtream playlist to start your library."
        >
          <SettingsButton variant="primary" onClick={onAddSource}>
            Add a playlist
          </SettingsButton>
        </SettingRow>
      </SettingsCard>
    );

  return (
    <div className="flex flex-col gap-6">
      {error && <SettingsError message={error} retry={() => void load()} />}
      {sources.map((s) => (
        <SettingsCard key={s.id} title={`${s.name} · ${s.kind.toUpperCase()}`}>
          <SettingRow title="Refresh now" description={s.displayUrl}>
            <SettingsButton onClick={() => refresh(s.id)} disabled={busy !== null}>
              {busy === s.id ? "Working…" : "Refresh"}
            </SettingsButton>
          </SettingRow>
          <SettingRow
            title="Auto-refresh"
            description="Re-fetch this playlist automatically while Nova is open. Paused during playback."
          >
            <SelectInput
              value={String(s.autoRefreshHours)}
              onChange={(v) => setAutoRefresh(s.id, Number(v))}
              ariaLabel="Auto-refresh interval"
              disabled={busy !== null}
              options={REFRESH_OPTIONS}
            />
          </SettingRow>
          <GuideStatusRow profileId={profileId} sourceId={s.id} refreshQueued={guideQueued[s.id]} />
          <SettingRow
            title="EPG (XMLTV) URL"
            description={
              epgError[s.id] ||
              "Add a community guide feed for this source; it is fetched right away."
            }
          >
            <div className="flex w-72 items-center gap-2">
              <TextInput
                value={epgUrl[s.id] ?? ""}
                onChange={(v) => setEpgUrl((prev) => ({ ...prev, [s.id]: v }))}
                placeholder="https://…/epg.xml"
                ariaLabel="EPG URL"
              />
              <SettingsButton
                disabled={busy !== null || !(epgUrl[s.id] ?? "").trim()}
                onClick={() => addEpg(s.id)}
              >
                Add
              </SettingsButton>
            </div>
          </SettingRow>
          <SettingRow
            title="Remove source"
            description="Deletes the source and everything it imported."
          >
            <SettingsButton variant="danger" onClick={() => remove(s)} disabled={busy !== null}>
              Remove
            </SettingsButton>
          </SettingRow>
        </SettingsCard>
      ))}
    </div>
  );
}
