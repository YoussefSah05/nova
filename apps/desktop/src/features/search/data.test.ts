import { describe, expect, it } from "vitest";

import { interpret } from "@/ui/interpret";
import { channelsApply, momentsApply, planFrom, seriesApply, shows } from "./data";

const plan = (q: string) => {
  const r = interpret(q, ["Drama", "Crime"]);
  return planFrom(r.text, r.facets);
};

describe("search plans", () => {
  it("turns a decade or a year into a year range and names the groups to search", () => {
    expect(plan("crime 90s")).toMatchObject({ genre: "Crime", yearFrom: 1990, yearTo: 1999 });
    expect(plan("drama from 2001")).toMatchObject({ yearFrom: 2001, yearTo: 2001 });
    const showsOnly = plan("crime shows");
    expect(showsOnly.types).toEqual(["series"]);
    expect(shows(showsOnly, "movies")).toBe(false);
    expect(seriesApply(showsOnly)).toBe(true);
  });

  it("keeps channels and dialogue to plain words", () => {
    expect(channelsApply(plan("bbc news"))).toBe(true);
    expect(channelsApply(plan("crime 90s"))).toBe(false);
    expect(momentsApply(plan("see you later"))).toBe(true);
    expect(momentsApply(plan("a"))).toBe(false);
    expect(momentsApply(plan("𐐀"))).toBe(false);
    expect(momentsApply(plan("go"))).toBe(true);
    expect(momentsApply(plan("crime shows"))).toBe(false);
    expect(seriesApply(plan("drama 90s")), "shows have no year filter").toBe(false);
  });
});
