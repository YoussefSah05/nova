import { useDeferredValue, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertCircleIcon,
  ChevronRightIcon,
  DatabaseIcon,
  ListFilterIcon,
  PlusIcon,
  SearchIcon,
  XIcon,
} from "lucide-react";
import type { Channel, Profile, SeriesSummary, Source, VodItem } from "@/types";
import { AddSourceModal } from "@/features/source/AddSourceModal";
import { SeriesDetail } from "@/features/browse/SeriesDetail";
import { Shelves } from "@/features/browse/Shelves";
import { FavoriteButton } from "@/features/browse/FavoriteButton";
import { WatchlistButton } from "@/features/browse/WatchlistButton";
import { NowAndNextChip } from "@/features/browse/NowAndNextChip";
import { useNowAndNext } from "@/features/browse/useNowAndNext";
import { PlayerModal, type PlayMedia } from "@/features/player/PlayerModal";
import { cn } from "@/lib/utils";

type Tab = "live" | "movies" | "series";

const SERIES_PAGE_LIMIT = 10_000;

interface HomeProps {
  profile: Profile;
  onSwitchProfile: () => void;
}

interface ContentCounts {
  live: number;
  movies: number;
  series: number;
}

interface FilterOptions {
  channelCountries: string[];
  channelCategories: string[];
  vodCategories: string[];
  seriesCategories: string[];
}

const PAGE_SIZE = 200;
const TABS: Tab[] = ["live", "movies", "series"];
const EMPTY_COUNTS: ContentCounts = { live: 0, movies: 0, series: 0 };
const EMPTY_FILTER_OPTIONS: FilterOptions = {
  channelCountries: [],
  channelCategories: [],
  vodCategories: [],
  seriesCategories: [],
};

export function Home({ profile, onSwitchProfile }: HomeProps) {
  const [sources, setSources] = useState<Source[]>([]);
  const [sourceTotals, setSourceTotals] = useState<Record<number, number>>({});
  const [activeSourceId, setActiveSourceId] = useState<number | null>(null);
  const [tab, setTab] = useState<Tab>("live");
  const [counts, setCounts] = useState<ContentCounts>(EMPTY_COUNTS);
  const [filterOptions, setFilterOptions] = useState<FilterOptions>(EMPTY_FILTER_OPTIONS);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [vod, setVod] = useState<VodItem[]>([]);
  const [series, setSeries] = useState<SeriesSummary[]>([]);
  const [selectedSeries, setSelectedSeries] = useState<SeriesSummary | null>(null);
  const [playingMedia, setPlayingMedia] = useState<PlayMedia | null>(null);
  const [showAddSource, setShowAddSource] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [countryFilter, setCountryFilter] = useState("");
  const [categoryFilter, setCategoryFilter] = useState("");
  const [activeTotal, setActiveTotal] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [sourceError, setSourceError] = useState<string | null>(null);
  const [contentError, setContentError] = useState<string | null>(null);
  const [shelvesKey, setShelvesKey] = useState(0);
  const deferredSearchQuery = useDeferredValue(searchQuery);

  const activeSource = sources.find((source) => source.id === activeSourceId) ?? null;
  const visibleCount =
    tab === "live" ? channels.length : tab === "movies" ? vod.length : series.length;
  const hasMore = tab !== "series" && visibleCount < activeTotal;
  const categoryOptions =
    tab === "live"
      ? filterOptions.channelCategories
      : tab === "movies"
        ? filterOptions.vodCategories
        : filterOptions.seriesCategories;

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
        if (!cancelled) {
          setSourceError(errorMessage(error));
        }
      }
    }

    void loadSources();
    return () => {
      cancelled = true;
    };
  }, [profile.id]);

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
            invoke<number>("count_vod", {
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

      if (!cancelled) {
        setSourceTotals(Object.fromEntries(entries));
      }
    }

    void loadSourceTotals().catch(() => {
      /* counts are decoration; ignore errors */
    });
    return () => {
      cancelled = true;
    };
  }, [profile.id, sources]);

  useEffect(() => {
    if (activeSourceId === null) {
      setCounts(EMPTY_COUNTS);
      return;
    }

    let cancelled = false;

    async function loadCounts() {
      try {
        const [live, movies, nextSeries] = await Promise.all([
          invoke<number>("count_channels", {
            profileId: profile.id,
            sourceId: activeSourceId,
            filters: null,
          }),
          invoke<number>("count_vod", {
            profileId: profile.id,
            sourceId: activeSourceId,
            filters: null,
          }),
          invoke<number>("count_series", {
            profileId: profile.id,
            sourceId: activeSourceId,
            filters: null,
          }),
        ]);

        if (!cancelled) {
          setCounts({ live, movies, series: nextSeries });
        }
      } catch (error: unknown) {
        if (!cancelled) {
          setContentError(errorMessage(error));
        }
      }
    }

    void loadCounts();
    return () => {
      cancelled = true;
    };
  }, [profile.id, activeSourceId]);

  useEffect(() => {
    if (activeSourceId === null) {
      setFilterOptions(EMPTY_FILTER_OPTIONS);
      return;
    }

    let cancelled = false;

    async function loadFilterOptions() {
      try {
        const [channelCountries, channelCategories, vodCategories, seriesCategories] =
          await Promise.all([
            invoke<string[]>("list_channel_countries", {
              profileId: profile.id,
              sourceId: activeSourceId,
            }),
            invoke<string[]>("list_channel_categories", {
              profileId: profile.id,
              sourceId: activeSourceId,
            }),
            invoke<string[]>("list_vod_categories", {
              profileId: profile.id,
              sourceId: activeSourceId,
            }),
            invoke<string[]>("list_series_categories", {
              profileId: profile.id,
              sourceId: activeSourceId,
            }),
          ]);

        if (!cancelled) {
          setFilterOptions({
            channelCountries,
            channelCategories,
            vodCategories,
            seriesCategories,
          });
        }
      } catch (error: unknown) {
        if (!cancelled) {
          setContentError(errorMessage(error));
        }
      }
    }

    void loadFilterOptions();
    return () => {
      cancelled = true;
    };
  }, [profile.id, activeSourceId]);

  useEffect(() => {
    if (activeSourceId === null) {
      clearContent();
      return;
    }

    let cancelled = false;

    async function loadContent() {
      setIsLoading(true);
      setContentError(null);

      try {
        const query = filterParam(deferredSearchQuery);
        const category = filterParam(categoryFilter);

        if (tab === "live") {
          const [nextChannels, total] = await Promise.all([
            invoke<Channel[]>("list_channels", {
              profileId: profile.id,
              sourceId: activeSourceId,
              limit: PAGE_SIZE,
              offset: 0,
              filters: {
                country: filterParam(countryFilter),
                category,
                query,
              },
            }),
            invoke<number>("count_channels", {
              profileId: profile.id,
              sourceId: activeSourceId,
              filters: {
                country: filterParam(countryFilter),
                category,
                query,
              },
            }),
          ]);

          if (!cancelled) {
            setChannels(nextChannels);
            setActiveTotal(total);
          }
        } else if (tab === "movies") {
          const [nextVod, total] = await Promise.all([
            invoke<VodItem[]>("list_vod", {
              profileId: profile.id,
              sourceId: activeSourceId,
              limit: PAGE_SIZE,
              offset: 0,
              filters: {
                category,
                query,
              },
            }),
            invoke<number>("count_vod", {
              profileId: profile.id,
              sourceId: activeSourceId,
              filters: {
                category,
                query,
              },
            }),
          ]);

          if (!cancelled) {
            setVod(nextVod);
            setActiveTotal(total);
          }
        } else {
          const [nextSeries, total] = await Promise.all([
            invoke<SeriesSummary[]>("list_series", {
              profileId: profile.id,
              sourceId: activeSourceId,
              limit: SERIES_PAGE_LIMIT,
              offset: 0,
              filters: {
                category,
                query,
              },
            }),
            invoke<number>("count_series", {
              profileId: profile.id,
              sourceId: activeSourceId,
              filters: {
                category,
                query,
              },
            }),
          ]);

          if (!cancelled) {
            setSeries(nextSeries);
            setActiveTotal(total);
          }
        }
      } catch (error: unknown) {
        if (!cancelled) {
          clearContentForTab(tab);
          setActiveTotal(0);
          setContentError(errorMessage(error));
        }
      } finally {
        if (!cancelled) {
          setIsLoading(false);
        }
      }
    }

    void loadContent();
    return () => {
      cancelled = true;
    };
  }, [profile.id, activeSourceId, tab, deferredSearchQuery, countryFilter, categoryFilter]);

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

  function handleTabChange(nextTab: Tab) {
    if (nextTab === tab) return;
    setTab(nextTab);
    resetBrowseState();
  }

  function handleSourceChange(value: string) {
    setActiveSourceId(Number(value));
    resetBrowseState();
  }

  async function handleLoadMore() {
    if (activeSourceId === null || tab === "series" || isLoading || isLoadingMore || !hasMore) {
      return;
    }

    setIsLoadingMore(true);
    setContentError(null);

    try {
      const query = filterParam(deferredSearchQuery);
      const category = filterParam(categoryFilter);

      if (tab === "live") {
        const nextChannels = await invoke<Channel[]>("list_channels", {
          profileId: profile.id,
          sourceId: activeSourceId,
          limit: PAGE_SIZE,
          offset: channels.length,
          filters: {
            country: filterParam(countryFilter),
            category,
            query,
          },
        });
        setChannels((current) => [...current, ...nextChannels]);
      } else {
        const nextVod = await invoke<VodItem[]>("list_vod", {
          profileId: profile.id,
          sourceId: activeSourceId,
          limit: PAGE_SIZE,
          offset: vod.length,
          filters: {
            category,
            query,
          },
        });
        setVod((current) => [...current, ...nextVod]);
      }
    } catch (error: unknown) {
      setContentError(errorMessage(error));
    } finally {
      setIsLoadingMore(false);
    }
  }

  function resetBrowseState() {
    setSearchQuery("");
    setCountryFilter("");
    setCategoryFilter("");
    setSelectedSeries(null);
    setContentError(null);
  }

  async function handleToggleChannelFavorite(channel: Channel, next: boolean) {
    setChannels((current) =>
      current.map((c) => (c.id === channel.id ? { ...c, isFavorite: next } : c))
    );
    try {
      await invoke<void>("set_channel_favorite", {
        profileId: profile.id,
        channelId: channel.id,
        isFavorite: next,
      });
      setShelvesKey((k) => k + 1);
    } catch (error: unknown) {
      setChannels((current) =>
        current.map((c) => (c.id === channel.id ? { ...c, isFavorite: !next } : c))
      );
      setContentError(errorMessage(error));
    }
  }

  async function handleToggleVodFavorite(item: VodItem, next: boolean) {
    setVod((current) => current.map((v) => (v.id === item.id ? { ...v, isFavorite: next } : v)));
    try {
      await invoke<void>("set_vod_favorite", {
        profileId: profile.id,
        vodId: item.id,
        isFavorite: next,
      });
      setShelvesKey((k) => k + 1);
    } catch (error: unknown) {
      setVod((current) => current.map((v) => (v.id === item.id ? { ...v, isFavorite: !next } : v)));
      setContentError(errorMessage(error));
    }
  }

  async function handleToggleSeriesFavorite(item: SeriesSummary, next: boolean) {
    setSeries((current) =>
      current.map((s) =>
        s.seriesTitle === item.seriesTitle && s.sourceId === item.sourceId
          ? { ...s, isFavorite: next }
          : s
      )
    );
    if (
      selectedSeries?.seriesTitle === item.seriesTitle &&
      selectedSeries.sourceId === item.sourceId
    ) {
      setSelectedSeries({ ...selectedSeries, isFavorite: next });
    }
    try {
      await invoke<void>("set_series_favorite", {
        profileId: profile.id,
        sourceId: item.sourceId,
        seriesTitle: item.seriesTitle,
        isFavorite: next,
      });
      setShelvesKey((k) => k + 1);
    } catch (error: unknown) {
      setSeries((current) =>
        current.map((s) =>
          s.seriesTitle === item.seriesTitle && s.sourceId === item.sourceId
            ? { ...s, isFavorite: !next }
            : s
        )
      );
      if (
        selectedSeries?.seriesTitle === item.seriesTitle &&
        selectedSeries.sourceId === item.sourceId
      ) {
        setSelectedSeries({ ...selectedSeries, isFavorite: !next });
      }
      setContentError(errorMessage(error));
    }
  }

  async function handleToggleChannelWatchlist(channel: Channel, next: boolean) {
    setChannels((current) =>
      current.map((c) => (c.id === channel.id ? { ...c, isInWatchlist: next } : c))
    );
    try {
      await invoke<void>("set_channel_in_watchlist", {
        profileId: profile.id,
        channelId: channel.id,
        inWatchlist: next,
      });
      setShelvesKey((k) => k + 1);
    } catch (error: unknown) {
      setChannels((current) =>
        current.map((c) => (c.id === channel.id ? { ...c, isInWatchlist: !next } : c))
      );
      setContentError(errorMessage(error));
    }
  }

  async function handleToggleVodWatchlist(item: VodItem, next: boolean) {
    setVod((current) => current.map((v) => (v.id === item.id ? { ...v, isInWatchlist: next } : v)));
    try {
      await invoke<void>("set_vod_in_watchlist", {
        profileId: profile.id,
        vodId: item.id,
        inWatchlist: next,
      });
      setShelvesKey((k) => k + 1);
    } catch (error: unknown) {
      setVod((current) =>
        current.map((v) => (v.id === item.id ? { ...v, isInWatchlist: !next } : v))
      );
      setContentError(errorMessage(error));
    }
  }

  async function handleToggleSeriesWatchlist(item: SeriesSummary, next: boolean) {
    setSeries((current) =>
      current.map((s) =>
        s.seriesTitle === item.seriesTitle && s.sourceId === item.sourceId
          ? { ...s, isInWatchlist: next }
          : s
      )
    );
    if (
      selectedSeries?.seriesTitle === item.seriesTitle &&
      selectedSeries.sourceId === item.sourceId
    ) {
      setSelectedSeries({ ...selectedSeries, isInWatchlist: next });
    }
    try {
      await invoke<void>("set_series_in_watchlist", {
        profileId: profile.id,
        sourceId: item.sourceId,
        seriesTitle: item.seriesTitle,
        inWatchlist: next,
      });
      setShelvesKey((k) => k + 1);
    } catch (error: unknown) {
      setSeries((current) =>
        current.map((s) =>
          s.seriesTitle === item.seriesTitle && s.sourceId === item.sourceId
            ? { ...s, isInWatchlist: !next }
            : s
        )
      );
      if (
        selectedSeries?.seriesTitle === item.seriesTitle &&
        selectedSeries.sourceId === item.sourceId
      ) {
        setSelectedSeries({ ...selectedSeries, isInWatchlist: !next });
      }
      setContentError(errorMessage(error));
    }
  }

  function clearContent() {
    setChannels([]);
    setVod([]);
    setSeries([]);
    setActiveTotal(0);
  }

  function clearContentForTab(nextTab: Tab) {
    if (nextTab === "live") {
      setChannels([]);
    } else if (nextTab === "movies") {
      setVod([]);
    } else {
      setSeries([]);
    }
  }

  if (selectedSeries) {
    return (
      <>
        <div className="min-h-screen bg-neutral-950 p-6">
          <SeriesDetail
            profileId={profile.id}
            series={selectedSeries}
            onBack={() => setSelectedSeries(null)}
            onToggleFavorite={(next) => handleToggleSeriesFavorite(selectedSeries, next)}
            onToggleWatchlist={(next) => handleToggleSeriesWatchlist(selectedSeries, next)}
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
      <header className="flex items-center justify-between border-b border-white/5 px-6 py-4">
        <div>
          <span className="text-lg font-semibold text-white">Nova</span>
          {activeSource && (
            <p className="mt-0.5 text-xs text-white/35">
              Browsing {formatCount(counts.live + counts.movies + counts.series)} imported items
            </p>
          )}
        </div>
        <button
          onClick={onSwitchProfile}
          className="flex items-center gap-2 text-sm text-white/50 transition-colors hover:text-white"
        >
          <div
            className="flex h-7 w-7 items-center justify-center rounded-full text-xs font-semibold text-white"
            style={{ backgroundColor: profile.avatarColor }}
          >
            {profile.name.slice(0, 2).toUpperCase()}
          </div>
          {profile.name}
        </button>
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
            <button
              onClick={() => setShowAddSource(true)}
              className="inline-flex h-10 items-center justify-center gap-2 rounded-xl border border-indigo-400/30 bg-indigo-500/10 px-4 text-sm font-semibold text-indigo-200 transition-colors hover:bg-indigo-500/20 focus:outline-none focus:ring-2 focus:ring-indigo-400 focus:ring-offset-2 focus:ring-offset-neutral-950"
            >
              <PlusIcon className="h-4 w-4" aria-hidden />
              Add playlist
            </button>
          </section>

          {activeSourceId !== null && (
            <Shelves
              profileId={profile.id}
              sourceId={activeSourceId}
              tab={tab}
              refreshKey={shelvesKey}
              onSelectSeries={setSelectedSeries}
              onPlay={setPlayingMedia}
            />
          )}

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
                  options={filterOptions.channelCountries}
                  onChange={setCountryFilter}
                />
              )}
            </div>

            {categoryOptions.length > 0 && (
              <GenreChips
                options={categoryOptions}
                value={categoryFilter}
                onChange={setCategoryFilter}
              />
            )}
          </section>

          <section className="flex-1 overflow-y-auto px-6 pb-6">
            <div className="mb-3 flex flex-wrap items-center justify-between gap-2 text-xs text-white/35">
              <span>
                Showing {formatCount(visibleCount)} of {formatCount(activeTotal)} {tab}
              </span>
              {(countryFilter || categoryFilter || searchQuery) && (
                <div className="flex flex-wrap items-center gap-1.5">
                  {searchQuery && (
                    <FilterChip
                      label="Search"
                      value={searchQuery}
                      onClear={() => setSearchQuery("")}
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
                  <button
                    onClick={resetBrowseState}
                    className="rounded-full px-2 py-1 text-indigo-300 transition-colors hover:text-indigo-200 focus:outline-none focus:ring-2 focus:ring-indigo-400"
                  >
                    Clear all
                  </button>
                </div>
              )}
            </div>

            {contentError && <ErrorMessage message={contentError} />}

            {isLoading ? (
              <SkeletonRows variant={tab} />
            ) : (
              <>
                {tab === "live" && (
                  <ChannelList
                    profileId={profile.id}
                    channels={channels}
                    onToggleFavorite={handleToggleChannelFavorite}
                    onToggleWatchlist={handleToggleChannelWatchlist}
                    onPlay={(channel) =>
                      setPlayingMedia({
                        kind: "live",
                        id: channel.id,
                        title: channel.name,
                        subtitle:
                          [channel.country, channel.category].filter(Boolean).join(" / ") || null,
                        posterUrl: channel.logoUrl,
                      })
                    }
                  />
                )}
                {tab === "movies" && (
                  <VodList
                    items={vod}
                    onToggleFavorite={handleToggleVodFavorite}
                    onToggleWatchlist={handleToggleVodWatchlist}
                    onPlay={(item) =>
                      setPlayingMedia({
                        kind: "vod",
                        id: item.id,
                        title: item.cleanTitle ?? item.name,
                        subtitle:
                          [item.year?.toString(), item.category].filter(Boolean).join(" / ") ||
                          null,
                        posterUrl: item.logoUrl,
                      })
                    }
                  />
                )}
                {tab === "series" && (
                  <SeriesList
                    items={series}
                    onSelect={(item) => setSelectedSeries(item)}
                    onToggleFavorite={handleToggleSeriesFavorite}
                    onToggleWatchlist={handleToggleSeriesWatchlist}
                  />
                )}

                {!contentError && visibleCount === 0 && (
                  <EmptyContent
                    tab={tab}
                    hasFilters={Boolean(countryFilter || categoryFilter || searchQuery)}
                  />
                )}

                {hasMore && (
                  <div className="flex justify-center pt-5">
                    <button
                      onClick={handleLoadMore}
                      disabled={isLoadingMore}
                      className="rounded-xl border border-white/10 bg-white/[0.03] px-5 py-2 text-sm font-semibold text-white/75 transition-colors hover:bg-white/[0.07] hover:text-white disabled:cursor-not-allowed disabled:opacity-50 focus:outline-none focus:ring-2 focus:ring-indigo-400 focus:ring-offset-2 focus:ring-offset-neutral-950"
                    >
                      {isLoadingMore
                        ? "Loading more..."
                        : `Load ${Math.min(PAGE_SIZE, activeTotal - visibleCount)} more`}
                    </button>
                  </div>
                )}
              </>
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

function ChannelList({
  profileId,
  channels,
  onToggleFavorite,
  onToggleWatchlist,
  onPlay,
}: {
  profileId: string;
  channels: Channel[];
  onToggleFavorite: (channel: Channel, next: boolean) => Promise<void> | void;
  onToggleWatchlist: (channel: Channel, next: boolean) => Promise<void> | void;
  onPlay: (channel: Channel) => void;
}) {
  const nowMap = useNowAndNext(profileId, channels);
  if (channels.length === 0) return null;

  return (
    <div className="flex flex-col divide-y divide-white/5 rounded-2xl border border-white/5 bg-white/[0.02]">
      {channels.map((channel) => (
        <div key={channel.id} className="flex items-center gap-3 px-3 py-2.5">
          <button
            onClick={() => onPlay(channel)}
            className="flex min-w-0 flex-1 items-center gap-3 text-left transition-opacity hover:opacity-80 focus:outline-none focus:ring-2 focus:ring-indigo-400/50 rounded-lg"
            aria-label={`Play ${channel.name}`}
          >
            <MediaThumb src={channel.logoUrl} className="h-9 w-9 rounded-lg object-contain" />
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm text-white">{channel.name}</p>
              <p className="truncate text-xs text-white/30">
                {[channel.country, channel.category].filter(Boolean).join(" / ") || "Live channel"}
              </p>
              {channel.tvgId && <NowAndNextChip data={nowMap[channel.tvgId]} />}
            </div>
          </button>
          <WatchlistButton
            inWatchlist={channel.isInWatchlist}
            onToggle={(next) => onToggleWatchlist(channel, next)}
            label={channel.name}
            size="sm"
          />
          <FavoriteButton
            isFavorite={channel.isFavorite}
            onToggle={(next) => onToggleFavorite(channel, next)}
            label={channel.name}
            size="sm"
          />
        </div>
      ))}
    </div>
  );
}

function VodList({
  items,
  onToggleFavorite,
  onToggleWatchlist,
  onPlay,
}: {
  items: VodItem[];
  onToggleFavorite: (item: VodItem, next: boolean) => Promise<void> | void;
  onToggleWatchlist: (item: VodItem, next: boolean) => Promise<void> | void;
  onPlay: (item: VodItem) => void;
}) {
  if (items.length === 0) return null;

  return (
    <div className="flex flex-col divide-y divide-white/5 rounded-2xl border border-white/5 bg-white/[0.02]">
      {items.map((item) => (
        <div key={item.id} className="flex items-center gap-3 px-3 py-2.5">
          <button
            onClick={() => onPlay(item)}
            className="flex min-w-0 flex-1 items-center gap-3 text-left transition-opacity hover:opacity-80 focus:outline-none focus:ring-2 focus:ring-indigo-400/50 rounded-lg"
            aria-label={`Play ${item.cleanTitle ?? item.name}`}
          >
            <MediaThumb src={item.logoUrl} className="h-14 w-10 rounded-lg object-cover" />
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm text-white">{item.cleanTitle ?? item.name}</p>
              <p className="truncate text-xs text-white/30">
                {[item.year?.toString(), item.category].filter(Boolean).join(" / ") || "Movie"}
              </p>
            </div>
          </button>
          <WatchlistButton
            inWatchlist={item.isInWatchlist}
            onToggle={(next) => onToggleWatchlist(item, next)}
            label={item.cleanTitle ?? item.name}
            size="sm"
          />
          <FavoriteButton
            isFavorite={item.isFavorite}
            onToggle={(next) => onToggleFavorite(item, next)}
            label={item.cleanTitle ?? item.name}
            size="sm"
          />
        </div>
      ))}
    </div>
  );
}

function SeriesList({
  items,
  onSelect,
  onToggleFavorite,
  onToggleWatchlist,
}: {
  items: SeriesSummary[];
  onSelect: (item: SeriesSummary) => void;
  onToggleFavorite: (item: SeriesSummary, next: boolean) => Promise<void> | void;
  onToggleWatchlist: (item: SeriesSummary, next: boolean) => Promise<void> | void;
}) {
  if (items.length === 0) return null;

  return (
    <div className="flex flex-col divide-y divide-white/5 rounded-2xl border border-white/5 bg-white/[0.02]">
      {items.map((item) => (
        <div
          key={`${item.sourceId}-${item.seriesTitle}`}
          className="flex w-full items-center gap-3 px-3 py-2.5 transition-colors hover:bg-white/5 focus-within:bg-white/[0.04]"
        >
          <button
            type="button"
            onClick={() => onSelect(item)}
            className="flex min-w-0 flex-1 items-center gap-3 text-left focus:outline-none focus:ring-2 focus:ring-inset focus:ring-indigo-400"
          >
            <MediaThumb src={item.posterUrl} className="h-14 w-10 rounded-lg object-cover" />
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm text-white">{item.seriesTitle}</p>
              <p className="truncate text-xs text-white/30">
                {item.category ? `${item.category} / ` : ""}
                {item.seasonCount} season{item.seasonCount !== 1 ? "s" : ""} / {item.episodeCount}{" "}
                ep
              </p>
            </div>
            <ChevronRightIcon className="h-4 w-4 flex-shrink-0 text-white/20" aria-hidden />
          </button>
          <WatchlistButton
            inWatchlist={item.isInWatchlist}
            onToggle={(next) => onToggleWatchlist(item, next)}
            label={item.seriesTitle}
            size="sm"
          />
          <FavoriteButton
            isFavorite={item.isFavorite}
            onToggle={(next) => onToggleFavorite(item, next)}
            label={item.seriesTitle}
            size="sm"
          />
        </div>
      ))}
    </div>
  );
}

function MediaThumb({ src, className }: { src: string | null; className: string }) {
  if (!src) {
    return <div className={cn("flex-shrink-0 bg-neutral-800", className)} />;
  }

  return (
    <img
      src={src}
      alt=""
      loading="lazy"
      className={cn("flex-shrink-0 bg-neutral-800", className)}
    />
  );
}

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

// Horizontal, scrollable genre chip-bar. Categories are canonical genres after
// import normalization (nova_core::normalize_category), so each option is a
// one-tap filter. Tapping the active chip (or "All") clears the filter.
function GenreChips({
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
}

function SkeletonRows({ variant }: { variant: Tab }) {
  const thumbClass = variant === "live" ? "h-9 w-9" : "h-14 w-10";

  return (
    <div className="flex flex-col divide-y divide-white/5 rounded-2xl border border-white/5 bg-white/[0.02]">
      {Array.from({ length: 8 }, (_, index) => (
        <div key={index} className="flex items-center gap-3 px-3 py-2.5">
          <div className={cn("animate-pulse rounded-lg bg-white/10", thumbClass)} />
          <div className="min-w-0 flex-1">
            <div className="h-3 w-2/5 animate-pulse rounded bg-white/10" />
            <div className="mt-2 h-2 w-1/4 animate-pulse rounded bg-white/5" />
          </div>
        </div>
      ))}
    </div>
  );
}

function EmptyContent({ tab, hasFilters }: { tab: Tab; hasFilters: boolean }) {
  const label = tab === "live" ? "channels" : tab;

  return (
    <div className="flex flex-col items-center justify-center rounded-2xl border border-white/5 bg-white/[0.02] px-6 py-14 text-center">
      <DatabaseIcon className="h-9 w-9 text-white/20" aria-hidden />
      <p className="mt-3 text-sm font-medium text-white">
        {hasFilters ? `No ${label} match this view` : `No ${label} imported`}
      </p>
      <p className="mt-1 max-w-sm text-sm text-white/35">
        {hasFilters
          ? "Clear filters or try a broader search."
          : "Import another playlist or check the selected source."}
      </p>
    </div>
  );
}

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
  if (typeof error === "string") {
    return error;
  }

  if (typeof error === "object" && error !== null) {
    const maybeMessage = "message" in error ? error.message : null;
    if (typeof maybeMessage === "string") {
      return maybeMessage;
    }
  }

  return "Something went wrong while loading imported content.";
}
