//! Deterministic, zero-network placeholder shown when a card has no artwork
//! (no TMDB match and no provider logo). A stable hue is hashed from the title so
//! the same item always gets the same colour. Posters (vod/series) show the title;
//! live tiles show brand initials on a wide gradient banner — so *every* card has
//! a deliberate visual, including live channels that TMDB can't cover.

import { memo } from "react";
import { cn } from "@/lib/utils";

/// Stable 0–359 hue from a string (small FNV-ish rolling hash).
function hashHue(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i += 1) h = (Math.imul(h, 31) + s.charCodeAt(i)) | 0;
  return Math.abs(h) % 360;
}

/// Up to two letters for the live banner; Unicode-aware so non-Latin brands work.
function initials(title: string): string {
  const words = title
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return [...words[0]].slice(0, 2).join("").toUpperCase();
  return ([...words[0]][0] + [...words[1]][0]).toUpperCase();
}

export const PosterFallback = memo(function PosterFallback({
  title,
  live = false,
  className,
}: {
  title: string;
  live?: boolean;
  className?: string;
}) {
  const hue = hashHue(title);
  const background = `linear-gradient(135deg, hsl(${hue} 42% 24%), hsl(${(hue + 38) % 360} 38% 12%))`;
  return (
    <div
      className={cn("flex h-full w-full items-center justify-center p-3 text-center", className)}
      style={{ background }}
      aria-hidden
    >
      {live ? (
        <span className="text-lg font-bold tracking-wide text-white/90">{initials(title)}</span>
      ) : (
        <span className="line-clamp-3 text-xs font-medium text-white/85">{title}</span>
      )}
    </div>
  );
});
