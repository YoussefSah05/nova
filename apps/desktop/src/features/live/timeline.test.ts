import { describe, expect, it } from "vitest";

import type { Program } from "@/types";
import { cellsFor, guideWindow, nowAndLater, PX_PER_MIN, ticks, timeLeft } from "./timeline";

const p = (title: string, startUtc: number, stopUtc: number): Program => ({
  channelTvgId: "c",
  title,
  subTitle: null,
  description: null,
  startUtc,
  stopUtc,
  category: null,
  iconUrl: null,
});

describe("guide timeline", () => {
  it("starts half an hour before the current half hour and moves only on the half hour", () => {
    const at = 1_800 * 1_000 + 600; // ten minutes into a half hour
    expect(guideWindow(at).from).toBe(1_800 * 999);
    expect(guideWindow(at + 1_000).from).toBe(guideWindow(at).from);
    expect(guideWindow(at).to - guideWindow(at).from).toBe(12 * 3600);
  });

  it("places programmes by time, clipped to the window, never overlapping", () => {
    const window = { from: 0, to: 7200 };
    const cells = cellsFor(
      [
        p("Before", -600, 600),
        p("Now", 600, 3000),
        p("Overlap", 2400, 4200),
        p("Late", 7000, 9000),
      ],
      window,
      1200
    );
    const r = (x: number) => Math.round(x * 100) / 100;
    expect(cells.map((c) => [c.program.title, r(c.left), r(c.width), c.state])).toEqual([
      ["Before", 0, 10 * PX_PER_MIN, "past"],
      ["Now", 10 * PX_PER_MIN, 40 * PX_PER_MIN, "now"],
      ["Overlap", 50 * PX_PER_MIN, 20 * PX_PER_MIN, "later"],
      ["Late", r((7000 / 60) * PX_PER_MIN), r((200 / 60) * PX_PER_MIN), "later"],
    ]);
    expect(ticks(window).map((t) => t.at)).toEqual([0, 1800, 3600, 5400]);
  });

  it("finds what's on now and later, and the time left", () => {
    const programs = [p("A", 0, 100), p("B", 100, 200), p("C", 200, 4000), p("D", 4000, 5000)];
    const { now, later } = nowAndLater(programs, 150, 2);
    expect(now?.title).toBe("B");
    expect(later.map((x) => x.title)).toEqual(["C", "D"]);
    expect(timeLeft(p("x", 0, 600), 0)).toBe("10 min left");
    expect(timeLeft(p("x", 0, 3900), 0)).toBe("1 h 05 left");
  });
});
