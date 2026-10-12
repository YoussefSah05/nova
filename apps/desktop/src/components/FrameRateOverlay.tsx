//! Frame-rate overlay for measuring scroll smoothness on real libraries.
//!
//! Toggle with ⌘⌥F (remembered on this Mac). While off it registers only the
//! shortcut: no animation-frame loop, no scroll listener. While on, it records
//! every frame interval, shows the live frame rate, and summarizes each scroll
//! gesture (it ends 300 ms after the last scroll event) with the screen it
//! happened on. "Copy report" puts every summary on the clipboard.
//!
//! The panel is solid (never blurred) and re-renders at most twice a second,
//! so it does not distort what it measures.

import { useEffect, useRef, useState } from "react";

import {
  formatSummary,
  refreshIntervalMs,
  summarizeFrames,
  type FrameSummary,
} from "@/lib/frameStats";

const STORAGE_KEY = "nova.frameRateOverlay";
const SCROLL_END_MS = 300;
const MAX_SESSIONS = 8;
/** Live readout window: the last ~2 s of frames. */
const LIVE_FRAMES = 240;

interface ScrollSession {
  label: string;
  summary: FrameSummary;
}

function readEnabled(): boolean {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function writeEnabled(on: boolean) {
  try {
    if (on) window.localStorage.setItem(STORAGE_KEY, "1");
    else window.localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Private storage unavailable: the toggle still works for this session.
  }
}

function screenLabel(target: EventTarget | null): string {
  const el = target instanceof Element ? target : document.activeElement;
  const screen = el?.closest?.(".nova-screen");
  return screen?.getAttribute("aria-label") || "Screen";
}

export function FrameRateOverlay() {
  const [enabled, setEnabled] = useState(readEnabled);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.metaKey && e.altKey && e.code === "KeyF") {
        e.preventDefault();
        setEnabled((on) => {
          writeEnabled(!on);
          return !on;
        });
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const hide = () => {
    writeEnabled(false);
    setEnabled(false);
  };
  return enabled ? <FrameRatePanel onClose={hide} /> : null;
}

function FrameRatePanel({ onClose }: { onClose: () => void }) {
  const [live, setLive] = useState<{ fps: number; refreshHz: number } | null>(null);
  const [scrolling, setScrolling] = useState<string | null>(null);
  const [sessions, setSessions] = useState<ScrollSession[]>([]);
  const [copied, setCopied] = useState(false);
  const recent = useRef<number[]>([]);

  useEffect(() => {
    let frame = 0;
    let last = 0;
    let lastPublish = 0;
    let session: { label: string; intervals: number[] } | null = null;
    let endTimer = 0;

    const tick = (now: number) => {
      if (last) {
        const interval = now - last;
        const buffer = recent.current;
        buffer.push(interval);
        if (buffer.length > LIVE_FRAMES) buffer.shift();
        session?.intervals.push(interval);
        if (now - lastPublish > 500) {
          lastPublish = now;
          const summary = summarizeFrames(buffer.slice(-120));
          if (summary) {
            setLive({
              fps: summary.avgFps,
              refreshHz: 1000 / refreshIntervalMs(buffer),
            });
          }
        }
      }
      last = now;
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);

    const finish = () => {
      endTimer = 0;
      if (!session) return;
      const done = session;
      session = null;
      setScrolling(null);
      const summary = summarizeFrames(done.intervals, refreshIntervalMs(recent.current));
      if (summary && summary.frames >= 10) {
        setSessions((list) => [{ label: done.label, summary }, ...list].slice(0, MAX_SESSIONS));
      }
    };
    const onScroll = (event: Event) => {
      if (!session) {
        session = { label: screenLabel(event.target), intervals: [] };
        setScrolling(session.label);
      }
      window.clearTimeout(endTimer);
      endTimer = window.setTimeout(finish, SCROLL_END_MS);
    };
    // Scroll events don't bubble; capture sees every scroller (screens and rails).
    document.addEventListener("scroll", onScroll, { capture: true, passive: true });
    return () => {
      cancelAnimationFrame(frame);
      window.clearTimeout(endTimer);
      document.removeEventListener("scroll", onScroll, true);
    };
  }, []);

  const report = () =>
    [
      `Nova frame-rate report · display ~${live ? live.refreshHz.toFixed(0) : "?"} Hz`,
      ...[...sessions].reverse().map((s) => formatSummary(s.label, s.summary)),
    ].join("\n");

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(report());
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
    }
  };

  const tone = (fps: number, hz: number) =>
    fps >= hz * 0.9 ? "text-emerald-400" : fps >= hz * 0.7 ? "text-amber-300" : "text-red-400";

  return (
    <div
      role="status"
      aria-label="Frame rate"
      className="fixed bottom-4 right-4 z-[1000] w-[22rem] rounded-xl border border-white/10 bg-black/90 p-3 font-mono text-[11px] leading-relaxed text-white/80 shadow-2xl"
    >
      <div className="flex items-center justify-between gap-2">
        <span>
          <span className={live ? tone(live.fps, live.refreshHz) : ""}>
            {live ? `${live.fps.toFixed(0)} fps` : "measuring…"}
          </span>
          {live && (
            <span className="text-white/50"> · display ~{live.refreshHz.toFixed(0)} Hz</span>
          )}
        </span>
        <span className="flex gap-1">
          <button
            type="button"
            onClick={() => void copy()}
            disabled={sessions.length === 0}
            className="rounded bg-white/10 px-2 py-0.5 text-white hover:bg-white/20 disabled:opacity-40"
          >
            {copied ? "Copied" : "Copy report"}
          </button>
          <button
            type="button"
            onClick={() => setSessions([])}
            className="rounded bg-white/10 px-2 py-0.5 hover:bg-white/20"
          >
            Clear
          </button>
          <button
            type="button"
            onClick={onClose}
            aria-label="Hide frame rate"
            className="rounded bg-white/10 px-2 py-0.5 hover:bg-white/20"
          >
            ✕
          </button>
        </span>
      </div>
      <p className="mt-1 text-white/50">
        {scrolling
          ? `Recording scroll on ${scrolling}…`
          : "Scroll a screen to record it · ⌘⌥F hides"}
      </p>
      {sessions.length > 0 && (
        <ul className="mt-2 space-y-1.5 border-t border-white/10 pt-2">
          {sessions.map((s, i) => (
            <li key={sessions.length - i}>
              <span className="text-white">{s.label}</span>{" "}
              <span className={tone(s.summary.avgFps, live?.refreshHz ?? 60)}>
                avg {s.summary.avgFps.toFixed(0)}
              </span>
              <span className="text-white/60">
                {" "}
                · 1% low {s.summary.low1Fps.toFixed(0)} · worst {s.summary.worstMs.toFixed(0)} ms ·
                janky {s.summary.jankyPct.toFixed(1)}%
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
