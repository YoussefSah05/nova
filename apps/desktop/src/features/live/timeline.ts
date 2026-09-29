//! The guide's timeline: the window it shows, where each programme sits in
//! it, the half-hour ticks, and what's later on a channel. Times are epoch
//! seconds (as the guide stores them); positions are pixels.

import type { Program } from "@/types";

const HALF_HOUR = 30 * 60;
/// How much guide one screen reads: half an hour back, the rest ahead.
export const WINDOW_SECS = 12 * 3600;
/// Horizontal scale: pixels per minute.
export const PX_PER_MIN = 5;

export interface GuideWindow {
  from: number;
  to: number;
}

/// Starts on the half hour before the current one, so what's on now always
/// has its start in view; moves only every half hour (stable query keys).
export function guideWindow(nowSec: number): GuideWindow {
  const from = Math.floor(nowSec / HALF_HOUR) * HALF_HOUR - HALF_HOUR;
  return { from, to: from + WINDOW_SECS };
}

export function xFor(timeSec: number, window: GuideWindow): number {
  return ((timeSec - window.from) / 60) * PX_PER_MIN;
}

export const TIMELINE_WIDTH = (WINDOW_SECS / 60) * PX_PER_MIN;

export type CellState = "past" | "now" | "later";

export interface GuideCell {
  program: Program;
  left: number;
  width: number;
  state: CellState;
}

/// Programmes placed on the timeline, clipped to the window. Overlapping
/// programmes (feeds do overlap) never draw over each other: each starts no
/// earlier than the previous one ends.
export function cellsFor(programs: Program[], window: GuideWindow, nowSec: number): GuideCell[] {
  const cells: GuideCell[] = [];
  let edge = window.from;
  for (const p of programs) {
    const start = Math.max(p.startUtc, window.from, edge);
    const stop = Math.min(p.stopUtc, window.to);
    if (stop <= start) continue;
    edge = stop;
    cells.push({
      program: p,
      left: xFor(start, window),
      width: xFor(stop, window) - xFor(start, window),
      state: p.stopUtc <= nowSec ? "past" : p.startUtc <= nowSec ? "now" : "later",
    });
  }
  return cells;
}

/// Half-hour ticks across the window.
export function ticks(window: GuideWindow): { at: number; x: number }[] {
  const out = [];
  for (let at = window.from; at < window.to; at += HALF_HOUR) out.push({ at, x: xFor(at, window) });
  return out;
}

/// What's airing at `nowSec`, and up to `count` programmes after it.
export function nowAndLater(
  programs: Program[],
  nowSec: number,
  count = 4
): { now: Program | null; later: Program[] } {
  const now =
    [...programs].reverse().find((p) => p.startUtc <= nowSec && p.stopUtc > nowSec) ?? null;
  const later = programs.filter((p) => p.startUtc > nowSec).slice(0, count);
  return { now, later };
}

/// `12 min left`, `1 h 05 left`.
export function timeLeft(p: Program, nowSec: number): string {
  const min = Math.max(1, Math.round((p.stopUtc - nowSec) / 60));
  if (min < 60) return `${min} min left`;
  return `${Math.floor(min / 60)} h ${String(min % 60).padStart(2, "0")} left`;
}
