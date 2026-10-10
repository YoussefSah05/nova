//! The player overlay and the ways to start it (`playback.ts`). Closing the
//! player refreshes what watching changes: Continue Watching and resume points.

import { useCallback, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useQueryClient } from "@tanstack/react-query";

import type { MoviePlaybackTarget } from "@/types";
import { PlayerModal, type PlayMedia } from "@/features/player/PlayerModal";
import { browseKeys, type BrowseItem } from "@/features/browse/queries";
import { canonicalMovieQuery, invalidateDetailProgress } from "@/features/browse/detail/data";
import { errorMessage, useSession } from "./session";
import { PlaybackContext, PlayingContext, type Playback } from "./playback";
import type { PlaybackLaunchOptions } from "@/features/player/playbackLaunch";

export function PlaybackProvider({ children }: { children: ReactNode }) {
  const qc = useQueryClient();
  const { profile, activeSourceId, setError } = useSession();
  const sequence = useRef(0);
  const [request, setRequest] = useState<{
    media: PlayMedia;
    options?: PlaybackLaunchOptions;
    id: number;
  } | null>(null);

  const playMedia = useCallback((media: PlayMedia, options?: PlaybackLaunchOptions) => {
    setRequest({ media, options, id: ++sequence.current });
  }, []);

  // Automatic episode/recovery changes reuse the player and its manual caption
  // choice, but an audio ordinal belongs only to the original source.
  const continueMedia = useCallback((media: PlayMedia) => {
    setRequest((previous) =>
      previous
        ? {
            ...previous,
            media,
            options: previous.options ? { ...previous.options, audioTrack: undefined } : undefined,
          }
        : null
    );
  }, []);

  const resolveMovie = useCallback(
    async (item: BrowseItem, variantId: string | null, options?: PlaybackLaunchOptions) => {
      if (!item.entityId) return;
      const id = ++sequence.current;
      try {
        const target = await invoke<MoviePlaybackTarget>("resolve_movie_playback", {
          profileId: profile.id,
          entityId: item.entityId,
          variantId,
        });
        if (id !== sequence.current) return;
        setRequest({
          id,
          options,
          media: {
            kind: target.mediaKind,
            id: target.mediaId,
            title: item.title,
            subtitle: item.subtitle,
            posterUrl: item.posterUrl,
          },
        });
      } catch (e: unknown) {
        if (id !== sequence.current) return;
        setError(errorMessage(e));
        // A chosen source that vanished: show the page's current sources.
        if (variantId)
          void qc.invalidateQueries({
            queryKey: canonicalMovieQuery(profile.id, item.entityId).queryKey,
          });
      }
    },
    [profile.id, setError, qc]
  );

  const playVariant = useCallback(
    (item: BrowseItem, variantId: string, options?: PlaybackLaunchOptions) =>
      resolveMovie(item, variantId, options),
    [resolveMovie]
  );

  const playItem = useCallback(
    async (item: BrowseItem, options?: PlaybackLaunchOptions) => {
      if (item.kind === "movie" && item.entityId) {
        await resolveMovie(item, null, options);
        return;
      }
      playMedia(
        {
          kind: item.kind === "live" ? "live" : "vod",
          id: item.id,
          title: item.title,
          subtitle: item.subtitle,
          posterUrl: item.posterUrl,
          channelGroup: item.groupKey ?? item.channel?.channelGroup ?? null,
          category: item.channel?.category ?? null,
          region: item.channel?.region ?? null,
        },
        options
      );
    },
    [resolveMovie, playMedia]
  );

  const refreshProgress = useCallback(() => {
    void qc.invalidateQueries({ queryKey: browseKeys.continueWatching(profile.id) });
    invalidateDetailProgress(qc, profile.id);
    // Shows' watched counts (Continue cards, series pages, the wall).
    void qc.invalidateQueries({
      predicate: (q) =>
        q.queryKey[1] === profile.id &&
        (q.queryKey[3] === "history" ||
          q.queryKey[3] === "seriesCards" ||
          (q.queryKey[3] === "grid" && q.queryKey[4] === "series")),
    });
  }, [qc, profile.id]);

  const close = useCallback(() => {
    ++sequence.current;
    setRequest(null);
    // Also refresh failed opens; successful sessions refresh again after their
    // asynchronous final progress write through onProgressSaved.
    refreshProgress();
  }, [refreshProgress]);

  const value = useMemo<Playback>(
    () => ({ playItem, playVariant, playMedia }),
    [playItem, playVariant, playMedia]
  );

  return (
    <PlaybackContext.Provider value={value}>
      <PlayingContext.Provider value={request !== null}>{children}</PlayingContext.Provider>
      {request && (
        <PlayerModal
          key={request.id}
          profileId={profile.id}
          media={request.media}
          launchOptions={request.options}
          onClose={close}
          onProgressSaved={refreshProgress}
          sourceId={activeSourceId}
          onPlayMedia={continueMedia}
        />
      )}
    </PlaybackContext.Provider>
  );
}
