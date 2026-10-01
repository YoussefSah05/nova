//! Settings → Appearance: pick the app's accent colour. Applies live to the
//! whole UI (buttons / focus / active states / glow) by overriding the brand
//! CSS vars, and persists to the profile.

import { Check } from "lucide-react";

import { cn } from "@/lib/utils";
import { ACCENT_THEMES, applyAccentTheme } from "@/lib/useAccentTheme";
import { useProfileSettings } from "../useProfileSettings";
import { SettingsCard } from "../SettingsUI";

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
    </div>
  );
}
