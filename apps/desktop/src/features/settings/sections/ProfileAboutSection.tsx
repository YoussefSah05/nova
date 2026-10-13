//! Settings → Profile & About: edit/switch/delete profile + app info.

import { useEffect, useState } from "react";
import { NovaLogo } from "@/components/NovaLogo";
import { invoke } from "@tauri-apps/api/core";

import type { Profile } from "@/types";
import { SettingRow, SettingsCard, SettingsButton, TextInput, SettingsError } from "../SettingsUI";
import { formatBytes } from "../format";
import { useConfirm } from "../confirm";

const AVATAR_COLORS = ["#6366f1", "#ec4899", "#f59e0b", "#10b981", "#3b82f6", "#ef4444"];

interface AppInfo {
  version: string;
  dataDir: string;
  transcriptsBytes: number;
  modelsBytes: number;
}

export function ProfileAboutSection({
  profile,
  onProfileUpdated,
  onSwitchProfile,
}: {
  profile: Profile;
  onProfileUpdated: (p: Profile) => void;
  onSwitchProfile: () => void;
}) {
  const confirm = useConfirm();
  const [name, setName] = useState(profile.name);
  const [color, setColor] = useState(profile.avatarColor);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    void invoke<AppInfo>("get_app_info")
      .then(setInfo)
      .catch(() => undefined);
  }, []);

  const dirty = name.trim() !== profile.name || color !== profile.avatarColor;
  const save = async () => {
    if (!dirty || !name.trim()) return;
    setSaving(true);
    setError(null);
    try {
      const updated = await invoke<Profile>("update_profile", {
        id: profile.id,
        name: name.trim(),
        avatarColor: color,
      });
      onProfileUpdated(updated);
    } catch {
      setError("Couldn’t save your profile. Try again.");
    } finally {
      setSaving(false);
    }
  };
  const remove = async () => {
    const yes = await confirm({
      title: `Delete “${profile.name}”?`,
      description:
        "Its playlists, library, watch history and settings are deleted from this Mac. Shared models and artwork stay.",
      confirmLabel: "Delete profile",
      destructive: true,
    });
    if (!yes) return;
    setSaving(true);
    setError(null);
    try {
      await invoke("delete_profile", { id: profile.id });
      onSwitchProfile();
    } catch {
      setError("Couldn’t delete the profile. Try again.");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-6">
      {error && <SettingsError message={error} />}
      <SettingsCard title="Profile">
        <SettingRow title="Name">
          <div className="w-56">
            <TextInput value={name} onChange={setName} ariaLabel="Profile name" />
          </div>
        </SettingRow>
        <SettingRow title="Avatar colour">
          <div className="flex items-center gap-2">
            {AVATAR_COLORS.map((c) => (
              <button
                key={c}
                type="button"
                aria-label={`Colour ${c}`}
                aria-pressed={color === c}
                onClick={() => setColor(c)}
                className="h-7 w-7 rounded-full border-2 transition-transform hover:scale-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                style={{ backgroundColor: c, borderColor: color === c ? "#fff" : "transparent" }}
              />
            ))}
          </div>
        </SettingRow>
        <SettingRow title="">
          <SettingsButton
            variant="primary"
            onClick={save}
            disabled={!dirty || saving || !name.trim()}
          >
            {saving ? "Saving…" : "Save changes"}
          </SettingsButton>
        </SettingRow>
      </SettingsCard>

      <SettingsCard title="Session">
        <SettingRow title="Switch profile" description="Return to the profile picker.">
          <SettingsButton onClick={onSwitchProfile}>Switch</SettingsButton>
        </SettingRow>
        <SettingRow title="Delete this profile" description="Permanently removes this profile.">
          <SettingsButton variant="danger" onClick={remove} disabled={saving}>
            Delete
          </SettingsButton>
        </SettingRow>
      </SettingsCard>

      <SettingsCard title="About">
        <div className="flex items-center gap-4 border-b border-white/[0.06] py-5">
          <NovaLogo height={34} />
          <p className="text-xs leading-relaxed text-white/50">
            The star that brightens: your playlist, turned into a cinema that understands it.
          </p>
        </div>
        <SettingRow title="Version" description={info?.version ?? "…"} />
        <SettingRow title="Data folder" description={info?.dataDir ?? "…"} />
        <SettingRow
          title="Storage used"
          description={
            info
              ? `Transcripts ${formatBytes(info.transcriptsBytes)} · Models ${formatBytes(info.modelsBytes)}`
              : "…"
          }
        />
      </SettingsCard>

      <p className="text-xs text-white/40">
        This product uses the TMDB API but is not endorsed or certified by TMDB.
      </p>
    </div>
  );
}
