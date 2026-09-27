/* eslint-disable react-refresh/only-export-components -- shared ambient toolkit: sampler + hook + glow component co-located by design */
//! Ambient dynamic accent. Samples the dominant (saturated) colour of a piece of
//! artwork on a tiny offscreen canvas and exposes it as an rgb triplet, plus a
//! soft glow layer that tints the space behind a hero / detail backdrop with it.
//! This is deliberately separate from the brand accent (the theme picker) so a
//! film's own palette can bleed in without fighting the UI's chrome colour.
//!
//! Perf: each URL is sampled once (module cache), on a 12×12 canvas — cheap. The
//! glow is a static blurred gradient (no per-frame work); it only crossfades when
//! the focused artwork changes. Tainted-canvas / CORS failures degrade to no tint.

import { useEffect, useState } from "react";
import { motion, AnimatePresence, useReducedMotion } from "@/lib/motion";
import { artworkSrc } from "@/lib/artwork";
import { cn } from "@/lib/utils";

const cache = new Map<string, string | null>();
const SIZE = 12;

/// Resolve an artwork URL to an `"r, g, b"` triplet (or null if unavailable).
/// Prefers saturated, mid-luminance pixels so the tint takes the artwork's hue
/// rather than its black bars; falls back to a plain average.
export function sampleAmbientColor(url: string): Promise<string | null> {
  const hit = cache.get(url);
  if (hit !== undefined) return Promise.resolve(hit);
  return new Promise((resolve) => {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.decoding = "async";
    img.onload = () => {
      try {
        const canvas = document.createElement("canvas");
        canvas.width = SIZE;
        canvas.height = SIZE;
        const ctx = canvas.getContext("2d", { willReadFrequently: true });
        if (!ctx) return finish(url, null, resolve);
        ctx.drawImage(img, 0, 0, SIZE, SIZE);
        const { data } = ctx.getImageData(0, 0, SIZE, SIZE);
        let r = 0;
        let g = 0;
        let b = 0;
        let n = 0;
        for (let i = 0; i < data.length; i += 4) {
          const cr = data[i];
          const cg = data[i + 1];
          const cb = data[i + 2];
          const max = Math.max(cr, cg, cb);
          const min = Math.min(cr, cg, cb);
          if (max - min < 18 || max < 30 || max > 240) continue; // skip grey/dark/blown
          r += cr;
          g += cg;
          b += cb;
          n += 1;
        }
        if (n === 0) {
          for (let i = 0; i < data.length; i += 4) {
            r += data[i];
            g += data[i + 1];
            b += data[i + 2];
            n += 1;
          }
        }
        finish(url, `${Math.round(r / n)}, ${Math.round(g / n)}, ${Math.round(b / n)}`, resolve);
      } catch {
        finish(url, null, resolve);
      }
    };
    img.onerror = () => finish(url, null, resolve);
    img.src = artworkSrc(url);
  });
}

function finish(url: string, color: string | null, resolve: (c: string | null) => void) {
  cache.set(url, color);
  resolve(color);
}

/// React binding: returns the sampled triplet for `url` (null while pending /
/// unavailable). Re-samples when the URL changes.
export function useAmbientColor(url: string | null | undefined): string | null {
  const [color, setColor] = useState<string | null>(null);
  useEffect(() => {
    if (!url) {
      setColor(null);
      return;
    }
    let active = true;
    void sampleAmbientColor(url).then((c) => {
      if (active) setColor(c);
    });
    return () => {
      active = false;
    };
  }, [url]);
  return color;
}

/// Soft, blurred tint layer driven by an ambient colour. Absolutely-positioned
/// and `pointer-events-none`; drop it as the first child of a `relative` (or
/// `fixed`) container, behind the content. Crossfades when the colour changes.
export function AmbientGlow({ color, className }: { color: string | null; className?: string }) {
  const reduce = useReducedMotion();
  return (
    <AnimatePresence mode="wait">
      {color && (
        <motion.div
          key={color}
          aria-hidden
          className={cn("pointer-events-none absolute inset-0 overflow-hidden", className)}
          initial={reduce ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={reduce ? undefined : { opacity: 0 }}
          transition={{ duration: 0.7, ease: "easeInOut" }}
          style={{
            background: `radial-gradient(60% 55% at 18% 0%, rgba(${color}, 0.38), transparent 70%), radial-gradient(55% 50% at 88% 8%, rgba(${color}, 0.22), transparent 72%)`,
          }}
        />
      )}
    </AnimatePresence>
  );
}
