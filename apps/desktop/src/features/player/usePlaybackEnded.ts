//! Playback decisions from the Rust playback coordinator, executed by the
//! player: `playback:ended` (autoplay the next episode) and
//! `playback:recover` (reopen the same item, or another source of the
//! title, after a failure). The coordinator decides; the player only acts on
//! events for its own session, and not while it is already switching to
//! other media (the user's choice wins).

import { useEffect, useRef, type RefObject } from "react";
import { listen } from "@tauri-apps/api/event";

import type { PlaybackEnded, PlaybackRecover, SeriesEpisode } from "@/types";

function usePlaybackEvent<T extends { token: string }>(
  name: string,
  enabled: boolean,
  tokenRef: RefObject<string | null>,
  openingRef: RefObject<boolean>,
  handle: (payload: T) => void
): void {
  const handleRef = useRef(handle);
  handleRef.current = handle;
  useEffect(() => {
    if (!enabled) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen<T>(name, (event) => {
      if (openingRef.current || event.payload.token !== tokenRef.current) return;
      handleRef.current(event.payload);
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [name, enabled, tokenRef, openingRef]);
}

export function usePlaybackEnded(
  enabled: boolean,
  tokenRef: RefObject<string | null>,
  openingRef: RefObject<boolean>,
  openNext: (next: SeriesEpisode) => void
): void {
  usePlaybackEvent<PlaybackEnded>("playback:ended", enabled, tokenRef, openingRef, (e) => {
    if (e.autoplay && e.nextEpisode) openNext(e.nextEpisode);
  });
}

export function usePlaybackRecover(
  enabled: boolean,
  tokenRef: RefObject<string | null>,
  openingRef: RefObject<boolean>,
  recover: (attempt: PlaybackRecover) => void
): void {
  usePlaybackEvent<PlaybackRecover>("playback:recover", enabled, tokenRef, openingRef, recover);
}
