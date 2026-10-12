//! Settings → Appearance: pick the app's accent colour. Applies live to the
//! whole UI (buttons / focus / active states / glow) by overriding the brand
//! CSS vars, and persists to the profile.

import { invoke } from "@tauri-apps/api/core";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check } from "lucide-react";

import { cn } from "@/lib/utils";
import { ACCENT_THEMES, applyAccentTheme } from "@/lib/useAccentTheme";
import { useProfileSettings } from "../useProfileSettings";
import { SettingRow, SettingsCard, SettingsError, Toggle } from "../SettingsUI";

export function AppearanceSection({ profileId }: { profileId: string }) {
  const { settings, setSettings } = useProfileSettings(profileId);

  const choose = (id: string) => {
    applyAccentTheme(id); // instant, app-wide
    setSettings({ accentColor: id }); // persist
  };

  return (
    <div className="flex flex-col gap-6">
      <SettingsCard title="Accent colour">
        <div className="py-4">
          <div className="flex flex-wrap gap-3">
            {ACCENT_THEMES.map((t) => {
              const active = settings.accentColor === t.id;
              return (
                <button
                  key={t.id}
                  type="button"
                  onClick={() => choose(t.id)}
                  aria-label={t.label}
                  aria-pressed={active}
                  title={t.label}
                  className={cn(
                    "flex h-10 w-10 items-center justify-center rounded-full ring-2 ring-offset-2 ring-offset-neutral-950 transition-transform hover:scale-110 focus-visible:outline-none focus-visible:ring-primary",
                    active ? "ring-white" : "ring-transparent"
                  )}
                  style={{ background: `linear-gradient(to bottom, ${t.hi}, ${t.lo})` }}
                >
                  {active && <Check className="h-4 w-4 text-black/70" aria-hidden />}
                </button>
              );
            })}
          </div>
        </div>
      </SettingsCard>

      <p className="text-xs text-white/40">
        The accent recolours buttons, highlights, focus rings and the active rail item across the
        app. Saved to this profile.
      </p>

      <DisplayCard />
    </div>
  );
}

const REFRESH_KEY = ["settings-display", "highRefreshRate"] as const;

/** App-wide: render the interface at the display's full refresh rate. */
function DisplayCard() {
  const client = useQueryClient();
  const current = useQuery({
    queryKey: REFRESH_KEY,
    queryFn: () => invoke<boolean>("get_high_refresh_rate"),
  });
  const change = useMutation({
    mutationFn: (enabled: boolean) => invoke<void>("set_high_refresh_rate", { enabled }),
    onMutate: (enabled) => client.setQueryData(REFRESH_KEY, enabled),
    onSettled: () => client.invalidateQueries({ queryKey: REFRESH_KEY }),
  });
  return (
    <SettingsCard title="Display">
      {current.isError ? (
        <div className="py-4">
          <SettingsError
            message="Couldn’t read the display setting."
            retry={() => void current.refetch()}
          />
        </div>
      ) : (
        <SettingRow
          title="High refresh rate"
          description="Animate and scroll at your display’s full rate (120 Hz on ProMotion) instead of 60. Smoother, uses a little more battery. Shared by every profile on this Mac."
        >
          <Toggle
            checked={current.data ?? true}
            onChange={(next) => change.mutate(next)}
            label="High refresh rate"
          />
        </SettingRow>
      )}
      {change.isError && (
        <p role="alert" className="pb-4 text-xs text-red-300">
          Couldn’t save the display setting. Try again.
        </p>
      )}
    </SettingsCard>
  );
}
