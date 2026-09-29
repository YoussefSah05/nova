import { useProfilePreference } from "@/lib/useProfilePreference";
import { DEFAULT_PROFILE_SETTINGS, type ProfileSettings } from "@/types";

export interface ProfileSettingsState {
  settings: ProfileSettings;
  setSettings: (patch: Partial<ProfileSettings>) => void;
  /// Saves without the debounce; settles once the value is persisted.
  saveSettings: (patch: Partial<ProfileSettings>) => Promise<unknown>;
  loaded: boolean;
}

export function useProfileSettings(profileId: string): ProfileSettingsState {
  const { value, update, save, loaded } = useProfilePreference(
    profileId,
    DEFAULT_PROFILE_SETTINGS,
    "get_profile_settings",
    "set_profile_settings",
    "settings"
  );
  return { settings: value, setSettings: update, saveSettings: save, loaded };
}
