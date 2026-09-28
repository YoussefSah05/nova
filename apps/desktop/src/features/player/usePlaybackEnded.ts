//! Playback decisions from the Rust playback coordinator, executed by the
//! player: `playback:ended` (autoplay the next episode) and
//! `playback:recover` (reopen the same item, or another source of the
//! title, after a failure). The coordinator decides; the player only acts on
//! events for the session it holds (`token` is null while an open is in
//! flight), so an event for a session the user already left never applies.

import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";

import type { PlaybackEnded, PlaybackRecover, SeriesEpisode } from "@/types";

/// At most this many events wait for an open to settle (the newest win).
const MAX_BUFFERED = 4;

function usePlaybackEvent<T extends { token: string }>(
  name: string,
  enabled: boolean,
  token: string | null,
  handle: (payload: T) => void
): void {
  const handleRef = useRef(handle);
  handleRef.current = handle;
  const tokenRef = useRef(token);
  tokenRef.current = token;
  // Events for a token the player doesn't hold yet: a very fast failure can
  // be announced before the open that created the session has returned.
  // They're handled when that token arrives; events for a session the user
  // already left never match and are dropped.
  const buffered = useRef(new Map<string, T>());

  useEffect(() => {
    if (!enabled) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen<T>(name, (event) => {
      const payload = event.payload;
      if (tokenRef.current !== null && payload.token === tokenRef.current) {
        handleRef.current(payload);
        return;
      }
      buffered.current.set(payload.token, payload);
      while (buffered.current.size > MAX_BUFFERED) {
        const oldest = buffered.current.keys().next().value;
        if (oldest === undefined) break;
        buffered.current.delete(oldest);
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [name, enabled]);

  useEffect(() => {
    if (!enabled || token === null) return;
    const waiting = buffered.current.get(token);
    buffered.current.clear();
    if (waiting) handleRef.current(waiting);
  }, [enabled, token]);
}

export function usePlaybackEnded(
  enabled: boolean,
  token: string | null,
  openNext: (next: SeriesEpisode) => void
): void {
  usePlaybackEvent<PlaybackEnded>("playback:ended", enabled, token, (e) => {
    if (e.autoplay && e.nextEpisode) openNext(e.nextEpisode);
  });
}

export function usePlaybackRecover(
  enabled: boolean,
  token: string | null,
  recover: (attempt: PlaybackRecover) => void
): void {
  usePlaybackEvent<PlaybackRecover>("playback:recover", enabled, token, recover);
}
