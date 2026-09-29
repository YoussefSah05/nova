//! The player overlay and the ways to start it (`playback.ts`). Closing the
//! player refreshes what watching changes: Continue Watching and resume points.

import { useCallback, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useQueryClient } from "@tanstack/react-query";

import type { MoviePlaybackTarget } from "@/types";
import { PlayerModal, type PlayMedia } from "@/features/player/PlayerModal";
import { browseKeys, type BrowseItem } from "@/features/browse/queries";
import { canonicalMovieQuery, invalidateDetailProgress } from "@/features/browse/detail/data";
import { errorMessage, useSession } from "./session";
import { PlaybackContext, PlayingContext, type Playback } from "./playback";

export function PlaybackProvider({ children }: { children: ReactNode }) {
  const qc = useQueryClient();
  const { profile, activeSourceId, setError } = useSession();
  const [media, setMedia] = useState<PlayMedia | null>(null);

  const resolveMovie = useCallback(
    async (item: BrowseItem, variantId: string | null) => {
      if (!item.entityId) return;
      try {
        const target = await invoke<MoviePlaybackTarget>("resolve_movie_playback", {
          profileId: profile.id,
          entityId: item.entityId,
          variantId,
        });
        setMedia({
          kind: target.mediaKind,
          id: target.mediaId,
          title: item.title,
          subtitle: item.subtitle,
          posterUrl: item.posterUrl,
        });
      } catch (e: unknown) {
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
    (item: BrowseItem, variantId: string) => resolveMovie(item, variantId),
    [resolveMovie]
  );

  const playItem = useCallback(
    async (item: BrowseItem) => {
      if (item.kind === "movie" && item.entityId) {
        await resolveMovie(item, null);
        return;
      }
      setMedia({
        kind: item.kind === "live" ? "live" : "vod",
        id: item.id,
        title: item.title,
        subtitle: item.subtitle,
        posterUrl: item.posterUrl,
        channelGroup: item.groupKey ?? item.channel?.channelGroup ?? null,
        category: item.channel?.category ?? null,
        region: item.channel?.region ?? null,
      });
    },
    [resolveMovie]
  );

  const close = useCallback(() => {
    setMedia(null);
    void qc.invalidateQueries({ queryKey: browseKeys.continueWatching(profile.id) });
    invalidateDetailProgress(qc, profile.id);
    // Shows' watched counts (Continue cards, series pages).
    void qc.invalidateQueries({
      predicate: (q) => q.queryKey[1] === profile.id && q.queryKey[3] === "seriesCards",
    });
  }, [qc, profile.id]);

  const value = useMemo<Playback>(
    () => ({ playItem, playVariant, playMedia: setMedia }),
    [playItem, playVariant]
  );

  return (
    <PlaybackContext.Provider value={value}>
      <PlayingContext.Provider value={media !== null}>{children}</PlayingContext.Provider>
      {media && (
        <PlayerModal
          profileId={profile.id}
          media={media}
          onClose={close}
          sourceId={activeSourceId}
          onPlayMedia={setMedia}
        />
      )}
    </PlaybackContext.Provider>
  );
}
