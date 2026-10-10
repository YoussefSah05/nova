//! Warm a title's page while the pointer rests on its card: the movie's
//! detail, its TMDB art and its facts land in the query cache, so opening the
//! page paints complete at once. A short dwell keeps a pointer sweeping
//! across a rail from sending anything.

import { useContext, useEffect, useRef } from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";

import { GuideProfile } from "./guide";
import { canonicalMovieQuery, titleFactsQuery } from "./detail/data";
import { titleArtQuery, titleArtSubject } from "./titleArt";
import type { BrowseItem } from "./queries";

const DWELL_MS = 120;

export function prefetchTitle(qc: QueryClient, profileId: string, item: BrowseItem) {
  const subject = titleArtSubject(item);
  if (!subject) return;
  void qc.prefetchQuery(titleArtQuery(profileId, subject));
  void qc.prefetchQuery(titleFactsQuery(profileId, subject));
  if (subject.kind === "movie") {
    void qc.prefetchQuery({
      ...canonicalMovieQuery(profileId, subject.entityId),
      staleTime: 60_000,
    });
  }
}

/// Pointer handlers for a card: prefetch its page after a short dwell.
export function usePrefetchTitle(item: BrowseItem) {
  const qc = useQueryClient();
  const profileId = useContext(GuideProfile);
  const timer = useRef<number | null>(null);
  const cancel = () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
  };
  useEffect(() => cancel, []);
  return {
    onEnter: (event: { currentTarget: Element }) => {
      if (!profileId || !titleArtSubject(item)) return;
      cancel();
      const card = event.currentTarget;
      if (card.closest(".nova-scrolling, [inert]")) return;
      timer.current = window.setTimeout(() => {
        timer.current = null;
        if (!card.closest(".nova-scrolling, [inert]")) prefetchTitle(qc, profileId, item);
      }, DWELL_MS);
    },
    onLeave: cancel,
  };
}
