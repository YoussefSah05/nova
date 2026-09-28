//! Settings → Sources & EPG: manage playlists (refresh / auto-refresh / remove)
//! and register community EPG (XMLTV) URLs.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { Source } from "@/types";
import { SettingRow, SettingsCard, SettingsButton, SelectInput, TextInput } from "../SettingsUI";

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
}: {
  profileId: string;
  onSourcesChanged: () => void;
}) {
  const [sources, setSources] = useState<Source[]>([]);
  const [epgUrl, setEpgUrl] = useState<Record<number, string>>({});
  const [busy, setBusy] = useState<number | null>(null);

  const load = useCallback(async () => {
    try {
      setSources(await invoke<Source[]>("list_sources", { profileId }));
    } catch {
      setSources([]);
    }
  }, [profileId]);

  useEffect(() => {
    void load();
  }, [load]);

  const refresh = async (id: number) => {
    setBusy(id);
    try {
      await invoke("refresh_source", { profileId, sourceId: id });
    } finally {
      setBusy(null);
    }
  };
  const setAutoRefresh = async (id: number, hours: number) => {
    setSources((prev) => prev.map((s) => (s.id === id ? { ...s, autoRefreshHours: hours } : s)));
    await invoke("set_source_auto_refresh", { profileId, sourceId: id, hours }).catch(
      () => undefined
    );
  };
  const remove = async (s: Source) => {
    if (!window.confirm(`Remove “${s.name}” and all its imported content?`)) return;
    setBusy(s.id);
    try {
      await invoke("delete_source", { profileId, sourceId: s.id });
      await load();
      onSourcesChanged();
    } finally {
      setBusy(null);
    }
  };
  const addEpg = async (id: number) => {
    const url = (epgUrl[id] ?? "").trim();
    if (!url) return;
    await invoke("add_epg_url", { profileId, sourceId: id, url }).catch(() => undefined);
    setEpgUrl((prev) => ({ ...prev, [id]: "" }));
  };

  if (sources.length === 0) {
    return <p className="text-sm text-white/50">No sources yet. Add one from the home screen.</p>;
  }

  return (
    <div className="flex flex-col gap-6">
      {sources.map((s) => (
        <SettingsCard key={s.id} title={`${s.name} · ${s.kind.toUpperCase()}`}>
          <SettingRow title="Refresh now" description={s.displayUrl}>
            <SettingsButton onClick={() => refresh(s.id)} disabled={busy === s.id}>
              {busy === s.id ? "Working…" : "Refresh"}
            </SettingsButton>
          </SettingRow>
          <SettingRow title="Auto-refresh" description="How often to re-fetch this playlist.">
            <SelectInput
              value={String(s.autoRefreshHours)}
              onChange={(v) => setAutoRefresh(s.id, Number(v))}
              ariaLabel="Auto-refresh interval"
              options={REFRESH_OPTIONS}
            />
          </SettingRow>
          <SettingRow
            title="EPG (XMLTV) URL"
            description="Add a community guide feed for this source."
          >
            <div className="flex w-72 items-center gap-2">
              <TextInput
                value={epgUrl[s.id] ?? ""}
                onChange={(v) => setEpgUrl((prev) => ({ ...prev, [s.id]: v }))}
                placeholder="https://…/epg.xml"
                ariaLabel="EPG URL"
              />
              <SettingsButton onClick={() => addEpg(s.id)}>Add</SettingsButton>
            </div>
          </SettingRow>
          <SettingRow
            title="Remove source"
            description="Deletes the source and everything it imported."
          >
            <SettingsButton variant="danger" onClick={() => remove(s)} disabled={busy === s.id}>
              Remove
            </SettingsButton>
          </SettingRow>
        </SettingsCard>
      ))}
    </div>
  );
}
