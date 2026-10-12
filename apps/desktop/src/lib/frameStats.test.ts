import { describe, expect, it } from "vitest";
import { formatSummary, refreshIntervalMs, summarizeFrames } from "./frameStats";

describe("frame statistics", () => {
  it("reports a steady 60 Hz scroll as smooth", () => {
    const s = summarizeFrames(Array(120).fill(1000 / 60))!;
    expect(s.avgFps).toBeCloseTo(60, 5);
    expect(s.janky).toBe(0);
    expect(s.low1Fps).toBeCloseTo(60, 5);
  });

  it("counts dropped frames and the slowest 1%", () => {
    const intervals = [...Array(198).fill(1000 / 60), 50, 100];
    const s = summarizeFrames(intervals)!;
    expect(s.janky).toBe(2);
    expect(s.worstMs).toBe(100);
    expect(s.low1Fps).toBeCloseTo(1000 / 75, 5);
    expect(s.avgFps).toBeLessThan(60);
  });

  it("measures the display rate and ignores pauses", () => {
    const at120 = [...Array(50).fill(1000 / 120), 2000];
    expect(1000 / refreshIntervalMs(at120)).toBeCloseTo(120, 5);
    expect(summarizeFrames(at120)!.frames).toBe(50);
    expect(summarizeFrames([5000])).toBeNull();
  });

  it("formats a pasteable line", () => {
    const line = formatSummary("Movies", summarizeFrames(Array(60).fill(1000 / 60))!);
    expect(line).toMatch(/^Movies: 1\.0 s, 60 frames, avg 60 fps, 1% low 60 fps/);
  });
});
