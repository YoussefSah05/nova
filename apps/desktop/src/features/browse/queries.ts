//! Browse data layer. Every read of catalog content goes through these typed
//! query keys + fetchers so TanStack Query can cache, dedupe, prefetch, and
//! background-revalidate. Switching tabs becomes a cache read (instant) instead
//! of an IPC round-trip, and rails/grids share one normalized `BrowseItem`.

import {
  keepPreviousData,
  useInfiniteQuery,
  useQuery,
  type QueryClient,
} from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import type {
  Channel,
  ChannelGroup,
  ContentSort,
  ContinueWatchingItem,
  SeriesSummary,
  VodItem,
} from "@/types";

export type BrowseTab = "live" | "movies" | "series";
export type BrowseKind = "live" | "vod" | "series";

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

function channelGroupToItem(g: ChannelGroup): BrowseItem {
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

function vodToItem(v: VodItem): BrowseItem {
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

function seriesToItem(s: SeriesSummary): BrowseItem {
  return {
    key: `series-${s.sourceId}-${s.seriesTitle}`,
    kind: "series",
    id: s.sourceId,
    title: s.canonicalTitle ?? s.seriesTitle,
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
  movies: "list_vod",
  series: "list_series",
};
const COUNT_COMMAND: Record<BrowseTab, string> = {
  live: "count_channels",
  movies: "count_vod",
  series: "count_series",
};
const CATEGORIES_COMMAND: Record<BrowseTab, string> = {
  live: "list_channel_categories",
  movies: "list_vod_categories",
  series: "list_series_categories",
};

/// Build the per-tab filter payload. Movies/series accept no `country`/`region`.
function filterPayload(tab: BrowseTab, filters: BrowseFilters): BrowseFilters {
  if (tab === "live") return filters;
  const { country: _country, region: _region, ...rest } = filters;
  return rest;
}

async function fetchItems(
  tab: BrowseTab,
  profileId: string,
  sourceId: number,
  filters: BrowseFilters,
  limit: number,
  offset: number
): Promise<BrowseItem[]> {
  const args = { profileId, sourceId, limit, offset, filters: filterPayload(tab, filters) };
  if (tab === "live") {
    // Live is grouped into brand-family bundles server-side (one card per brand).
    return (await invoke<ChannelGroup[]>("list_channel_groups", args)).map(channelGroupToItem);
  }
  if (tab === "movies") {
    // Gather the same film's language/source copies under one card (title-based,
    // from the raw name); fall back to the embeddings group when no real title.
    return collapseByKey(
      (await invoke<VodItem[]>(LIST_COMMAND.movies, args)).map(vodToItem),
      (item) => movieVersionKey(item) ?? item.dedupGroup ?? null
    );
  }
  // Series have no backend dedup group; collapse exact-duplicate titles instead.
  return collapseByKey(
    (await invoke<SeriesSummary[]>(LIST_COMMAND.series, args)).map(seriesToItem),
    seriesGroupKey
  );
}

/// Collapse a page's duplicates to one representative card each, keeping the
/// other members as `variants` on the representative (lossless — nothing is
/// dropped, so quality/backup versions stay reachable). `keyOf` returns the
/// grouping key, or `null`/`""` for items that should never group. Page-local —
/// exact for rails (one page) and good enough for the grid's pages.
function collapseByKey(items: BrowseItem[], keyOf: (item: BrowseItem) => string | null): BrowseItem[] {
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
  const raw = item.series?.canonicalTitle ?? item.series?.seriesTitle ?? item.title;
  const key = raw
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
  return key || null;
}

/// Quality/codec/source junk dropped anywhere in a movie title.
const MOVIE_JUNK = new Set([
  "hd", "fhd", "uhd", "sd", "hq", "4k", "8k", "hdr", "sdr", "hevc", "avc", "h264", "h265", "x264",
  "x265", "aac", "ac3", "dts", "multi", "dual", "vostfr", "vost", "webdl", "webrip", "hdtv",
  "bluray", "brrip", "bdrip", "hdrip", "dvdrip", "remux", "truefrench",
]);
/// Language tags stripped only at the END of the title (where providers append
/// them, e.g. "… Fr"); never mid-title so "Un Homme en colère" keeps "en".
const TRAILING_LANG = new Set(["fr", "en", "ar", "eng", "ara", "fra", "vo", "vf", "vff", "vfq", "es", "esp", "de", "it", "pt", "tr"]);

/// Grouping key for a movie's language/source versions, derived from the RAW name
/// (which keeps the real title even when `clean_title` is broken). Drops the
/// provider prefix, bracketed groups, year, quality junk and a trailing language
/// tag; keeps numbers so sequels/parts stay distinct. `null` when no real title
/// remains (e.g. year-only names) so those never group.
function movieVersionKey(item: BrowseItem): string | null {
  const raw = item.vod?.name ?? item.title;
  // Everything after the last '|' drops "FR|", "EN-AR|", "NF-FR| FR|" prefixes.
  const afterPipe = raw.includes("|") ? raw.slice(raw.lastIndexOf("|") + 1) : raw;
  const unbracketed = afterPipe.replace(/[([{][^)\]}]*[)\]}]/g, " ");
  const tokens = unbracketed
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean)
    .filter((t) => !MOVIE_JUNK.has(t))
    .filter((t) => !/^(?:19|20)\d{2}$/.test(t));
  // Strip trailing language tags (one or more).
  while (tokens.length > 1 && TRAILING_LANG.has(tokens[tokens.length - 1])) {
    tokens.pop();
  }
  const key = tokens.join(" ").trim();
  // Require some alphabetic content — never group year/number-only names.
  return key && /\p{L}/u.test(key) ? key : null;
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

async function fetchContinueWatching(profileId: string): Promise<ContinueWatchingItem[]> {
  return invoke<ContinueWatchingItem[]>("list_continue_watching", { profileId, limit: RAIL_SIZE });
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
  continueWatching: (profileId: string) =>
    [...browseKeys.all, profileId, "continueWatching"] as const,
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
    queryKey: browseKeys.count(profileId, sourceId ?? -1, tab, filters),
    queryFn: () => fetchCount(tab, profileId, sourceId!, filters),
    enabled: sourceId !== null,
  });
}

export function useCategories(tab: BrowseTab, profileId: string, sourceId: number | null) {
  return useQuery({
    queryKey: browseKeys.categories(profileId, sourceId ?? -1, tab),
    queryFn: () => fetchCategories(tab, profileId, sourceId!),
    enabled: sourceId !== null,
  });
}

export function useRegions(profileId: string, sourceId: number | null) {
  return useQuery({
    queryKey: browseKeys.regions(profileId, sourceId ?? -1),
    queryFn: () => fetchRegions(profileId, sourceId!),
    enabled: sourceId !== null,
  });
}

export function useCountries(profileId: string, sourceId: number | null) {
  return useQuery({
    queryKey: browseKeys.countries(profileId, sourceId ?? -1),
    queryFn: () => fetchCountries(profileId, sourceId!),
    enabled: sourceId !== null,
  });
}

export function useContinueWatching(profileId: string) {
  return useQuery({
    queryKey: browseKeys.continueWatching(profileId),
    queryFn: () => fetchContinueWatching(profileId),
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
    queryKey: browseKeys.rail(profileId, sourceId ?? -1, tab, spec.id),
    queryFn: () => fetchItems(tab, profileId, sourceId!, spec.filters, RAIL_SIZE, 0),
    enabled: enabled && sourceId !== null,
  });
}

/// Search / filter / see-all view: paginated, windowed grid.
export function useBrowseGrid(
  tab: BrowseTab,
  profileId: string,
  sourceId: number | null,
  filters: BrowseFilters
) {
  return useInfiniteQuery({
    queryKey: browseKeys.grid(profileId, sourceId ?? -1, tab, filters),
    queryFn: ({ pageParam }) =>
      fetchItems(tab, profileId, sourceId!, filters, GRID_PAGE_SIZE, pageParam),
    initialPageParam: 0,
    getNextPageParam: (lastPage, allPages) =>
      lastPage.length < GRID_PAGE_SIZE ? undefined : allPages.length * GRID_PAGE_SIZE,
    enabled: sourceId !== null,
    placeholderData: keepPreviousData,
  });
}

// ── Prefetch ────────────────────────────────────────────────────────────────

/// Warm the cheap per-tab metadata (counts + categories) for every tab plus the
/// profile's Continue Watching, so the tab bar and rail skeleton render with no
/// IPC the first time each tab is opened.
export function prefetchSourceOverview(qc: QueryClient, profileId: string, sourceId: number) {
  void qc.prefetchQuery({
    queryKey: browseKeys.continueWatching(profileId),
    queryFn: () => fetchContinueWatching(profileId),
  });
  for (const tab of ["live", "movies", "series"] as const) {
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
  qc.setQueriesData<{ pages: BrowseItem[][]; pageParams: unknown[] }>(
    { queryKey: browseKeys.source(profileId, sourceId), predicate: gridPredicate },
    (data) => (data ? { ...data, pages: data.pages.map((page) => page.map(mapItem)) } : data)
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
}
