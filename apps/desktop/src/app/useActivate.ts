//! What opening a card does, everywhere: a series or a title opens its page
//! in the current section's stack; a channel brand bundle opens its channel
//! list; a single channel plays at once.

import { useCallback } from "react";

import type { BrowseItem } from "@/features/browse/queries";
import { useNavActions } from "./navigation/useNav";
import { usePlayback } from "./playback";

export function useActivate(): (item: BrowseItem) => void {
  const { push } = useNavActions();
  const { playItem } = usePlayback();
  return useCallback(
    (item: BrowseItem) => {
      if (item.kind === "series" && item.series) {
        push({ kind: "series", series: item.series });
        return;
      }
      if (
        item.kind === "movie" ||
        item.kind === "vod" ||
        (item.kind === "live" && (item.memberCount ?? 1) > 1)
      ) {
        push({ kind: "title", item });
        return;
      }
      void playItem(item);
    },
    [push, playItem]
  );
}
