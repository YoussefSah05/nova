import { useCallback, useDeferredValue, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import {
  AlertCircleIcon,
  DatabaseIcon,
  PlusIcon,
  RotateCwIcon,
  SearchIcon,
  XIcon,
} from "lucide-react";
import type { ImportProgress, MoviePlaybackTarget, Profile, SeriesSummary, Source } from "@/types";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { SimpleSelect } from "@/components/ui/select";
import { DragRegion } from "@/components/DragRegion";
import { AddSourceModal } from "@/features/source/AddSourceModal";
import { SettingsModal } from "@/features/settings/SettingsModal";
import { Sidebar } from "@/features/browse/Sidebar";
import { FilterPanel } from "@/features/browse/FilterPanel";
import { CommandPalette } from "@/features/browse/CommandPalette";
import { SeriesDetail } from "@/features/browse/SeriesDetail";
import { ContentDetail } from "@/features/browse/ContentDetail";
import { BrowseRails } from "@/features/browse/BrowseRails";
import { BrowseGrid } from "@/features/browse/BrowseGrid";
import { BrowseBoundary } from "@/features/browse/BrowseBoundary";
import {
  browseKeys,
  invalidateMembershipRails,
  invalidateSource,
  patchItemEverywhere,
  prefetchPersonalRails,
  prefetchSourceOverview,
  seriesKey,
  useBrowseCount,
  useCategories,
  useCountries,
  useRegions,
  type BrowseFilters,
  type BrowseItem,
  type BrowseTab,
} from "@/features/browse/queries";
import { PlayerModal, type PlayMedia } from "@/features/player/PlayerModal";
import { useProfileSettings } from "@/features/settings/useProfileSettings";
import { useAccentTheme } from "@/lib/useAccentTheme";
import { cn } from "@/lib/utils";
import { invalidateDetailProgress } from "@/features/browse/detail/data";
import { invokeFavorite, invokeWatchlist } from "@/features/browse/toggles";

type Tab = BrowseTab;

interface HomeProps {
  profile: Profile;
  onSwitchProfile: () => void;
}

const TABS: Tab[] = ["live", "movies", "series"];

export function Home({ profile, onSwitchProfile }: HomeProps) {
  const qc = useQueryClient();
  const [sources, setSources] = useState<Source[]>([]);
  const [sourceTotals, setSourceTotals] = useState<Record<number, number>>({});
  const [activeSourceId, setActiveSourceId] = useState<number | null>(null);
  const [tab, setTab] = useState<Tab>("live");
  const [selectedSeries, setSelectedSeries] = useState<SeriesSummary | null>(null);
  // A movie's "starting screen" (overview + actions) shown before playback.
  const [selectedContent, setSelectedContent] = useState<BrowseItem | null>(null);
  const [playingMedia, setPlayingMedia] = useState<PlayMedia | null>(null);
  const [showAddSource, setShowAddSource] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showPalette, setShowPalette] = useState(false);
  // Local copy of the profile so a rename/recolour in Settings shows immediately.
  const [profileView, setProfileView] = useState(profile);
  useEffect(() => {
    setProfileView(profile);
  }, [profile]);
  // Apply the profile's chosen accent theme to the app's CSS vars (live).
  const { settings: profileSettings } = useProfileSettings(profile.id);
  useAccentTheme(profileSettings.accentColor);
  const [searchQuery, setSearchQuery] = useState("");
  const [countryFilter, setCountryFilter] = useState("");
  const [regionFilter, setRegionFilter] = useState("");
  const [categoryFilter, setCategoryFilter] = useState("");
  const [sourceError, setSourceError] = useState<string | null>(null);
  // Reloads the non-query bits (source list + per-source totals) after a refresh.
  const [reloadKey, setReloadKey] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const deferredSearchQuery = useDeferredValue(searchQuery);

  // Shared scroll container; we remember each tab's scrollTop so switching
  // sections feels continuous.
  const scrollRef = useRef<HTMLElement>(null);
  const scrollByTabRef = useRef<Record<Tab, number>>({ live: 0, movies: 0, series: 0 });

  const counts: Record<Tab, number> = {
    live: useBrowseCount("live", profile.id, activeSourceId).data ?? 0,
    movies: useBrowseCount("movies", profile.id, activeSourceId).data ?? 0,
    series: useBrowseCount("series", profile.id, activeSourceId).data ?? 0,
  };

  const categoryOptions = useCategories(tab, profile.id, activeSourceId).data ?? [];
  const countryOptions = useCountries(profile.id, activeSourceId).data ?? [];
  const regionOptions = useRegions(profile.id, activeSourceId).data ?? [];

  const query = filterParam(deferredSearchQuery);
  const category = filterParam(categoryFilter);
  const country = tab === "live" ? filterParam(countryFilter) : null;
  const region = tab === "live" ? filterParam(regionFilter) : null;
  const filters: BrowseFilters = { query, category, country, region };
  const searchOrFilterActive = Boolean(query || category || country || region);
  // Count of dropdown/genre filters (not search) — drives the Filters badge.
  const activeFilterCount = [category, country, region].filter(Boolean).length;

  // ── Sources ────────────────────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    async function loadSources() {
      try {
        setSourceError(null);
        const nextSources = await invoke<Source[]>("list_sources", { profileId: profile.id });
        if (cancelled) return;
        setSources(nextSources);
        setActiveSourceId((current) => {
          if (current !== null && nextSources.some((source) => source.id === current)) {
            return current;
          }
          return nextSources[0]?.id ?? null;
        });
      } catch (error: unknown) {
        if (!cancelled) setSourceError(errorMessage(error));
      }
    }
    void loadSources();
    return () => {
      cancelled = true;
    };
  }, [profile.id, reloadKey]);

  // Per-source totals for the playlist dropdown labels.
  useEffect(() => {
    if (sources.length === 0) {
      setSourceTotals({});
      return;
    }
    let cancelled = false;
    async function loadSourceTotals() {
      const entries = await Promise.all(
        sources.map(async (source) => {
          const [live, movies, nextSeries] = await Promise.all([
            invoke<number>("count_channels", {
              profileId: profile.id,
              sourceId: source.id,
              filters: null,
            }),
            invoke<number>("count_movies", {
              profileId: profile.id,
              sourceId: source.id,
              filters: null,
            }),
            invoke<number>("count_series", {
              profileId: profile.id,
              sourceId: source.id,
              filters: null,
            }),
          ]);
          return [source.id, live + movies + nextSeries] as const;
        })
      );
      if (!cancelled) setSourceTotals(Object.fromEntries(entries));
    }
    void loadSourceTotals().catch(() => {
      /* counts are decoration; ignore errors */
    });
    return () => {
      cancelled = true;
    };
  }, [profile.id, sources, reloadKey]);

  // Warm the cache for instant tab switches: counts + categories for every tab
  // up front, then the personal rails (the old Shelves cost) on idle.
  useEffect(() => {
    if (activeSourceId === null) return;
    prefetchSourceOverview(qc, profile.id, activeSourceId);
    const cancel = onIdle(() => {
      for (const t of TABS) prefetchPersonalRails(qc, profile.id, activeSourceId, t);
    });
    return cancel;
  }, [qc, profile.id, activeSourceId, reloadKey]);

  // While a refresh runs, watch import progress; on completion repaint.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<ImportProgress>("playlist:import:progress", (event) => {
      const p = event.payload;
      if (p.sourceId !== activeSourceId) return;
      if (p.error) setSourceError(p.error);
      if (p.done) {
        setRefreshing(false);
        setReloadKey((k) => k + 1);
        if (activeSourceId !== null) {
          invalidateSource(qc, profile.id, activeSourceId);
          // Kick off catalog enrichment (de-dup + genre fill). No-ops if the
          // embedding model isn't installed; results stream in via the listener.
          void invoke("start_enrichment", { profileId: profile.id, sourceId: activeSourceId });
          // Fill missing channel logos from the opt-in directory (no-op when off).
          void invoke("match_channel_logos", {
            profileId: profile.id,
            sourceId: activeSourceId,
            forceRefresh: false,
          });
        }
      }
    }).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [qc, profile.id, activeSourceId]);

  // Enrichment runs in the background; refresh the browse caches when it finishes
  // so de-duplicated cards + filled-in genres appear without a manual reload.
  // TMDB matching commits in small, priority-ordered batches, so also refresh
  // (at most every few seconds) while it runs: posters for favourites and
  // in-progress titles appear first.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    let lastRefresh = 0;
    void listen<{ sourceId: number; done: boolean; stage: string; processed: number }>(
      "catalog:enrich:progress",
      (event) => {
        const p = event.payload;
        if (p.sourceId !== activeSourceId) return;
        const now = Date.now();
        const progressive = p.stage === "matching" && p.processed > 0 && now - lastRefresh > 4000;
        if (p.done || progressive) {
          lastRefresh = now;
          invalidateSource(qc, profile.id, activeSourceId);
        }
      }
    ).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [qc, profile.id, activeSourceId]);

  // Directory channel logos land in one write per source: repaint when done.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<{ done: boolean; stage: string }>("artwork:channel-logos", (event) => {
      if (event.payload.done && event.payload.stage === "complete" && activeSourceId !== null) {
        invalidateSource(qc, profile.id, activeSourceId);
      }
    }).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [qc, profile.id, activeSourceId]);

  // Restore the saved scroll position when entering a tab.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = scrollByTabRef.current[tab] ?? 0;
  }, [tab]);

  // Global ⌘K / Ctrl+K toggles the command palette.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setShowPalette((v) => !v);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // ── Actions ──────────────────────────────────────────────────────────────────
  function handleTabChange(nextTab: Tab) {
    if (nextTab === tab) return;
    setTab(nextTab);
    resetBrowseState();
  }

  function handleSourceChange(value: string) {
    setActiveSourceId(Number(value));
    resetBrowseState();
  }

  function resetBrowseState() {
    setSearchQuery("");
    setCountryFilter("");
    setRegionFilter("");
    setCategoryFilter("");
    setSelectedSeries(null);
    setSelectedContent(null);
  }

  async function handleSourceAdded() {
    setShowAddSource(false);
    try {
      setSourceError(null);
      const nextSources = await invoke<Source[]>("list_sources", { profileId: profile.id });
      setSources(nextSources);
      setActiveSourceId(nextSources.at(-1)?.id ?? null);
      resetBrowseState();
    } catch (error: unknown) {
      setSourceError(errorMessage(error));
    }
  }

  async function handleRefresh() {
    if (activeSourceId === null || refreshing) return;
    setRefreshing(true);
    setSourceError(null);
    try {
      await invoke("refresh_source", { profileId: profile.id, sourceId: activeSourceId });
    } catch (error: unknown) {
      setRefreshing(false);
      setSourceError(errorMessage(error));
    }
  }

  const playLegacyItem = useCallback((item: BrowseItem) => {
    setPlayingMedia({
      kind: item.kind === "live" ? "live" : "vod",
      id: item.id,
      title: item.title,
      subtitle: item.subtitle,
      posterUrl: item.posterUrl,
      channelGroup: item.groupKey ?? item.channel?.channelGroup ?? null,
      category: item.channel?.category ?? null,
      region: item.channel?.region ?? null,
    });
  }, []);

  const playItem = useCallback(
    async (item: BrowseItem) => {
      // Canonical movies: Rust picks the variant and returns a transient
      // player locator; progress written against it lands on the entity.
      if (item.kind === "movie" && item.entityId) {
        try {
          const target = await invoke<MoviePlaybackTarget>("resolve_movie_playback", {
            profileId: profile.id,
            entityId: item.entityId,
            variantId: null,
          });
          setPlayingMedia({
            kind: target.mediaKind,
            id: target.mediaId,
            title: item.title,
            subtitle: item.subtitle,
            posterUrl: item.posterUrl,
          });
        } catch (error: unknown) {
          setSourceError(errorMessage(error));
        }
        return;
      }
      playLegacyItem(item);
    },
    [profile.id, playLegacyItem]
  );

  // Closing the player refreshes every tab's Continue Watching rail (progress,
  // finished episodes and "Up next" change while watching).
  const closePlayer = useCallback(() => {
    setPlayingMedia(null);
    void qc.invalidateQueries({ queryKey: browseKeys.continueWatching(profile.id) });
    invalidateDetailProgress(qc, profile.id);
  }, [qc, profile.id]);

  const activate = useCallback(
    (item: BrowseItem) => {
      if (item.kind === "series" && item.series) {
        setSelectedSeries(item.series);
        return;
      }
      // Movies open an overview screen; live brand bundles open their channel
      // list; a single live channel plays immediately.
      if (
        item.kind === "movie" ||
        item.kind === "vod" ||
        (item.kind === "live" && (item.memberCount ?? 1) > 1)
      ) {
        setSelectedContent(item);
        return;
      }
      void playItem(item);
    },
    [playItem]
  );

  // A title opened from a detail page ("More like this", a collection) shows
  // at the top, replacing the page (Back returns to the browse view).
  const openFromDetail = useCallback(
    (item: BrowseItem) => {
      window.scrollTo({ top: 0 });
      if (item.kind === "series" && item.series) {
        setSelectedContent(null);
        setSelectedSeries(item.series);
        return;
      }
      setSelectedSeries(null);
      activate(item);
    },
    [activate]
  );

  const toggleFavorite = useCallback(
    async (item: BrowseItem, next: boolean) => {
      if (activeSourceId === null) return;
      const rollback = patchItemEverywhere(qc, profile.id, activeSourceId, item.key, {
        isFavorite: next,
      });
      setSelectedContent((current) =>
        current?.key === item.key ? { ...current, isFavorite: next } : current
      );
      if (item.kind === "series") setSelectedSeries(patchSelected(item, { isFavorite: next }));
      try {
        await invokeFavorite(item, profile.id, next);
        invalidateMembershipRails(qc, profile.id, activeSourceId, tabForKind(item.kind));
        void qc.invalidateQueries({ queryKey: browseKeys.continueWatching(profile.id) });
      } catch (error: unknown) {
        rollback();
        setSelectedContent((current) =>
          current?.key === item.key ? { ...current, isFavorite: !next } : current
        );
        if (item.kind === "series") setSelectedSeries(patchSelected(item, { isFavorite: !next }));
        setSourceError(errorMessage(error));
      }
    },
    [qc, profile.id, activeSourceId]
  );

  const toggleWatchlist = useCallback(
    async (item: BrowseItem, next: boolean) => {
      if (activeSourceId === null) return;
      const rollback = patchItemEverywhere(qc, profile.id, activeSourceId, item.key, {
        isInWatchlist: next,
      });
      setSelectedContent((current) =>
        current?.key === item.key ? { ...current, isInWatchlist: next } : current
      );
      if (item.kind === "series") setSelectedSeries(patchSelected(item, { isInWatchlist: next }));
      try {
        await invokeWatchlist(item, profile.id, next);
        invalidateMembershipRails(qc, profile.id, activeSourceId, tabForKind(item.kind));
      } catch (error: unknown) {
        rollback();
        setSelectedContent((current) =>
          current?.key === item.key ? { ...current, isInWatchlist: !next } : current
        );
        if (item.kind === "series")
          setSelectedSeries(patchSelected(item, { isInWatchlist: !next }));
        setSourceError(errorMessage(error));
      }
    },
    [qc, profile.id, activeSourceId]
  );

  // Series-detail toggles operate on the selected SeriesSummary directly.
  const toggleSeriesFavorite = useCallback(
    (s: SeriesSummary, next: boolean) => toggleFavorite(seriesItem(s), next),
    [toggleFavorite]
  );
  const toggleSeriesWatchlist = useCallback(
    (s: SeriesSummary, next: boolean) => toggleWatchlist(seriesItem(s), next),
    [toggleWatchlist]
  );

  if (selectedSeries) {
    return (
      <>
        <SeriesDetail
          key={seriesKey(selectedSeries)}
          profileId={profile.id}
          series={selectedSeries}
          onOpenTitle={openFromDetail}
          onToggleItemFavorite={toggleFavorite}
          onToggleItemWatchlist={toggleWatchlist}
          onBack={() => setSelectedSeries(null)}
          onToggleFavorite={(next) => toggleSeriesFavorite(selectedSeries, next)}
          onToggleWatchlist={(next) => toggleSeriesWatchlist(selectedSeries, next)}
          onPlayEpisode={(episode) =>
            setPlayingMedia({
              kind: "episode",
              id: episode.id,
              title: selectedSeries.displayTitle,
              subtitle: episode.label,
              posterUrl: episode.stillUrl ?? selectedSeries.posterUrl,
            })
          }
        />
        {playingMedia && (
          <PlayerModal
            profileId={profile.id}
            media={playingMedia}
            onClose={closePlayer}
            sourceId={activeSourceId}
            onPlayMedia={setPlayingMedia}
          />
        )}
      </>
    );
  }

  if (selectedContent) {
    return (
      <>
        <ContentDetail
          key={selectedContent.key}
          onOpenTitle={openFromDetail}
          profileId={profile.id}
          sourceId={activeSourceId}
          item={selectedContent}
          onBack={() => setSelectedContent(null)}
          onPlay={playItem}
          onToggleFavorite={toggleFavorite}
          onToggleWatchlist={toggleWatchlist}
        />
        {playingMedia && (
          <PlayerModal
            profileId={profile.id}
            media={playingMedia}
            onClose={closePlayer}
            sourceId={activeSourceId}
            onPlayMedia={setPlayingMedia}
          />
        )}
      </>
    );
  }

  return (
    <div className="flex h-screen overflow-hidden bg-background text-foreground">
      {sources.length > 0 && (
        <Sidebar
          tab={tab}
          onTabChange={handleTabChange}
          counts={counts}
          profile={profileView}
          onSettings={() => setShowSettings(true)}
          onSwitchProfile={onSwitchProfile}
        />
      )}
      <main className="flex min-h-0 min-w-0 flex-1 flex-col">
        {sources.length === 0 ? (
          <>
            <DragRegion />
            <div className="flex flex-1 flex-col items-center justify-center gap-4 px-6 text-center">
              <DatabaseIcon className="h-10 w-10 text-muted-foreground" aria-hidden />
              <div>
                <p className="text-sm font-medium text-foreground">No playlists yet</p>
                <p className="mt-1 max-w-sm text-sm text-muted-foreground">
                  Import an M3U or Xtream playlist to browse live channels, movies, and series.
                </p>
              </div>
              {sourceError && <ErrorMessage message={sourceError} />}
              <Button onClick={() => setShowAddSource(true)}>Add playlist</Button>
            </div>
          </>
        ) : (
          <>
            <header data-tauri-drag-region className="flex items-center gap-2 px-6 py-3">
              <div className="relative max-w-md flex-1">
                <SearchIcon className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                  placeholder={`Search ${tab}`}
                  className="pl-9"
                />
              </div>
              <div className="ml-auto flex items-center gap-2">
                <SimpleSelect
                  value={activeSourceId != null ? String(activeSourceId) : ""}
                  options={sources.map((source) => ({
                    value: String(source.id),
                    label:
                      sourceTotals[source.id] !== undefined
                        ? `${source.name} (${formatCount(sourceTotals[source.id])})`
                        : source.name,
                  }))}
                  onChange={handleSourceChange}
                  ariaLabel="Playlist"
                  className="h-10 min-w-[180px]"
                />
                <FilterPanel
                  tab={tab}
                  regionFilter={regionFilter}
                  setRegionFilter={setRegionFilter}
                  regionOptions={regionOptions}
                  countryFilter={countryFilter}
                  setCountryFilter={setCountryFilter}
                  countryOptions={countryOptions}
                  categoryFilter={categoryFilter}
                  setCategoryFilter={setCategoryFilter}
                  categoryOptions={categoryOptions}
                  activeCount={activeFilterCount}
                />
                <Button
                  variant="outline"
                  size="icon"
                  onClick={handleRefresh}
                  disabled={activeSourceId === null || refreshing}
                  aria-label="Refresh"
                  title="Re-fetch this playlist from the provider"
                >
                  <RotateCwIcon
                    className={cn("h-4 w-4", refreshing && "animate-spin")}
                    aria-hidden
                  />
                </Button>
                <Button onClick={() => setShowAddSource(true)} title="Add playlist">
                  <PlusIcon className="h-4 w-4" aria-hidden />
                  Add
                </Button>
              </div>
            </header>

            {searchOrFilterActive && (
              <div className="flex flex-wrap items-center gap-1.5 px-6 pb-2 text-xs">
                {searchQuery && (
                  <FilterChip
                    label="Search"
                    value={searchQuery}
                    onClear={() => setSearchQuery("")}
                  />
                )}
                {regionFilter && (
                  <FilterChip
                    label="Region"
                    value={regionFilter}
                    onClear={() => setRegionFilter("")}
                  />
                )}
                {countryFilter && (
                  <FilterChip
                    label="Country"
                    value={countryFilter}
                    onClear={() => setCountryFilter("")}
                  />
                )}
                {categoryFilter && (
                  <FilterChip
                    label="Category"
                    value={categoryFilter}
                    onClear={() => setCategoryFilter("")}
                  />
                )}
                <Button
                  variant="link"
                  size="sm"
                  onClick={resetBrowseState}
                  className="h-auto px-1 py-0 text-xs"
                >
                  Clear all
                </Button>
              </div>
            )}

            <section
              ref={scrollRef}
              onScroll={(event) => {
                scrollByTabRef.current[tab] = event.currentTarget.scrollTop;
              }}
              className="relative flex-1 overflow-y-auto px-6 pb-6"
            >
              {sourceError && <ErrorMessage message={sourceError} />}
              {activeSourceId !== null && (
                <BrowseBoundary resetKey={`${tab}-${activeSourceId}-${searchOrFilterActive}`}>
                  <div key={`${tab}-${searchOrFilterActive}`} className="nova-fade-in">
                    {searchOrFilterActive ? (
                      <BrowseGrid
                        tab={tab}
                        profileId={profile.id}
                        sourceId={activeSourceId}
                        filters={filters}
                        scrollRef={scrollRef}
                        onActivate={activate}
                        onToggleFavorite={toggleFavorite}
                        onToggleWatchlist={toggleWatchlist}
                      />
                    ) : (
                      <BrowseRails
                        tab={tab}
                        profileId={profile.id}
                        sourceId={activeSourceId}
                        scrollRef={scrollRef}
                        onPlay={setPlayingMedia}
                        onSelectCategory={setCategoryFilter}
                        onActivate={activate}
                        onToggleFavorite={toggleFavorite}
                        onToggleWatchlist={toggleWatchlist}
                      />
                    )}
                  </div>
                </BrowseBoundary>
              )}
            </section>
          </>
        )}
      </main>

      <CommandPalette
        open={showPalette}
        onOpenChange={setShowPalette}
        profileId={profile.id}
        sourceId={activeSourceId}
        onActivate={activate}
        onGoToTab={handleTabChange}
        onOpenSettings={() => setShowSettings(true)}
        onAddPlaylist={() => setShowAddSource(true)}
        onSwitchProfile={onSwitchProfile}
        onRefresh={handleRefresh}
      />

      {showAddSource && (
        <AddSourceModal
          profileId={profile.id}
          onDone={handleSourceAdded}
          onCancel={() => setShowAddSource(false)}
        />
      )}

      {showSettings && (
        <SettingsModal
          profile={profileView}
          onClose={() => setShowSettings(false)}
          onSwitchProfile={onSwitchProfile}
          onProfileUpdated={setProfileView}
          onSourcesChanged={() => setReloadKey((k) => k + 1)}
        />
      )}

      {playingMedia && (
        <PlayerModal
          profileId={profile.id}
          media={playingMedia}
          onClose={closePlayer}
          sourceId={activeSourceId}
          onPlayMedia={setPlayingMedia}
        />
      )}
    </div>
  );
}

// ── Toggle plumbing ─────────────────────────────────────────────────────────

function tabForKind(kind: BrowseItem["kind"]): BrowseTab {
  return kind === "live" ? "live" : kind === "vod" || kind === "movie" ? "movies" : "series";
}

function seriesItem(s: SeriesSummary): BrowseItem {
  return {
    key: seriesKey(s),
    kind: "series",
    id: s.sourceId,
    title: s.seriesTitle,
    subtitle: s.category,
    posterUrl: s.posterUrl,
    tvgId: null,
    isFavorite: s.isFavorite,
    isInWatchlist: s.isInWatchlist,
    series: s,
  };
}

function patchSelected(
  item: BrowseItem,
  patch: Partial<Pick<SeriesSummary, "isFavorite" | "isInWatchlist">>
): (cur: SeriesSummary | null) => SeriesSummary | null {
  return (cur) =>
    cur && item.series && seriesKey(cur) === seriesKey(item.series) ? { ...cur, ...patch } : cur;
}

// ── Idle scheduling (WebKit has no requestIdleCallback) ───────────────────────

function onIdle(cb: () => void): () => void {
  if (typeof requestIdleCallback === "function") {
    const id = requestIdleCallback(cb);
    return () => cancelIdleCallback(id);
  }
  const id = setTimeout(cb, 200);
  return () => clearTimeout(id);
}

// ── Filter chrome (presentational) ────────────────────────────────────────────

function FilterChip({
  label,
  value,
  onClear,
}: {
  label: string;
  value: string;
  onClear: () => void;
}) {
  return (
    <Badge variant="outline" className="gap-1 py-1 pl-2.5 pr-1 font-normal">
      <span className="text-muted-foreground">{label}:</span>
      <span className="max-w-[10rem] truncate font-medium text-foreground">{value}</span>
      <button
        type="button"
        onClick={onClear}
        aria-label={`Clear ${label} filter`}
        className="ml-0.5 flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <XIcon className="h-3 w-3" aria-hidden />
      </button>
    </Badge>
  );
}

function ErrorMessage({ message }: { message: string }) {
  return (
    <div className="mb-3 flex items-start gap-2 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive-foreground">
      <AlertCircleIcon className="mt-0.5 h-4 w-4 flex-shrink-0" aria-hidden />
      <span>{message}</span>
    </div>
  );
}

function filterParam(value: string): string | null {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function formatCount(value: number): string {
  return new Intl.NumberFormat().format(value);
}

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (typeof error === "object" && error !== null) {
    const maybeMessage = "message" in error ? error.message : null;
    if (typeof maybeMessage === "string") return maybeMessage;
  }
  return "Something went wrong while loading imported content.";
}
