//! Per-profile caption-style state (Phase 6.2).
//!
//! Loads the saved caption appearance via `get_caption_style` on mount and
//! exposes a setter that updates optimistically and persists (debounced) via
//! `set_caption_style`. Style follows the profile (stored in its SQLite DB), so
//! it is restored on the next session and reused by Phase 7's translation card.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { DEFAULT_CAPTION_STYLE, type CaptionStyle } from "@/types";

const PERSIST_DEBOUNCE_MS = 400;

export interface CaptionStyleState {
  style: CaptionStyle;
  setStyle: (patch: Partial<CaptionStyle>) => void;
}

export function useCaptionStyle(profileId: string): CaptionStyleState {
  const [style, setStyleState] = useState<CaptionStyle>(DEFAULT_CAPTION_STYLE);
  const saveTimer = useRef<number | null>(null);
  // Latest style, so the debounced writer persists the freshest value.
  const latest = useRef(style);
  latest.current = style;

  // Load the saved style for this profile.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const loaded = await invoke<CaptionStyle>("get_caption_style", { profileId });
        if (!cancelled) setStyleState(loaded);
      } catch {
        // Keep defaults if the read fails (e.g. profile not yet open).
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [profileId]);

  const setStyle = useCallback(
    (patch: Partial<CaptionStyle>) => {
      setStyleState((prev) => ({ ...prev, ...patch }));
      if (saveTimer.current != null) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => {
        void invoke("set_caption_style", { profileId, style: latest.current }).catch(
          () => undefined
        );
      }, PERSIST_DEBOUNCE_MS);
    },
    [profileId]
  );

  // Flush any pending write on unmount.
  useEffect(
    () => () => {
      if (saveTimer.current != null) window.clearTimeout(saveTimer.current);
    },
    []
  );

  return { style, setStyle };
}
