//! Audio metadata belongs to a playable source, not to the film's original
//! language or the profile's preference. Probe only the visible detail page.

import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";

import { useScreenActive } from "@/lib/screenActive";
import type { AudioLanguages, MoviePlaybackTarget } from "@/types";

export interface AudioTarget {
  mediaKind: "vod" | "episode" | "live";
  mediaId: number;
}

export function useAudioLanguages(profileId: string, target: AudioTarget | null) {
  const active = useScreenActive();
  return useQuery({
    subscribed: active,
    queryKey: ["audio-languages", profileId, target?.mediaKind, target?.mediaId],
    queryFn: () => {
      if (!target) throw new Error("No playable audio source");
      return invoke<AudioLanguages>("get_audio_languages", { profileId, ...target });
    },
    enabled: active && target !== null,
    staleTime: 10 * 60 * 1000,
    gcTime: 30 * 60 * 1000,
    retry: false,
  });
}

/// Resolve exactly the version marked as Nova's pick in the current detail.
/// The returned provider row is transient, so refresh it when the page opens.
export function useMovieAudioTarget(
  profileId: string,
  entityId: string | null | undefined,
  variantId: string | null | undefined,
  ready: boolean
) {
  const active = useScreenActive();
  return useQuery({
    subscribed: active,
    queryKey: ["movie-audio-target", profileId, entityId, variantId],
    queryFn: async (): Promise<AudioTarget> => {
      const target = await invoke<MoviePlaybackTarget>("resolve_movie_playback", {
        profileId,
        entityId,
        variantId: variantId ?? null,
      });
      if (target.mediaKind !== "vod") throw new Error("No playable movie audio source");
      return { mediaKind: "vod", mediaId: target.mediaId };
    },
    enabled: active && !!entityId && ready,
    staleTime: 0,
    retry: false,
  });
}
