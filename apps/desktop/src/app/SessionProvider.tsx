//! Owns the session (`session.ts`): playlists, the active playlist, refresh,
//! and the cache warm-up that makes every section's first paint a cache read.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useQueryClient } from "@tanstack/react-query";

import type { Profile, Source } from "@/types";
import { prefetchPersonalRails, prefetchSourceOverview } from "@/features/browse/queries";
import { useProfileSettings } from "@/features/settings/useProfileSettings";
import { useAccentTheme } from "@/lib/useAccentTheme";
import {
  errorMessage,
  SessionContext,
  SessionStatusContext,
  type Session,
  type SessionStatus,
} from "./session";
import { useCatalogEvents } from "./useCatalogEvents";
import { onIdle } from "./idle";

export function SessionProvider({
  profile: initialProfile,
  onSwitchProfile,
  children,
}: {
  profile: Profile;
  onSwitchProfile: () => void;
  children: ReactNode;
}) {
  const qc = useQueryClient();
  const [profile, setProfile] = useState(initialProfile);
  useEffect(() => setProfile(initialProfile), [initialProfile]);
  const profileId = initialProfile.id;
  const { settings } = useProfileSettings(profileId);
  useAccentTheme(settings.accentColor);

  const [sources, setSources] = useState<Source[]>([]);
  const [sourcesLoaded, setSourcesLoaded] = useState(false);
  const [sourceTotals, setSourceTotals] = useState<Record<number, number>>({});
  const [activeSourceId, setActiveSourceId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    invoke<Source[]>("list_sources", { profileId })
      .then((next) => {
        if (cancelled) return;
        setError(null);
        setSources(next);
        setSourcesLoaded(true);
        setActiveSourceId((current) =>
          current !== null && next.some((s) => s.id === current) ? current : (next[0]?.id ?? null)
        );
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setSourcesLoaded(true);
        setError(errorMessage(e));
      });
    return () => {
      cancelled = true;
    };
  }, [profileId, reloadKey]);

  // Items per playlist for the switcher labels (decoration: errors ignored).
  useEffect(() => {
    if (sources.length === 0) {
      setSourceTotals({});
      return;
    }
    let cancelled = false;
    Promise.all(
      sources.map(async (source) => {
        const args = { profileId, sourceId: source.id, filters: null };
        const [live, movies, series] = await Promise.all([
          invoke<number>("count_channels", args),
          invoke<number>("count_movies", args),
          invoke<number>("count_series", args),
        ]);
        return [source.id, live + movies + series] as const;
      })
    )
      .then((entries) => {
        if (!cancelled) setSourceTotals(Object.fromEntries(entries));
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [profileId, sources, reloadKey]);

  // Warm the cache: counts, categories and Continue Watching for every
  // section now; the personal rails when the app is idle.
  useEffect(() => {
    if (activeSourceId === null) return;
    prefetchSourceOverview(qc, profileId, activeSourceId);
    return onIdle(() => {
      for (const tab of ["live", "movies", "series"] as const) {
        prefetchPersonalRails(qc, profileId, activeSourceId, tab);
      }
    });
  }, [qc, profileId, activeSourceId, reloadKey]);

  useCatalogEvents({
    qc,
    profileId,
    activeSourceId,
    onImportError: setError,
    onImportDone: () => {
      setRefreshing(false);
      setReloadKey((k) => k + 1);
    },
  });

  // Stable identity: reads the latest state through refs.
  const refreshState = useRef({ activeSourceId, refreshing });
  refreshState.current = { activeSourceId, refreshing };
  const refresh = useCallback(async () => {
    const { activeSourceId: id, refreshing: busy } = refreshState.current;
    if (id === null || busy) return;
    setRefreshing(true);
    setError(null);
    try {
      await invoke("refresh_source", { profileId, sourceId: id });
    } catch (e: unknown) {
      setRefreshing(false);
      setError(errorMessage(e));
    }
  }, [profileId]);

  const sourceAdded = useCallback(async () => {
    try {
      setError(null);
      const next = await invoke<Source[]>("list_sources", { profileId });
      setSources(next);
      setSourcesLoaded(true);
      setActiveSourceId(next.at(-1)?.id ?? null);
    } catch (e: unknown) {
      setError(errorMessage(e));
    }
  }, [profileId]);

  const reloadSources = useCallback(() => setReloadKey((k) => k + 1), []);

  const value = useMemo<Session>(
    () => ({
      profile,
      setProfile,
      switchProfile: onSwitchProfile,
      sources,
      sourcesLoaded,
      activeSourceId,
      setActiveSourceId,
      setError,
      refresh,
      reloadSources,
      sourceAdded,
    }),
    [
      profile,
      onSwitchProfile,
      sources,
      sourcesLoaded,
      activeSourceId,
      refresh,
      reloadSources,
      sourceAdded,
    ]
  );
  const status = useMemo<SessionStatus>(
    () => ({ sourceTotals, error, refreshing }),
    [sourceTotals, error, refreshing]
  );

  return (
    <SessionContext.Provider value={value}>
      <SessionStatusContext.Provider value={status}>{children}</SessionStatusContext.Provider>
    </SessionContext.Provider>
  );
}
