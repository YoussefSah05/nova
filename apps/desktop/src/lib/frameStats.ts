//! Frame-pacing statistics for the frame-rate overlay.
//!
//! Inputs are intervals between consecutive `requestAnimationFrame`
//! callbacks, in milliseconds. A frame is "janky" when it took noticeably
//! longer than one display refresh (the display skipped at least one frame).

/** Gaps longer than this are pauses (hidden window, debugger), not frames. */
const PAUSE_MS = 500;

export interface FrameSummary {
  frames: number;
  durationMs: number;
  avgFps: number;
  /** Average FPS over the slowest 1% of frames. */
  low1Fps: number;
  p95Ms: number;
  worstMs: number;
  /** Frames longer than 1.5 refresh intervals (at least one frame dropped). */
  janky: number;
  jankyPct: number;
}

/** The display's refresh interval: the median frame interval. */
export function refreshIntervalMs(intervals: readonly number[]): number {
  const usable = intervals.filter((d) => d > 0 && d < PAUSE_MS).sort((a, b) => a - b);
  if (usable.length === 0) return 1000 / 60;
  return usable[Math.floor(usable.length / 2)];
}

export function summarizeFrames(
  intervals: readonly number[],
  refreshMs: number = refreshIntervalMs(intervals)
): FrameSummary | null {
  const usable = intervals.filter((d) => d > 0 && d < PAUSE_MS);
  if (usable.length < 2) return null;
  const sorted = [...usable].sort((a, b) => a - b);
  const durationMs = usable.reduce((sum, d) => sum + d, 0);
  const slowest = sorted.slice(-Math.max(1, Math.ceil(sorted.length / 100)));
  const slowestAvg = slowest.reduce((sum, d) => sum + d, 0) / slowest.length;
  const janky = usable.filter((d) => d > refreshMs * 1.5).length;
  return {
    frames: usable.length,
    durationMs,
    avgFps: (usable.length * 1000) / durationMs,
    low1Fps: 1000 / slowestAvg,
    p95Ms: sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.95))],
    worstMs: sorted[sorted.length - 1],
    janky,
    jankyPct: (janky * 100) / usable.length,
  };
}

/** One line per measurement, ready to paste into a report. */
export function formatSummary(label: string, s: FrameSummary): string {
  return (
    `${label}: ${(s.durationMs / 1000).toFixed(1)} s, ${s.frames} frames, ` +
    `avg ${s.avgFps.toFixed(0)} fps, 1% low ${s.low1Fps.toFixed(0)} fps, ` +
    `p95 ${s.p95Ms.toFixed(1)} ms, worst ${s.worstMs.toFixed(0)} ms, ` +
    `janky ${s.janky} (${s.jankyPct.toFixed(1)}%)`
  );
}
