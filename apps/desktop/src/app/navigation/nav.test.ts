import { describe, expect, it } from "vitest";

import type { BrowseItem } from "@/features/browse/queries";
import { canGoBack, currentEntry, initialNav, MAX_DEPTH, navReducer, type NavState } from "./nav";

function title(key: string): BrowseItem {
  return {
    key,
    kind: "movie",
    id: 0,
    title: key,
    subtitle: null,
    posterUrl: null,
    tvgId: null,
    isFavorite: false,
    isInWatchlist: false,
  };
}

const push = (s: NavState, key: string) =>
  navReducer(s, { type: "push", route: { kind: "title", item: title(key) } });

describe("navigation", () => {
  it("keeps one stack per section across switches", () => {
    let s = initialNav();
    s = navReducer(s, { type: "switch", section: "movies" });
    s = push(s, "heat");
    s = navReducer(s, { type: "switch", section: "series" });
    expect(currentEntry(s).route.kind).toBe("root");
    s = navReducer(s, { type: "switch", section: "movies" });
    const top = currentEntry(s).route;
    expect(top.kind === "title" && top.item.key).toBe("heat");
  });

  it("re-selecting the current section returns to its root", () => {
    let s = navReducer(initialNav(), { type: "switch", section: "movies" });
    s = push(push(s, "a"), "b");
    s = navReducer(s, { type: "switch", section: "movies" });
    expect(canGoBack(s)).toBe(false);
    expect(s.stacks.movies).toHaveLength(1);
  });

  it("pops, never below the root, and ignores pushing the page already shown", () => {
    let s = push(initialNav(), "a");
    const again = push(s, "a");
    expect(again).toBe(s);
    s = navReducer(s, { type: "pop" });
    expect(navReducer(s, { type: "pop" })).toBe(s);
    expect(currentEntry(s).key).toBe("home:root");
  });

  it("gives every pushed page a unique key and bounds the depth", () => {
    let s = initialNav();
    for (let i = 0; i < MAX_DEPTH + 5; i++) s = push(s, `t${i}`);
    const stack = s.stacks.home;
    expect(stack).toHaveLength(MAX_DEPTH);
    expect(stack[0].key).toBe("home:root");
    expect(new Set(stack.map((e) => e.key)).size).toBe(stack.length);
    const top = currentEntry(s).route;
    expect(top.kind === "title" && top.item.key).toBe(`t${MAX_DEPTH + 4}`);
  });

  it("patches an open page's flags in every section", () => {
    let s = push(initialNav(), "heat");
    s = navReducer(s, { type: "switch", section: "movies" });
    s = push(s, "heat");
    s = navReducer(s, { type: "patchItem", key: "heat", patch: { isFavorite: true } });
    for (const section of ["home", "movies"] as const) {
      const route = s.stacks[section][1].route;
      expect(route.kind === "title" && route.item.isFavorite).toBe(true);
    }
    expect(navReducer(s, { type: "patchItem", key: "nope", patch: { isFavorite: true } })).toBe(s);
  });
});
