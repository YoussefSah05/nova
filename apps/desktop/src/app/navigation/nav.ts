//! In-app navigation: one stack of screens per section, the tvOS/iOS tab
//! model. Switching sections keeps each section exactly where it was (its
//! pushed pages and, because screens stay mounted, its scroll position);
//! re-selecting the current section goes back to its root.
//!
//! Pure data + reducer (no React), so it is unit-tested directly.

import type { SeriesSummary } from "@/types";
import { seriesKey, type BrowseItem } from "@/features/browse/queries";

export const SECTIONS = ["home", "movies", "series", "live", "library"] as const;
export type SectionId = (typeof SECTIONS)[number];

export const SECTION_LABELS: Record<SectionId, string> = {
  home: "Home",
  movies: "Movies",
  series: "Series",
  live: "Live",
  library: "Library",
};

/// A screen. Pages carry the card they were opened from as a seed, so they
/// paint at once; live data then comes from the query cache.
export type Route =
  | { kind: "root" }
  | { kind: "title"; item: BrowseItem }
  | { kind: "series"; series: SeriesSummary }
  /// The search page, opened with what was typed in the palette.
  | { kind: "search"; query: string };

export interface Entry {
  key: string;
  route: Route;
}

export interface NavState {
  section: SectionId;
  stacks: Record<SectionId, Entry[]>;
  /// Monotonic id source for entry keys.
  seq: number;
}

export type NavAction =
  | { type: "switch"; section: SectionId }
  | { type: "push"; route: Route }
  | { type: "pop" }
  | { type: "popToRoot" }
  /// Every section back to its root (the playlist changed: pages belong to it).
  | { type: "resetAll" }
  | {
      type: "patchItem";
      key: string;
      patch: Partial<Pick<BrowseItem, "isFavorite" | "isInWatchlist">>;
    };

/// Deepest a stack may grow; older pages are dropped from the bottom (above
/// the root) so an endless "More like this" chain can't hold every page.
export const MAX_DEPTH = 8;

function rootEntry(section: SectionId): Entry {
  return { key: `${section}:root`, route: { kind: "root" } };
}

export function initialNav(section: SectionId = "home"): NavState {
  const stacks = Object.fromEntries(SECTIONS.map((s) => [s, [rootEntry(s)]])) as Record<
    SectionId,
    Entry[]
  >;
  return { section, stacks, seq: 0 };
}

export function currentEntry(state: NavState): Entry {
  const stack = state.stacks[state.section];
  return stack[stack.length - 1];
}

export function canGoBack(state: NavState): boolean {
  return state.stacks[state.section].length > 1;
}

function withStack(state: NavState, stack: Entry[]): NavState {
  return { ...state, stacks: { ...state.stacks, [state.section]: stack } };
}

/// The route's identity: pushing the page already on top is a no-op.
function routeId(route: Route): string {
  switch (route.kind) {
    case "root":
      return "root";
    case "title":
      return `title:${route.item.key}`;
    case "series":
      return seriesKey(route.series);
    case "search":
      return `search:${route.query}`;
  }
}

export function navReducer(state: NavState, action: NavAction): NavState {
  switch (action.type) {
    case "switch":
      if (action.section === state.section) {
        return canGoBack(state) ? navReducer(state, { type: "popToRoot" }) : state;
      }
      return { ...state, section: action.section };
    case "push": {
      const stack = state.stacks[state.section];
      if (routeId(stack[stack.length - 1].route) === routeId(action.route)) return state;
      const seq = state.seq + 1;
      let next = [...stack, { key: `${state.section}:${seq}`, route: action.route }];
      if (next.length > MAX_DEPTH) next = [next[0], ...next.slice(next.length - MAX_DEPTH + 1)];
      return { ...withStack(state, next), seq };
    }
    case "pop": {
      const stack = state.stacks[state.section];
      return stack.length > 1 ? withStack(state, stack.slice(0, -1)) : state;
    }
    case "popToRoot": {
      const stack = state.stacks[state.section];
      return stack.length > 1 ? withStack(state, [stack[0]]) : state;
    }
    case "resetAll": {
      if (SECTIONS.every((s) => state.stacks[s].length === 1)) return state;
      const stacks = Object.fromEntries(SECTIONS.map((s) => [s, [state.stacks[s][0]]])) as Record<
        SectionId,
        Entry[]
      >;
      return { ...state, stacks };
    }
    case "patchItem": {
      let changed = false;
      const stacks = { ...state.stacks };
      for (const section of SECTIONS) {
        const patched = stacks[section].map((entry) => {
          const route = entry.route;
          if (route.kind === "title" && route.item.key === action.key) {
            changed = true;
            return { ...entry, route: { ...route, item: { ...route.item, ...action.patch } } };
          }
          if (route.kind === "series" && seriesKey(route.series) === action.key) {
            changed = true;
            return { ...entry, route: { ...route, series: { ...route.series, ...action.patch } } };
          }
          return entry;
        });
        stacks[section] = patched;
      }
      return changed ? { ...state, stacks } : state;
    }
  }
}
