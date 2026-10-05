//! Catalog events from Rust that repaint the browse caches: an import
//! finishing (then enrichment and logo matching start), enrichment progress,
//! and directory channel logos landing.

import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { QueryClient } from "@tanstack/react-query";

import type { ImportProgress } from "@/types";
import { invalidateSource } from "@/features/browse/queries";

/// Subscribe for the life of the component; `cancelled` covers a listener
/// that resolves after unmount.
function useTauriEvent<T>(event: string, handler: (payload: T) => void, deps: unknown[]) {
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<T>(event, (e) => handler(e.payload))
      .then((un) => {
        if (cancelled) un();
        else unlisten = un;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
}

export function useCatalogEvents({
  qc,
  profileId,
  activeSourceId,
  onImportError,
  onImportDone,
}: {
  qc: QueryClient;
  profileId: string;
  activeSourceId: number | null;
  onImportError: (message: string) => void;
  onImportDone: () => void;
}) {
  useTauriEvent<ImportProgress>(
    "playlist:import:progress",
    (p) => {
      if (p.sourceId !== activeSourceId) return;
      if (p.error) onImportError(p.error);
      if (!p.done) return;
      onImportDone();
      if (activeSourceId === null) return;
      invalidateSource(qc, profileId, activeSourceId);
      // Enrichment (de-dup + genres) and directory logos; both no-op when off.
      void invoke("start_enrichment", { profileId, sourceId: activeSourceId }).catch(
        () => undefined
      );
      void invoke("match_channel_logos", {
        profileId,
        sourceId: activeSourceId,
        forceRefresh: false,
      }).catch(() => undefined);
    },
    [qc, profileId, activeSourceId]
  );

  // A scheduled (background) refresh: quietly pick up its new catalog. Its
  // progress and failures are not shown: the user did not start it, and the
  // existing catalog stays available when it fails.
  // Source ids are per profile: a refresh another profile started (before the
  // user switched) must not touch this one's caches.
  useTauriEvent<ImportProgress & { profileId: string }>(
    "playlist:import:background",
    (p) => {
      if (p.profileId !== profileId || p.sourceId !== activeSourceId) return;
      if (!p.done || p.error) return;
      invalidateSource(qc, profileId, activeSourceId);
      void invoke("start_enrichment", { profileId, sourceId: activeSourceId }).catch(
        () => undefined
      );
      void invoke("match_channel_logos", {
        profileId,
        sourceId: activeSourceId,
        forceRefresh: false,
      }).catch(() => undefined);
    },
    [qc, profileId, activeSourceId]
  );

  // TMDB matching commits in small, priority-ordered batches: repaint at most
  // every few seconds while it runs, and once when it is done.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    let lastRefresh = 0;
    void listen<{ sourceId: number; done: boolean; stage: string; processed: number }>(
      "catalog:enrich:progress",
      (event) => {
        const p = event.payload;
        if (p.sourceId !== activeSourceId || activeSourceId === null) return;
        const now = Date.now();
        const progressive = p.stage === "matching" && p.processed > 0 && now - lastRefresh > 4000;
        if (p.done || progressive) {
          lastRefresh = now;
          invalidateSource(qc, profileId, activeSourceId);
        }
      }
    )
      .then((un) => {
        if (cancelled) un();
        else unlisten = un;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [qc, profileId, activeSourceId]);

  useTauriEvent<{ done: boolean; stage: string }>(
    "artwork:channel-logos",
    (p) => {
      if (p.done && p.stage === "complete" && activeSourceId !== null) {
        invalidateSource(qc, profileId, activeSourceId);
      }
    },
    [qc, profileId, activeSourceId]
  );
}
