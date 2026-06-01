import { memo, useCallback, useDeferredValue, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import {
  AlertCircleIcon,
  DatabaseIcon,
  ListFilterIcon,
  PlusIcon,
  RotateCwIcon,
  SearchIcon,
  SettingsIcon,
  XIcon,
} from "lucide-react";
import type { ImportProgress, Profile, SeriesSummary, Source } from "@/types";
import { AddSourceModal } from "@/features/source/AddSourceModal";
import { SettingsModal } from "@/features/settings/SettingsModal";
import { SeriesDetail } from "@/features/browse/SeriesDetail";
import { BrowseRails } from "@/features/browse/BrowseRails";
import { BrowseGrid } from "@/features/browse/BrowseGrid";
import {
  browseKeys,
  invalidateMembershipRails,
  invalidateSource,
  patchItemEverywhere,
  prefetchPersonalRails,
  prefetchSourceOverview,
  useBrowseCount,
  useCategories,
  useCountries,
  type BrowseFilters,
  type BrowseItem,
  type BrowseTab,
} from "@/features/browse/queries";
import { PlayerModal, type PlayMedia } from "@/features/player/PlayerModal";
import { cn } from "@/lib/utils";

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
  const [playingMedia, setPlayingMedia] = useState<PlayMedia | null>(null);
  const [showAddSource, setShowAddSource] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  // Local copy of the profile so a rename/recolour in Settings shows immediately.
  const [profileView, setProfileView] = useState(profile);
  useEffect(() => {
    setProfileView(profile);
  }, [profile]);
  const [searchQuery, setSearchQuery] = useState("");
  const [countryFilter, setCountryFilter] = useState("");
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

  const activeSource = sources.find((source) => source.id === activeSourceId) ?? null;

  const counts: Record<Tab, number> = {
    live: useBrowseCount("live", profile.id, activeSourceId).data ?? 0,
    movies: useBrowseCount("movies", profile.id, activeSourceId).data ?? 0,
    series: useBrowseCount("series", profile.id, activeSourceId).data ?? 0,
  };

  const categoryOptions = useCategories(tab, profile.id, activeSourceId).data ?? [];
  const countryOptions = useCountries(profile.id, activeSourceId).data ?? [];

  const query = filterParam(deferredSearchQuery);
  const category = filterParam(categoryFilter);
  const country = tab === "live" ? filterParam(countryFilter) : null;
  const filters: BrowseFilters = { query, category, country };
  const searchOrFilterActive = Boolean(query || category || country);

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
            invoke<number>("count_channels", { profileId: profile.id, sourceId: source.id, filters: null }),
            invoke<number>("count_vod", { profileId: profile.id, sourceId: source.id, filters: null }),
            invoke<number>("count_series", { profileId: profile.id, sourceId: source.id, filters: null }),
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
        if (activeSourceId !== null) invalidateSource(qc, profile.id, activeSourceId);
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
    setCategoryFilter("");
    setSelectedSeries(null);
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

  const activate = useCallback((item: BrowseItem) => {
    if (item.kind === "series" && item.series) {
      setSelectedSeries(item.series);
      return;
    }
    setPlayingMedia({
      kind: item.kind === "live" ? "live" : "vod",
      id: item.id,
      title: item.title,
      subtitle: item.subtitle,
      posterUrl: item.posterUrl,
    });
  }, []);

  const toggleFavorite = useCallback(
    async (item: BrowseItem, next: boolean) => {
      if (activeSourceId === null) return;
      const rollback = patchItemEverywhere(qc, profile.id, activeSourceId, item.key, {
        isFavorite: next,
      });
      if (item.kind === "series") setSelectedSeries(patchSelected(item, { isFavorite: next }));
      try {
        await invokeFavorite(item, profile.id, next);
        invalidateMembershipRails(qc, profile.id, activeSourceId, tabForKind(item.kind));
        void qc.invalidateQueries({ queryKey: browseKeys.continueWatching(profile.id) });
      } catch (error: unknown) {
        rollback();
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
      if (item.kind === "series") setSelectedSeries(patchSelected(item, { isInWatchlist: next }));
      try {
        await invokeWatchlist(item, profile.id, next);
        invalidateMembershipRails(qc, profile.id, activeSourceId, tabForKind(item.kind));
      } catch (error: unknown) {
        rollback();
        if (item.kind === "series") setSelectedSeries(patchSelected(item, { isInWatchlist: !next }));
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
        <div className="min-h-screen bg-neutral-950 p-6">
          <SeriesDetail
            profileId={profile.id}
            series={selectedSeries}
            onBack={() => setSelectedSeries(null)}
            onToggleFavorite={(next) => toggleSeriesFavorite(selectedSeries, next)}
            onToggleWatchlist={(next) => toggleSeriesWatchlist(selectedSeries, next)}
            onPlayEpisode={(episode) =>
              setPlayingMedia({
                kind: "episode",
                id: episode.id,
                title: selectedSeries.seriesTitle,
                subtitle: `S${String(episode.season).padStart(2, "0")}E${String(episode.episode).padStart(2, "0")}${episode.episodeTitle ? ` — ${episode.episodeTitle}` : ""}`,
                posterUrl: episode.logoUrl ?? selectedSeries.posterUrl,
              })
            }
          />
        </div>
        {playingMedia && (
          <PlayerModal
            profileId={profile.id}
            media={playingMedia}
            onClose={() => setPlayingMedia(null)}
            sourceId={activeSourceId}
            onPlayMedia={setPlayingMedia}
          />
        )}
      </>
    );
  }

  return (
    <div className="flex min-h-screen flex-col bg-neutral-950">
      <header
        data-tauri-drag-region
        className="flex items-center justify-between border-b border-white/5 py-4 pl-20 pr-6"
      >
        <div>
          <span className="text-lg font-semibold text-white">Nova</span>
          {activeSource && (
            <p className="mt-0.5 text-xs text-white/35">
              Browsing {formatCount(counts.live + counts.movies + counts.series)} imported items
            </p>
          )}
        </div>
        <div className="flex items-center gap-1">
          <button
            onClick={() => setShowSettings(true)}
            aria-label="Settings"
            title="Settings"
            className="flex h-9 w-9 items-center justify-center rounded-full text-white/60 transition-colors hover:bg-white/10 hover:text-white"
          >
            <SettingsIcon className="h-5 w-5" aria-hidden />
          </button>
          <button
            onClick={onSwitchProfile}
            className="flex items-center gap-2 text-sm text-white/50 transition-colors hover:text-white"
          >
            <div
              className="flex h-7 w-7 items-center justify-center rounded-full text-xs font-semibold text-white"
              style={{ backgroundColor: profileView.avatarColor }}
            >
              {profileView.name.slice(0, 2).toUpperCase()}
            </div>
            {profileView.name}
          </button>
        </div>
      </header>

      {sources.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-4 px-6 text-center">
          <DatabaseIcon className="h-10 w-10 text-white/20" aria-hidden />
          <div>
            <p className="text-sm font-medium text-white">No playlists yet</p>
            <p className="mt-1 max-w-sm text-sm text-white/40">
              Import an M3U or Xtream playlist to browse live channels, movies, and series.
            </p>
          </div>
          {sourceError && <ErrorMessage message={sourceError} />}
          <button
            onClick={() => setShowAddSource(true)}
            className="rounded-lg bg-indigo-600 px-5 py-2 text-sm font-semibold text-white transition-colors hover:bg-indigo-500 focus:outline-none focus:ring-2 focus:ring-indigo-400 focus:ring-offset-2 focus:ring-offset-neutral-950"
          >
            Add playlist
          </button>
        </div>
      ) : (
        <main className="flex flex-1 flex-col">
          <section className="flex flex-col gap-3 px-6 pb-3 pt-4 md:flex-row md:items-center md:justify-between">
            <label className="flex min-w-0 flex-col gap-1 text-xs text-white/35">
              Playlist
              <select
                value={activeSourceId ?? ""}
                onChange={(event) => handleSourceChange(event.target.value)}
                className="h-10 rounded-xl border border-white/10 bg-neutral-900 px-3 text-sm font-medium text-white outline-none transition-colors hover:border-white/20 focus:border-indigo-400 focus:ring-2 focus:ring-indigo-400/30"
              >
                {sources.map((source) => {
                  const total = sourceTotals[source.id];
                  return (
                    <option key={source.id} value={source.id}>
                      {source.name}
                      {total !== undefined ? ` (${formatCount(total)})` : ""}
                    </option>
                  );
                })}
              </select>
            </label>
            <div className="flex items-center gap-2">
              <button
                onClick={handleRefresh}
                disabled={activeSourceId === null || refreshing}
                title="Re-fetch this playlist from the provider"
                className="inline-flex h-10 items-center justify-center gap-2 rounded-xl border border-white/10 bg-white/[0.03] px-4 text-sm font-medium text-white/80 transition-colors hover:bg-white/[0.08] hover:text-white focus:outline-none focus:ring-2 focus:ring-indigo-400 focus:ring-offset-2 focus:ring-offset-neutral-950 disabled:cursor-not-allowed disabled:opacity-50"
              >
                <RotateCwIcon className={cn("h-4 w-4", refreshing && "animate-spin")} aria-hidden />
                {refreshing ? "Refreshing…" : "Refresh"}
              </button>
              <button
                onClick={() => setShowAddSource(true)}
                className="inline-flex h-10 items-center justify-center gap-2 rounded-xl border border-indigo-400/30 bg-indigo-500/10 px-4 text-sm font-semibold text-indigo-200 transition-colors hover:bg-indigo-500/20 focus:outline-none focus:ring-2 focus:ring-indigo-400 focus:ring-offset-2 focus:ring-offset-neutral-950"
              >
                <PlusIcon className="h-4 w-4" aria-hidden />
                Add playlist
              </button>
            </div>
          </section>

          <section className="px-6 pb-3">
            <div className="flex gap-2 overflow-x-auto rounded-2xl border border-white/5 bg-white/[0.03] p-1">
              {TABS.map((nextTab) => (
                <button
                  key={nextTab}
                  onClick={() => handleTabChange(nextTab)}
                  className={cn(
                    "flex min-w-fit items-center gap-2 rounded-xl px-4 py-2 text-sm font-medium capitalize transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-400",
                    tab === nextTab
                      ? "bg-white text-black"
                      : "text-white/50 hover:bg-white/5 hover:text-white"
                  )}
                >
                  {nextTab}
                  <span
                    className={cn(
                      "rounded-full px-2 py-0.5 text-xs",
                      tab === nextTab ? "bg-black/10 text-black/70" : "bg-white/5 text-white/35"
                    )}
                  >
                    {formatCount(counts[nextTab])}
                  </span>
                </button>
              ))}
            </div>
          </section>

          <section className="px-6 pb-3">
            <div
              className={cn(
                "grid gap-2",
                tab === "live" ? "md:grid-cols-[minmax(0,1fr)_220px]" : "md:grid-cols-1"
              )}
            >
              <label className="relative">
                <SearchIcon className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-white/30" />
                <input
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                  placeholder={`Search ${tab}`}
                  className="h-10 w-full rounded-xl border border-white/10 bg-neutral-900 pl-9 pr-3 text-sm text-white outline-none transition-colors placeholder:text-white/25 hover:border-white/20 focus:border-indigo-400 focus:ring-2 focus:ring-indigo-400/30"
                />
              </label>

              {tab === "live" && (
                <FilterSelect
                  label="Country"
                  value={countryFilter}
                  options={countryOptions}
                  onChange={setCountryFilter}
                />
              )}
            </div>

            {categoryOptions.length > 0 && (
              <GenreChips options={categoryOptions} value={categoryFilter} onChange={setCategoryFilter} />
            )}

            {searchOrFilterActive && (
              <div className="mt-2 flex flex-wrap items-center gap-1.5 text-xs">
                {searchQuery && (
                  <FilterChip label="Search" value={searchQuery} onClear={() => setSearchQuery("")} />
                )}
                {countryFilter && (
                  <FilterChip label="Country" value={countryFilter} onClear={() => setCountryFilter("")} />
                )}
                {categoryFilter && (
                  <FilterChip label="Category" value={categoryFilter} onClear={() => setCategoryFilter("")} />
                )}
                <button
                  onClick={resetBrowseState}
                  className="rounded-full px-2 py-1 text-indigo-300 transition-colors hover:text-indigo-200 focus:outline-none focus:ring-2 focus:ring-indigo-400"
                >
                  Clear all
                </button>
              </div>
            )}
          </section>

          <section
            ref={scrollRef}
            onScroll={(event) => {
              scrollByTabRef.current[tab] = event.currentTarget.scrollTop;
            }}
            className="relative flex-1 overflow-y-auto px-6 pb-6"
          >
            {sourceError && <ErrorMessage message={sourceError} />}
            {activeSourceId !== null && (
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
                    onPlay={setPlayingMedia}
                    onSelectCategory={setCategoryFilter}
                    onActivate={activate}
                    onToggleFavorite={toggleFavorite}
                    onToggleWatchlist={toggleWatchlist}
                  />
                )}
              </div>
            )}
          </section>
        </main>
      )}

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
          onClose={() => setPlayingMedia(null)}
          sourceId={activeSourceId}
          onPlayMedia={setPlayingMedia}
        />
      )}
    </div>
  );
}

// ── Toggle plumbing ─────────────────────────────────────────────────────────

function invokeFavorite(item: BrowseItem, profileId: string, next: boolean): Promise<void> {
  if (item.kind === "live")
    return invoke("set_channel_favorite", { profileId, channelId: item.id, isFavorite: next });
  if (item.kind === "vod")
    return invoke("set_vod_favorite", { profileId, vodId: item.id, isFavorite: next });
  return invoke("set_series_favorite", {
    profileId,
    sourceId: item.series!.sourceId,
    seriesTitle: item.series!.seriesTitle,
    isFavorite: next,
  });
}

function invokeWatchlist(item: BrowseItem, profileId: string, next: boolean): Promise<void> {
  if (item.kind === "live")
    return invoke("set_channel_in_watchlist", { profileId, channelId: item.id, inWatchlist: next });
  if (item.kind === "vod")
    return invoke("set_vod_in_watchlist", { profileId, vodId: item.id, inWatchlist: next });
  return invoke("set_series_in_watchlist", {
    profileId,
    sourceId: item.series!.sourceId,
    seriesTitle: item.series!.seriesTitle,
    inWatchlist: next,
  });
}

function tabForKind(kind: BrowseItem["kind"]): BrowseTab {
  return kind === "live" ? "live" : kind === "vod" ? "movies" : "series";
}

function seriesItem(s: SeriesSummary): BrowseItem {
  return {
    key: `series-${s.sourceId}-${s.seriesTitle}`,
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
    cur && item.series && cur.seriesTitle === item.series.seriesTitle && cur.sourceId === item.series.sourceId
      ? { ...cur, ...patch }
      : cur;
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

function FilterSelect({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: string[];
  onChange: (value: string) => void;
}) {
  return (
    <label className="relative">
      <ListFilterIcon className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-white/30" />
      <select
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className="h-10 w-full rounded-xl border border-white/10 bg-neutral-900 pl-9 pr-3 text-sm text-white outline-none transition-colors hover:border-white/20 focus:border-indigo-400 focus:ring-2 focus:ring-indigo-400/30"
      >
        <option value="">All {label.toLowerCase()}s</option>
        {options.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
    </label>
  );
}

// Horizontal, scrollable genre chip-bar; tapping a chip filters and opens the grid.
const GenreChips = memo(function GenreChips({
  options,
  value,
  onChange,
}: {
  options: string[];
  value: string;
  onChange: (value: string) => void;
}) {
  const chip = (active: boolean) =>
    cn(
      "shrink-0 rounded-full px-3.5 py-1.5 text-xs font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-400/40",
      active
        ? "bg-indigo-500 text-white"
        : "bg-white/5 text-white/70 hover:bg-white/10 hover:text-white"
    );
  return (
    <div className="mt-2 flex gap-2 overflow-x-auto pb-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
      <button className={chip(value === "")} onClick={() => onChange("")}>
        All
      </button>
      {options.map((option) => {
        const active = value === option;
        return (
          <button
            key={option}
            className={chip(active)}
            onClick={() => onChange(active ? "" : option)}
            aria-pressed={active}
          >
            {option}
          </button>
        );
      })}
    </div>
  );
});

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
    <span className="inline-flex items-center gap-1 rounded-full border border-white/10 bg-white/[0.06] py-1 pl-2.5 pr-1 text-xs text-white/80">
      <span className="text-white/45">{label}:</span>
      <span className="max-w-[10rem] truncate font-medium">{value}</span>
      <button
        type="button"
        onClick={onClear}
        aria-label={`Clear ${label} filter`}
        className="ml-0.5 flex h-4 w-4 items-center justify-center rounded-full text-white/45 transition-colors hover:bg-white/10 hover:text-white focus:outline-none focus:ring-2 focus:ring-indigo-400"
      >
        <XIcon className="h-3 w-3" aria-hidden />
      </button>
    </span>
  );
}

function ErrorMessage({ message }: { message: string }) {
  return (
    <div className="mb-3 flex items-start gap-2 rounded-xl border border-red-400/20 bg-red-500/10 px-3 py-2 text-sm text-red-100">
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
