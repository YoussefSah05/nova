//! Autoplay handoff from the Rust playback coordinator: it decides the native
//! session's media ended, finds the next episode and reads the profile's
//! autoplay setting; the player only opens what it is handed, for its own
//! session (an event for a replaced session, or one arriving while the user
//! is already switching to other media, is ignored).

import { useEffect, useRef, type RefObject } from "react";
import { listen } from "@tauri-apps/api/event";

import type { PlaybackEnded, SeriesEpisode } from "@/types";

export function usePlaybackEnded(
  enabled: boolean,
  tokenRef: RefObject<string | null>,
  openingRef: RefObject<boolean>,
  openNext: (next: SeriesEpisode) => void
): void {
  const openNextRef = useRef(openNext);
  openNextRef.current = openNext;
  useEffect(() => {
    if (!enabled) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen<PlaybackEnded>("playback:ended", (event) => {
      const { token, nextEpisode, autoplay } = event.payload;
      if (openingRef.current || token !== tokenRef.current || !autoplay || !nextEpisode) return;
      openNextRef.current(nextEpisode);
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [enabled, tokenRef, openingRef]);
}
