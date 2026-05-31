//! Settings → Playback & Captions: viewing preferences + caption-style editor.

import { useCaptionStyle } from "@/features/player/useCaptionStyle";
import { CaptionStyleControls } from "@/features/player/CaptionStyleControls";
import { useProfileSettings } from "../useProfileSettings";
import { SettingRow, SettingsCard, Toggle, SelectInput, TextInput } from "../SettingsUI";
import type { DefaultPlaybackRoute } from "@/types";

export function PlaybackCaptionsSection({ profileId }: { profileId: string }) {
  const { settings, setSettings } = useProfileSettings(profileId);
  const caption = useCaptionStyle(profileId);

  return (
    <div className="flex flex-col gap-6">
      <SettingsCard title="Playback">
        <SettingRow
          title="Autoplay next episode"
          description="Roll into the next episode automatically when one finishes."
        >
          <Toggle
            checked={settings.autoplayNext}
            onChange={(v) => setSettings({ autoplayNext: v })}
            label="Autoplay next episode"
          />
        </SettingRow>
        <SettingRow title="Default playback route" description="How streams are delivered to the player.">
          <SelectInput<DefaultPlaybackRoute>
            value={settings.defaultPlaybackRoute}
            onChange={(v) => setSettings({ defaultPlaybackRoute: v })}
            ariaLabel="Default playback route"
            options={[
              { value: "auto", label: "Automatic" },
              { value: "remux_hls", label: "Remux (no re-encode)" },
              { value: "transcode_hls", label: "Transcode" },
            ]}
          />
        </SettingRow>
      </SettingsCard>

      <SettingsCard title="Captions">
        <SettingRow title="Default caption mode" description="Applied when a title opens.">
          <SelectInput
            value={settings.defaultCaptionMode}
            onChange={(v) => setSettings({ defaultCaptionMode: v })}
            ariaLabel="Default caption mode"
            options={[
              { value: "off", label: "Off" },
              { value: "source", label: "Source language" },
            ]}
          />
        </SettingRow>
        <SettingRow title="Preferred audio language" description="ISO code (e.g. en, fr, ar). Optional.">
          <div className="w-40">
            <TextInput
              value={settings.preferredAudioLang ?? ""}
              onChange={(v) => setSettings({ preferredAudioLang: v.trim() || null })}
              placeholder="auto"
              ariaLabel="Preferred audio language"
            />
          </div>
        </SettingRow>
        <SettingRow title="Preferred subtitle language" description="ISO code. Optional.">
          <div className="w-40">
            <TextInput
              value={settings.preferredSubtitleLang ?? ""}
              onChange={(v) => setSettings({ preferredSubtitleLang: v.trim() || null })}
              placeholder="auto"
              ariaLabel="Preferred subtitle language"
            />
          </div>
        </SettingRow>
      </SettingsCard>

      <SettingsCard title="Caption appearance">
        <div className="py-3">
          <CaptionStyleControls style={caption.style} onChange={caption.setStyle} />
        </div>
      </SettingsCard>
    </div>
  );
}
