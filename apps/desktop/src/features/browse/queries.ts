//! Browse data layer. Every read of catalog content goes through these typed
//! query keys + fetchers so TanStack Query can cache, dedupe, prefetch, and
//! background-revalidate. Switching tabs becomes a cache read (instant) instead
//! of an IPC round-trip, and rails/grids share one normalized `BrowseItem`.

import { useQuery, type QueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import type {
  CanonicalMediaSummary,
  Channel,
  ChannelGroup,
  ContentSort,
  ContinueWatchingItem,
  DecadeCount,
  FacetCount,
  LibraryCollection,
  SeriesStatus,
  SeriesSummary,
  VodItem,
} from "@/types";
import { useScreenActive } from "@/lib/screenActive";

export type BrowseTab = "live" | "movies" | "series";
/// `movie` is a canonical entity (P07); `vod` remains for surfaces not yet
/// migrated (semantic search results, which carry a legacy row).
export type BrowseKind = "live" | "movie" | "vod" | "series";

/// One catalog entry, normalized across the three content types so cards,
/// rails, and the grid stay tab-agnostic. The original payload is retained so
/// play / select / favourite actions can resolve back to the typed entity.
export interface BrowseItem {
  key: string;
  kind: BrowseKind;
  id: number;
  title: string;
  subtitle: string | null;
  posterUrl: string | null;
  tvgId: string | null;
  isFavorite: boolean;
  isInWatchlist: boolean;
  /// Enrichment de-dup cluster key — variants collapse to one card per page.
  dedupGroup?: string | null;
  /// Other members of this item's dedup group (quality/backup variants). The
  /// content is never dropped — it rides along on the representative so the
  /// detail screen / player can offer the alternate versions.
  variants?: BrowseItem[];
  /// Live brand-family bundle: the group key + how many channels it holds. When
  /// `memberCount > 1` the card opens the bundle screen instead of playing.
  groupKey?: string;
  memberCount?: number;
  channel?: Channel;
  vod?: VodItem;
  series?: SeriesSummary;
  /// Canonical movie identity + DTO; user actions and playback use `entityId`.
  entityId?: string;
  movie?: CanonicalMediaSummary;
}

/// Superset of channel + content filters. `country` is ignored for movies/series
/// (stripped before the IPC call so the Rust filter structs don't reject it).
export interface BrowseFilters {
  country?: string | null;
  category?: string | null;
  region?: string | null;
  query?: string | null;
  favoritesOnly?: boolean;
  watchlistOnly?: boolean;
  sort?: ContentSort;
  /** Movies and series: Nova's primary genre. */
  genre?: string | null;
  /** Series: still airing, or ended. */
  status?: SeriesStatus | null;
  /** Movies: release years, inclusive. */
  yearFrom?: number | null;
  yearTo?: number | null;
  /** Movies: a TMDB collection. */
  collectionId?: number | null;
}

export const GRID_PAGE_SIZE = 200;
export const RAIL_SIZE = 20;

// ── Normalizers ───────────────────────────────────────────────────────────────

export function channelToItem(c: Channel): BrowseItem {
  return {
    key: `live-${c.id}`,
    kind: "live",
    id: c.id,
    title: c.name,
    subtitle: [c.country, c.category].filter(Boolean).join(" / ") || null,
    posterUrl: c.logoUrl,
    tvgId: c.tvgId,
    isFavorite: c.isFavorite,
    isInWatchlist: c.isInWatchlist,
    dedupGroup: c.dedupGroup,
    channel: c,
  };
}

/// Trailing standalone number stripped for a bundle's display title
/// ("beIN Sports 1" → "beIN Sports") when it groups multiple channels.
function prettyBrand(title: string): string {
  return title.replace(/\s+\d+$/, "").trim() || title;
}

export function channelGroupToItem(g: ChannelGroup): BrowseItem {
  const bundle = g.memberCount > 1;
  return {
    key: `live-group-${g.group}`,
    kind: "live",
    id: g.representativeId,
    title: bundle ? prettyBrand(g.title) : g.title,
    subtitle: [g.region, g.category].filter(Boolean).join(" / ") || null,
    posterUrl: g.logoUrl,
    tvgId: null,
    isFavorite: g.isFavorite,
    isInWatchlist: g.isInWatchlist,
    groupKey: g.group,
    memberCount: g.memberCount,
  };
}

export function vodToItem(v: VodItem): BrowseItem {
  // Prefer the TMDB-enriched display layer, falling back to rules-cleaned / raw.
  const title = v.canonicalTitle ?? v.cleanTitle ?? v.name;
  return {
    key: `vod-${v.id}`,
    kind: "vod",
    id: v.id,
    title,
    subtitle: [v.year?.toString(), v.category].filter(Boolean).join(" / ") || null,
    posterUrl: v.posterUrl ?? v.logoUrl,
    tvgId: null,
    isFavorite: v.isFavorite,
    isInWatchlist: v.isInWatchlist,
    dedupGroup: v.dedupGroup,
    vod: v,
  };
}

export function movieToItem(m: CanonicalMediaSummary): BrowseItem {
  return {
    key: `movie-${m.entityId}`,
    kind: "movie",
    // Canonical movies have no numeric identity; `entityId` addresses them.
    id: 0,
    title: m.title,
    subtitle: [m.year?.toString(), m.category].filter(Boolean).join(" / ") || null,
    posterUrl: m.posterUrl,
    tvgId: null,
    isFavorite: m.isFavorite,
    isInWatchlist: m.isInWatchlist,
    dedupGroup: m.dedupGroup,
    entityId: m.entityId,
    movie: m,
  };
}

/// A series' stable React/cache key: its canonical entity, else (before the
/// catalog compiler has bound it) its source and title.
export function seriesKey(s: SeriesSummary): string {
  return s.entityId ? `series-${s.entityId}` : `series-${s.sourceId}-${s.seriesTitle}`;
}

export function seriesToItem(s: SeriesSummary): BrowseItem {
  return {
    key: seriesKey(s),
    kind: "series",
    id: s.sourceId,
    title: s.displayTitle,
    subtitle: s.category,
    posterUrl: s.posterUrl,
    tvgId: null,
    isFavorite: s.isFavorite,
    isInWatchlist: s.isInWatchlist,
    series: s,
  };
}

// ── Fetchers ────────────────────────────────────────────────────────────────

const LIST_COMMAND: Record<BrowseTab, string> = {
  live: "list_channels",
  movies: "list_movies",
  series: "list_series",
};
const COUNT_COMMAND: Record<BrowseTab, string> = {
  live: "count_channels",
  movies: "count_movies",
  series: "count_series",
};
const CATEGORIES_COMMAND: Record<BrowseTab, string> = {
  live: "list_channel_categories",
  movies: "list_vod_categories",
  series: "list_series_categories",
};

/// Build the per-tab filter payload. Movies/series accept no `country`/`region`.
function filterPayload(tab: BrowseTab, filters: BrowseFilters): BrowseFilters {
  // Each list only receives the filters it understands.
  const { country, region, genre, status, yearFrom, yearTo, collectionId, ...shared } = filters;
  if (tab === "live") return { ...shared, country, region };
  if (tab === "movies") return { ...shared, genre, yearFrom, yearTo, collectionId };
  return { ...shared, genre, status };
}

/// A series filter beyond search is applied (search then stays on the list).
export function seriesFacetApplied(filters: BrowseFilters): boolean {
  return Boolean(
    filters.category ||
    filters.favoritesOnly ||
    filters.watchlistOnly ||
    filters.genre ||
    filters.status
  );
}

/// A movie filter beyond search is applied (search then stays on the list).
export function movieFacetApplied(filters: BrowseFilters): boolean {
  return Boolean(
    filters.category ||
    filters.favoritesOnly ||
    filters.watchlistOnly ||
    filters.genre ||
    filters.yearFrom != null ||
    filters.yearTo != null ||
    filters.collectionId != null
  );
}

async function fetchItems(
  tab: BrowseTab,
  profileId: string,
  sourceId: number,
  filters: BrowseFilters,
  limit: number,
  offset: number,
  recordCount?: (count: number) => void
): Promise<BrowseItem[]> {
  const counted = <T>(rows: T[]): T[] => {
    recordCount?.(rows.length);
    return rows;
  };
  const args = { profileId, sourceId, limit, offset, filters: filterPayload(tab, filters) };
  if (tab === "live") {
    // A plain free-text search runs hybrid (semantic + keyword) retrieval over
    // channels, mapped back to brand-family bundles. Type/region/favourites
    // searches stay on the lexical grouped path.
    const q = filters.query?.trim();
    const plainSearch =
      !!q &&
      !filters.category &&
      !filters.region &&
      !filters.favoritesOnly &&
      !filters.watchlistOnly;
    if (plainSearch) {
      if (offset > 0) return [];
      const hits = await invoke<ChannelGroup[]>("semantic_search_channels", {
        profileId,
        sourceId,
        query: q,
        limit,
      });
      return hits.map(channelGroupToItem);
    }
    // Live is grouped into brand-family bundles server-side (one card per brand).
    return counted(await invoke<ChannelGroup[]>("list_channel_groups", args)).map(
      channelGroupToItem
    );
  }
  if (tab === "movies") {
    // A free-text search with no other restrictive filter goes through the
    // canonical search; category / favourites / watchlist searches stay on the
    // filtered list path.
    const q = filters.query?.trim();
    const plainSearch = !!q && !movieFacetApplied(filters);
    if (plainSearch) {
      // Canonical search: one card per entity, exact titles first, then
      // aliases and metadata (genres, keywords, people); the semantic leg is
      // optional and never outranks a title match. One ranked set, page 0.
      if (offset > 0) return [];
      const hits = await invoke<CanonicalMediaSummary[]>("search_movies", {
        profileId,
        sourceId,
        query: q,
        limit,
      });
      return hits.map(movieToItem);
    }
    // Canonical movies: one card per entity. Which copies are the same film
    // is decided server-side by entity resolution (evidence-based, never on a
    // title alone); a merged entity lists its versions in the detail view. No
    // client-side grouping, which could merge remakes or distinct films.
    return counted(await invoke<CanonicalMediaSummary[]>(LIST_COMMAND.movies, args)).map(
      movieToItem
    );
  }
  // Series: plain free-text search → hybrid (semantic + keyword) retrieval.
  const sq = filters.query?.trim();
  const seriesPlain = !!sq && !seriesFacetApplied(filters);
  if (seriesPlain) {
    if (offset > 0) return [];
    const hits = await invoke<SeriesSummary[]>("semantic_search_series", {
      profileId,
      sourceId,
      query: sq,
      limit,
    });
    return collapseByKey(hits.map(seriesToItem), seriesGroupKey);
  }
  // Series have no backend dedup group; collapse exact-duplicate titles instead.
  return collapseByKey(
    counted(await invoke<SeriesSummary[]>(LIST_COMMAND.series, args)).map(seriesToItem),
    seriesGroupKey
  );
}

/// Collapse a page's duplicates to one representative card each, keeping the
/// other members as `variants` on the representative (lossless — nothing is
/// dropped, so quality/backup versions stay reachable). `keyOf` returns the
/// grouping key, or `null`/`""` for items that should never group. Page-local —
/// exact for rails (one page) and good enough for the grid's pages.
function collapseByKey(
  items: BrowseItem[],
  keyOf: (item: BrowseItem) => string | null
): BrowseItem[] {
  const repAt = new Map<string, number>();
  const out: BrowseItem[] = [];
  for (const item of items) {
    const key = keyOf(item);
    if (!key) {
      out.push(item);
      continue;
    }
    const at = repAt.get(key);
    if (at === undefined) {
      repAt.set(key, out.length);
      out.push({ ...item, variants: [] });
    } else {
      (out[at].variants ??= []).push(item);
    }
  }
  return out;
}

/// Conservative dedup key for series titles: lowercase, keep letters/numbers
/// (Unicode-safe so Arabic/CJK titles aren't blanked), collapse whitespace. Only
/// *identical* normalized titles merge — distinct shows (incl. numbered seasons
/// like `Foo 2`) keep their own card. Empty → never group.
function seriesGroupKey(item: BrowseItem): string | null {
  const raw = item.series?.displayTitle ?? item.title;
  const key = raw
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
  return key || null;
}

async function fetchCount(
  tab: BrowseTab,
  profileId: string,
  sourceId: number,
  filters: BrowseFilters
): Promise<number> {
  return invoke<number>(COUNT_COMMAND[tab], {
    profileId,
    sourceId,
    filters: filterPayload(tab, filters),
  });
}

async function fetchCategories(
  tab: BrowseTab,
  profileId: string,
  sourceId: number
): Promise<string[]> {
  return invoke<string[]>(CATEGORIES_COMMAND[tab], { profileId, sourceId });
}

async function fetchCountries(profileId: string, sourceId: number): Promise<string[]> {
  return invoke<string[]>("list_channel_countries", { profileId, sourceId });
}

async function fetchRegions(profileId: string, sourceId: number): Promise<string[]> {
  return invoke<string[]>("list_channel_regions", { profileId, sourceId });
}

/// All channels of one live brand-family bundle (for the bundle screen / cinema).
export async function fetchChannelsInGroup(
  profileId: string,
  sourceId: number,
  group: string
): Promise<BrowseItem[]> {
  const chans = await invoke<Channel[]>("list_channels_in_group", { profileId, sourceId, group });
  return chans.map(channelToItem);
}

/// Each tab owns its rail; the backend section names match the tab ids.
async function fetchContinueWatching(
  profileId: string,
  sourceId: number,
  tab: BrowseTab
): Promise<ContinueWatchingItem[]> {
  return invoke<ContinueWatchingItem[]>("list_continue_watching", {
    profileId,
    sourceId,
    section: tab,
    limit: RAIL_SIZE,
  });
}

// ── Rail specs ────────────────────────────────────────────────────────────────

export interface RailSpec {
  id: string;
  title: string;
  filters: BrowseFilters;
}

export const RECENTLY_ADDED_RAIL: RailSpec = {
  id: "recentlyAdded",
  title: "Recently Added",
  filters: { sort: "recentlyAdded" },
};
export const WATCHLIST_RAIL: RailSpec = {
  id: "watchlist",
  title: "Your Watchlist",
  filters: { watchlistOnly: true },
};
export const FAVORITES_RAIL: RailSpec = {
  id: "favorites",
  title: "Your Favourites",
  filters: { favoritesOnly: true },
};
/// Personal rails shown at the top of every tab, in display order.
export const PERSONAL_RAILS: RailSpec[] = [WATCHLIST_RAIL, FAVORITES_RAIL, RECENTLY_ADDED_RAIL];

export function genreRail(category: string): RailSpec {
  return { id: `genre:${category}`, title: category, filters: { category } };
}

// ── Query keys ──────────────────────────────────────────────────────────────

export const browseKeys = {
  all: ["browse"] as const,
  source: (profileId: string, sourceId: number) =>
    [...browseKeys.all, profileId, sourceId] as const,
  count: (profileId: string, sourceId: number, tab: BrowseTab, filters: BrowseFilters = {}) =>
    [...browseKeys.source(profileId, sourceId), "count", tab, filters] as const,
  categories: (profileId: string, sourceId: number, tab: BrowseTab) =>
    [...browseKeys.source(profileId, sourceId), "categories", tab] as const,
  countries: (profileId: string, sourceId: number) =>
    [...browseKeys.source(profileId, sourceId), "countries"] as const,
  regions: (profileId: string, sourceId: number) =>
    [...browseKeys.source(profileId, sourceId), "regions"] as const,
  /// Without a source/tab it is the prefix of every tab's rail (invalidation).
  continueWatching: (profileId: string, sourceId?: number, tab?: BrowseTab) =>
    sourceId === undefined
      ? ([...browseKeys.all, profileId, "continueWatching"] as const)
      : ([...browseKeys.all, profileId, "continueWatching", sourceId, tab] as const),
  rail: (profileId: string, sourceId: number, tab: BrowseTab, railId: string) =>
    [...browseKeys.source(profileId, sourceId), "rail", tab, railId] as const,
  grid: (profileId: string, sourceId: number, tab: BrowseTab, filters: BrowseFilters) =>
    [...browseKeys.source(profileId, sourceId), "grid", tab, filters] as const,
};

// ── Hooks ─────────────────────────────────────────────────────────────────────

export function useBrowseCount(
  tab: BrowseTab,
  profileId: string,
  sourceId: number | null,
  filters: BrowseFilters = {}
) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: browseKeys.count(profileId, sourceId ?? -1, tab, filters),
    queryFn: () => fetchCount(tab, profileId, sourceId!, filters),
    enabled: sourceId !== null,
  });
}

export function useCategories(tab: BrowseTab, profileId: string, sourceId: number | null) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: browseKeys.categories(profileId, sourceId ?? -1, tab),
    queryFn: () => fetchCategories(tab, profileId, sourceId!),
    enabled: sourceId !== null,
  });
}

export function useRegions(profileId: string, sourceId: number | null) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: browseKeys.regions(profileId, sourceId ?? -1),
    queryFn: () => fetchRegions(profileId, sourceId!),
    enabled: sourceId !== null,
  });
}

export function useCountries(profileId: string, sourceId: number | null) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: browseKeys.countries(profileId, sourceId ?? -1),
    queryFn: () => fetchCountries(profileId, sourceId!),
    enabled: sourceId !== null,
  });
}

export function useContinueWatching(profileId: string, sourceId: number, tab: BrowseTab) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: browseKeys.continueWatching(profileId, sourceId, tab),
    queryFn: () => fetchContinueWatching(profileId, sourceId, tab),
  });
}

/// One rail's first page. `enabled` is driven by the rail's IntersectionObserver
/// so off-screen rails never fetch until they scroll near the viewport.
export function useRail(
  tab: BrowseTab,
  profileId: string,
  sourceId: number | null,
  spec: RailSpec,
  enabled: boolean
) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: browseKeys.rail(profileId, sourceId ?? -1, tab, spec.id),
    queryFn: () => fetchItems(tab, profileId, sourceId!, spec.filters, RAIL_SIZE, 0),
    enabled: enabled && sourceId !== null,
  });
}

// ── Movie facets (Cinémathèque) ──────────────────────────────────────────────

export const movieFacetKeys = {
  genres: (profileId: string, sourceId: number) =>
    [...browseKeys.source(profileId, sourceId), "movieGenres"] as const,
  decades: (profileId: string, sourceId: number) =>
    [...browseKeys.source(profileId, sourceId), "movieDecades"] as const,
  collections: (profileId: string, sourceId: number) =>
    [...browseKeys.source(profileId, sourceId), "movieCollections"] as const,
};

export function useMovieGenres(profileId: string, sourceId: number | null) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: movieFacetKeys.genres(profileId, sourceId ?? -1),
    queryFn: () => invoke<FacetCount[]>("list_movie_genres", { profileId, sourceId }),
    enabled: sourceId !== null,
  });
}

export function useMovieDecades(profileId: string, sourceId: number | null) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: movieFacetKeys.decades(profileId, sourceId ?? -1),
    queryFn: () => invoke<DecadeCount[]>("list_movie_decades", { profileId, sourceId }),
    enabled: sourceId !== null,
  });
}

export function useLibraryCollections(profileId: string, sourceId: number | null) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: movieFacetKeys.collections(profileId, sourceId ?? -1),
    queryFn: () => invoke<LibraryCollection[]>("list_library_collections", { profileId, sourceId }),
    enabled: sourceId !== null,
  });
}

export const seriesFacetKeys = {
  genres: (profileId: string, sourceId: number) =>
    [...browseKeys.source(profileId, sourceId), "seriesGenres"] as const,
  schedule: (profileId: string, sourceId: number, from: string, to: string) =>
    [...browseKeys.source(profileId, sourceId), "seriesSchedule", from, to] as const,
};

export function useSeriesGenres(profileId: string, sourceId: number | null) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: seriesFacetKeys.genres(profileId, sourceId ?? -1),
    queryFn: () => invoke<FacetCount[]>("list_series_genres", { profileId, sourceId }),
    enabled: sourceId !== null,
  });
}

/// Fresh cards (facts and progress) for series titles of the playlist.
export function useSeriesCards(profileId: string, sourceId: number | null, titles: string[]) {
  const sorted = [...titles].sort();
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: [...browseKeys.source(profileId, sourceId ?? -1), "seriesCards", sorted] as const,
    queryFn: () =>
      invoke<SeriesSummary[]>("list_series_cards", { profileId, sourceId, titles: sorted }),
    enabled: sourceId !== null && sorted.length > 0,
    placeholderData: (previous) => previous,
  });
}

/// Series whose next episode airs between the local dates (inclusive).
export function useSeriesSchedule(
  profileId: string,
  sourceId: number | null,
  from: string,
  to: string
) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: seriesFacetKeys.schedule(profileId, sourceId ?? -1, from, to),
    queryFn: () =>
      invoke<SeriesSummary[]>("list_series_schedule", { profileId, sourceId, from, to }),
    enabled: sourceId !== null,
  });
}

export interface BrowsePage {
  items: BrowseItem[];
  nextOffset: number | null;
}

// Advance by database rows, never by the number of cards left after grouping.
export async function fetchBrowsePage(
  tab: BrowseTab,
  profileId: string,
  sourceId: number,
  filters: BrowseFilters,
  offset: number
): Promise<BrowsePage> {
  let rows = 0;
  const items = await fetchItems(
    tab,
    profileId,
    sourceId,
    filters,
    GRID_PAGE_SIZE,
    offset,
    (count) => {
      rows = count;
    }
  );
  return { items, nextOffset: rows === GRID_PAGE_SIZE ? offset + GRID_PAGE_SIZE : null };
}

/// One bounded page. Inactive pages are released instead of retaining the whole catalog.
export function useBrowseGrid(
  tab: BrowseTab,
  profileId: string,
  sourceId: number | null,
  filters: BrowseFilters,
  offset: number
) {
  return useQuery({
    subscribed: useScreenActive(),
    queryKey: [...browseKeys.grid(profileId, sourceId ?? -1, tab, filters), offset],
    queryFn: () => fetchBrowsePage(tab, profileId, sourceId!, filters, offset),
    enabled: sourceId !== null,
    gcTime: 0,
  });
}

// ── Prefetch ────────────────────────────────────────────────────────────────

/// Warm the cheap per-tab metadata (counts + categories) for every tab plus the
/// profile's Continue Watching, so the tab bar and rail skeleton render with no
/// IPC the first time each tab is opened.
export function prefetchSourceOverview(qc: QueryClient, profileId: string, sourceId: number) {
  for (const tab of ["live", "movies", "series"] as const) {
    void qc.prefetchQuery({
      queryKey: browseKeys.continueWatching(profileId, sourceId, tab),
      queryFn: () => fetchContinueWatching(profileId, sourceId, tab),
    });
    void qc.prefetchQuery({
      queryKey: browseKeys.count(profileId, sourceId, tab, {}),
      queryFn: () => fetchCount(tab, profileId, sourceId, {}),
    });
    void qc.prefetchQuery({
      queryKey: browseKeys.categories(profileId, sourceId, tab),
      queryFn: () => fetchCategories(tab, profileId, sourceId),
    });
  }
}

/// Warm the three personal rails for a tab (the queries the old Shelves refetched
/// on every switch). Called on idle for not-yet-active tabs so switching is instant.
export function prefetchPersonalRails(
  qc: QueryClient,
  profileId: string,
  sourceId: number,
  tab: BrowseTab
) {
  for (const spec of PERSONAL_RAILS) {
    void qc.prefetchQuery({
      queryKey: browseKeys.rail(profileId, sourceId, tab, spec.id),
      queryFn: () => fetchItems(tab, profileId, sourceId, spec.filters, RAIL_SIZE, 0),
    });
  }
}

// ── Invalidation + optimistic patches ─────────────────────────────────────────

/// Invalidate everything for a source (used after refresh / re-import).
export function invalidateSource(qc: QueryClient, profileId: string, sourceId: number) {
  void qc.invalidateQueries({ queryKey: browseKeys.source(profileId, sourceId) });
  // Series detail data is keyed by the series entity, which survives a
  // refresh (and a provider rename) while its episode rows are replaced.
  for (const key of ["series-episodes", "season-episodes", "series-resume"]) {
    void qc.invalidateQueries({ queryKey: [key, profileId] });
  }
}

type ItemPatch = Partial<Pick<BrowseItem, "isFavorite" | "isInWatchlist">>;

/// Optimistically patch a single item across every cached rail + grid page for a
/// source, so its heart/bookmark flips instantly everywhere it's visible.
/// Returns a rollback function (re-applies the inverse patch).
export function patchItemEverywhere(
  qc: QueryClient,
  profileId: string,
  sourceId: number,
  key: string,
  patch: ItemPatch
): () => void {
  const inverse: ItemPatch = {};

  const mapItem = (item: BrowseItem): BrowseItem => {
    if (item.key !== key) return item;
    if (patch.isFavorite !== undefined && inverse.isFavorite === undefined)
      inverse.isFavorite = item.isFavorite;
    if (patch.isInWatchlist !== undefined && inverse.isInWatchlist === undefined)
      inverse.isInWatchlist = item.isInWatchlist;
    return { ...item, ...patch };
  };

  qc.setQueriesData<BrowseItem[]>(
    { queryKey: browseKeys.source(profileId, sourceId), predicate: railPredicate },
    (data) => (data ? data.map(mapItem) : data)
  );
  qc.setQueriesData<BrowsePage>(
    { queryKey: browseKeys.source(profileId, sourceId), predicate: gridPredicate },
    (data) => (data ? { ...data, items: data.items.map(mapItem) } : data)
  );
  // The live guide's channel rows (pages of channels).
  qc.setQueriesData<{ pages: Channel[][]; pageParams: unknown[] }>(
    { queryKey: browseKeys.source(profileId, sourceId), predicate: guideChannelsPredicate },
    (data) =>
      data
        ? {
            ...data,
            pages: data.pages.map((page) =>
              page.map((c) => {
                if (`live-${c.id}` !== key) return c;
                if (patch.isFavorite !== undefined && inverse.isFavorite === undefined)
                  inverse.isFavorite = c.isFavorite;
                if (patch.isInWatchlist !== undefined && inverse.isInWatchlist === undefined)
                  inverse.isInWatchlist = c.isInWatchlist;
                return { ...c, ...patch };
              })
            ),
          }
        : data
  );
  // Show cards read as summaries (Continue cards, the week, series pages).
  qc.setQueriesData<SeriesSummary[]>(
    { queryKey: browseKeys.source(profileId, sourceId), predicate: seriesSummariesPredicate },
    (data) =>
      data
        ? data.map((s) => {
            if (seriesKey(s) !== key) return s;
            if (patch.isFavorite !== undefined && inverse.isFavorite === undefined)
              inverse.isFavorite = s.isFavorite;
            if (patch.isInWatchlist !== undefined && inverse.isInWatchlist === undefined)
              inverse.isInWatchlist = s.isInWatchlist;
            return { ...s, ...patch };
          })
        : data
  );

  return () => {
    if (Object.keys(inverse).length === 0) return;
    patchItemEverywhere(qc, profileId, sourceId, key, inverse);
  };
}

function railPredicate(query: { queryKey: readonly unknown[] }): boolean {
  return query.queryKey.includes("rail");
}
function gridPredicate(query: { queryKey: readonly unknown[] }): boolean {
  return query.queryKey.includes("grid");
}
function guideChannelsPredicate(query: { queryKey: readonly unknown[] }): boolean {
  return query.queryKey.includes("guideChannels");
}
function seriesSummariesPredicate(query: { queryKey: readonly unknown[] }): boolean {
  return query.queryKey.includes("seriesCards") || query.queryKey.includes("seriesSchedule");
}

/// After a favourite/watchlist toggle settles, refresh the membership rails for a
/// tab (their contents add/remove an item) without disturbing the rest.
export function invalidateMembershipRails(
  qc: QueryClient,
  profileId: string,
  sourceId: number,
  tab: BrowseTab
) {
  for (const spec of [FAVORITES_RAIL, WATCHLIST_RAIL]) {
    void qc.invalidateQueries({ queryKey: browseKeys.rail(profileId, sourceId, tab, spec.id) });
  }
  // Library's saved views need new membership and totals after a toggle.
  void qc.invalidateQueries({
    queryKey: browseKeys.source(profileId, sourceId),
    predicate: (q) => {
      if (q.queryKey[4] !== tab || !["grid", "count"].includes(String(q.queryKey[3]))) return false;
      const filters = q.queryKey[5] as BrowseFilters | undefined;
      return Boolean(filters?.favoritesOnly || filters?.watchlistOnly);
    },
  });
}
