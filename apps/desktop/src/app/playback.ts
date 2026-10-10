//! Starting playback from anywhere in the shell. Provided by
//! `PlaybackProvider.tsx`, which owns the player overlay.

import { createContext, useContext } from "react";

import type { PlayMedia } from "@/features/player/PlayerModal";
import type { BrowseItem } from "@/features/browse/queries";
import type { PlaybackLaunchOptions } from "@/features/player/playbackLaunch";

export interface Playback {
  /// Play a catalog card: Rust picks a canonical movie's source.
  playItem: (item: BrowseItem, options?: PlaybackLaunchOptions) => Promise<void>;
  /// Play one chosen source of a canonical movie (Rust checks it belongs).
  playVariant: (
    item: BrowseItem,
    variantId: string,
    options?: PlaybackLaunchOptions
  ) => Promise<void>;
  /// Play resolved media (an episode, a channel, a Continue Watching card).
  playMedia: (media: PlayMedia, options?: PlaybackLaunchOptions) => void;
}

/// Stable actions: cards that start playback never re-render because the
/// player opened or closed.
export const PlaybackContext = createContext<Playback | null>(null);
/// Whether the player is open (the shell pauses navigation shortcuts).
export const PlayingContext = createContext(false);

export function usePlaying(): boolean {
  return useContext(PlayingContext);
}

export function usePlayback(): Playback {
  const playback = useContext(PlaybackContext);
  if (!playback) throw new Error("usePlayback outside PlaybackProvider");
  return playback;
}
