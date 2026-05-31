//! Per-profile viewing/behaviour settings state (cleanup phase).
//!
//! Loads via `get_profile_settings` on mount and exposes a setter that updates
//! optimistically and persists (debounced) via `set_profile_settings`. Mirrors
//! `features/player/useCaptionStyle.ts`.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { DEFAULT_PROFILE_SETTINGS, type ProfileSettings } from "@/types";

const PERSIST_DEBOUNCE_MS = 400;

export interface ProfileSettingsState {
  settings: ProfileSettings;
  setSettings: (patch: Partial<ProfileSettings>) => void;
  loaded: boolean;
}

export function useProfileSettings(profileId: string): ProfileSettingsState {
  const [settings, setState] = useState<ProfileSettings>(DEFAULT_PROFILE_SETTINGS);
  const [loaded, setLoaded] = useState(false);
  const saveTimer = useRef<number | null>(null);
  const latest = useRef(settings);
  latest.current = settings;

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const s = await invoke<ProfileSettings>("get_profile_settings", { profileId });
        if (!cancelled) setState(s);
      } catch {
        // Keep defaults on failure.
      } finally {
        if (!cancelled) setLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [profileId]);

  const setSettings = useCallback(
    (patch: Partial<ProfileSettings>) => {
      setState((prev) => ({ ...prev, ...patch }));
      if (saveTimer.current != null) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => {
        void invoke("set_profile_settings", { profileId, settings: latest.current }).catch(
          () => undefined
        );
      }, PERSIST_DEBOUNCE_MS);
    },
    [profileId]
  );

  useEffect(
    () => () => {
      if (saveTimer.current != null) window.clearTimeout(saveTimer.current);
    },
    []
  );

  return { settings, setSettings, loaded };
}
