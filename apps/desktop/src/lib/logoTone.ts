//! Brand-coloured channel tiles: how a channel logo should be presented.
//!
//! Provider logos come in three shapes, and one fixed grey box suits none of
//! them. Each logo is sampled once on a tiny canvas and classified:
//!
//! - `fill`: the logo has its own solid background (a JPEG box). The tile takes
//!   exactly that edge colour, so the box melts into the tile.
//! - `cover`: the logo is a picture with busy, opaque edges (a photo/banner).
//!   It fills the tile edge to edge.
//! - `tint`: a transparent logo with light or coloured ink. A deep gradient in
//!   the logo's own hue (neutral slate for white/grey logos).
//! - `light`: a transparent logo with dark ink, which would vanish on a dark
//!   tile. A light, faintly hue-tinted tile.
//!
//! Results are cached per URL and persisted on this Mac, so a logo is analysed
//! once, not on every launch. Sampling waits until scrolling pauses, decodes
//! the image off the main thread first, and runs at most a few at a time.
//! Unreadable logos (cross-origin, broken) get the neutral tint for the
//! session only, so a logo that comes back later is analysed then.

import { useEffect, useState } from "react";

import { artworkSrc } from "@/lib/artwork";
import { whenScrollIdle } from "@/lib/scrollActivity";

export type LogoMode = "fill" | "cover" | "tint" | "light";

export interface LogoTone {
  mode: LogoMode;
  /** CSS `background` for the tile. */
  background: string;
}

export const NEUTRAL_TONE: LogoTone = {
  mode: "tint",
  background: "linear-gradient(135deg, #2a2d36, #15171c)",
};

const SIZE = 40;
const EDGE = 2;

function luminance(r: number, g: number, b: number): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function hueSat(r: number, g: number, b: number): { h: number; s: number } {
  const [rn, gn, bn] = [r / 255, g / 255, b / 255];
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const d = max - min;
  if (d === 0) return { h: 0, s: 0 };
  const l = (max + min) / 2;
  const s = d / (1 - Math.abs(2 * l - 1));
  let h: number;
  if (max === rn) h = ((gn - bn) / d) % 6;
  else if (max === gn) h = (bn - rn) / d + 2;
  else h = (rn - gn) / d + 4;
  return { h: Math.round((h * 60 + 360) % 360), s };
}

/**
 * Classify a logo from its RGBA pixels (`width * height * 4`). Pure, so it is
 * unit-tested without a canvas.
 */
export function analyzeLogoPixels(
  data: ArrayLike<number>,
  width: number,
  height: number
): LogoTone {
  let edgeN = 0;
  let edgeOpaque = 0;
  const edgeSum = [0, 0, 0];
  const edgeSq = [0, 0, 0];
  let inkN = 0;
  let inkLum = 0;
  const satSum = [0, 0, 0];
  let satN = 0;

  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      const [r, g, b, a] = [data[i], data[i + 1], data[i + 2], data[i + 3]];
      const edge = x < EDGE || y < EDGE || x >= width - EDGE || y >= height - EDGE;
      if (edge) {
        edgeN += 1;
        if (a >= 230) {
          edgeOpaque += 1;
          edgeSum[0] += r;
          edgeSum[1] += g;
          edgeSum[2] += b;
          edgeSq[0] += r * r;
          edgeSq[1] += g * g;
          edgeSq[2] += b * b;
        }
      }
      if (a >= 128) {
        inkN += 1;
        inkLum += luminance(r, g, b);
        const max = Math.max(r, g, b);
        const min = Math.min(r, g, b);
        if (max - min >= 40 && max >= 50) {
          satSum[0] += r;
          satSum[1] += g;
          satSum[2] += b;
          satN += 1;
        }
      }
    }
  }

  // Opaque edges: the logo brings its own background.
  if (edgeN > 0 && edgeOpaque / edgeN >= 0.9) {
    const mean = edgeSum.map((s) => s / edgeOpaque);
    const spread = Math.max(
      ...edgeSq.map((sq, c) => Math.sqrt(Math.max(0, sq / edgeOpaque - mean[c] ** 2)))
    );
    const rgb = `rgb(${mean.map((m) => Math.round(m)).join(", ")})`;
    return spread < 24 ? { mode: "fill", background: rgb } : { mode: "cover", background: rgb };
  }

  if (inkN < width * height * 0.02) return NEUTRAL_TONE;

  const hue =
    satN >= inkN * 0.08 ? hueSat(satSum[0] / satN, satSum[1] / satN, satSum[2] / satN) : null;
  if (inkLum / inkN < 0.2) {
    // Dark ink needs a light tile.
    const h = hue?.h ?? 220;
    const s = hue ? 28 : 8;
    return {
      mode: "light",
      background: `linear-gradient(135deg, hsl(${h} ${s}% 96%), hsl(${h} ${s}% 84%))`,
    };
  }
  if (!hue) return NEUTRAL_TONE;
  const s = Math.round(Math.min(hue.s, 0.5) * 100);
  return {
    mode: "tint",
    background: `linear-gradient(135deg, hsl(${hue.h} ${s}% 21%), hsl(${(hue.h + 24) % 360} ${Math.round(s * 0.8)}% 10%))`,
  };
}

const STORAGE_KEY = "nova.logoTones.v1";
const MAX_STORED = 15_000;
const CONCURRENCY = 4;
const MODES: readonly LogoMode[] = ["fill", "cover", "tint", "light"];

const cache = new Map<string, LogoTone>();
const pending = new Map<string, Promise<LogoTone>>();
/** Tones worth keeping across launches: stored ones plus successful analyses. */
const persisted = new Map<string, LogoTone>();
let loaded = false;
let saveTimer: ReturnType<typeof setTimeout> | undefined;
let running = 0;
const queue: (() => void)[] = [];

function isTone(value: unknown): value is LogoTone {
  if (typeof value !== "object" || value === null) return false;
  const tone = value as Record<string, unknown>;
  return (
    typeof tone.background === "string" &&
    typeof tone.mode === "string" &&
    (MODES as readonly string[]).includes(tone.mode)
  );
}

/** Load persisted tones once; storage may be unavailable or corrupt. */
function loadStored() {
  if (loaded) return;
  loaded = true;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    if (parsed && typeof parsed === "object") {
      for (const [url, tone] of Object.entries(parsed)) {
        if (!isTone(tone)) continue;
        cache.set(url, tone);
        persisted.set(url, tone);
      }
    }
  } catch {
    // Start empty: tones are recomputed as logos appear.
  }
}

function scheduleSave() {
  if (saveTimer !== undefined) return;
  saveTimer = setTimeout(() => {
    saveTimer = undefined;
    try {
      const entries = [...persisted.entries()].slice(-MAX_STORED);
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(Object.fromEntries(entries)));
    } catch {
      // Quota or private storage: the in-memory cache still serves this session.
    }
  }, 2_000);
}

/** Run `task` when a sampling slot is free. */
function limited<T>(task: () => Promise<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const start = () => {
      running += 1;
      task()
        .then(resolve, reject)
        .finally(() => {
          running -= 1;
          queue.shift()?.();
        });
    };
    if (running < CONCURRENCY) start();
    else queue.push(start);
  });
}

async function analyse(url: string): Promise<{ tone: LogoTone; keep: boolean }> {
  // Optional work: never while content is moving.
  await whenScrollIdle();
  const img = new Image();
  img.crossOrigin = "anonymous";
  img.decoding = "async";
  img.src = artworkSrc(url);
  try {
    // Decode off the main thread; drawing an undecoded image would decode it
    // synchronously in the middle of a frame.
    await img.decode();
  } catch {
    return { tone: NEUTRAL_TONE, keep: false };
  }
  try {
    const canvas = document.createElement("canvas");
    canvas.width = SIZE;
    canvas.height = SIZE;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return { tone: NEUTRAL_TONE, keep: false };
    ctx.drawImage(img, 0, 0, SIZE, SIZE);
    return {
      tone: analyzeLogoPixels(ctx.getImageData(0, 0, SIZE, SIZE).data, SIZE, SIZE),
      keep: true,
    };
  } catch {
    return { tone: NEUTRAL_TONE, keep: false }; // tainted canvas
  }
}

/** A cached tone, if this logo was analysed before (this launch or earlier). */
export function cachedLogoTone(url: string): LogoTone | undefined {
  loadStored();
  return cache.get(url);
}

/** Sample a logo (through the artwork cache) and classify it; cached. */
export function sampleLogoTone(url: string): Promise<LogoTone> {
  const hit = cachedLogoTone(url);
  if (hit) return Promise.resolve(hit);
  const inflight = pending.get(url);
  if (inflight) return inflight;
  const promise = limited(() => analyse(url)).then(({ tone, keep }) => {
    cache.set(url, tone);
    pending.delete(url);
    if (keep) {
      persisted.set(url, tone);
      scheduleSave();
    }
    return tone;
  });
  pending.set(url, promise);
  return promise;
}

/** Test hook: forget every cached and persisted tone. */
export function resetLogoTones() {
  cache.clear();
  pending.clear();
  persisted.clear();
  loaded = false;
}

/** The tone for `url`; `null` while sampling or without a URL. */
export function useLogoTone(url: string | null | undefined): LogoTone | null {
  const [tone, setTone] = useState<LogoTone | null>(() =>
    url ? (cachedLogoTone(url) ?? null) : null
  );
  useEffect(() => {
    if (!url) {
      setTone(null);
      return;
    }
    const hit = cachedLogoTone(url);
    if (hit) {
      setTone(hit);
      return;
    }
    setTone(null);
    let active = true;
    void sampleLogoTone(url).then((t) => {
      if (active) setTone(t);
    });
    return () => {
      active = false;
    };
  }, [url]);
  return tone;
}
