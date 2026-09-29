//! The film's sources, with Nova's pick explained. Choosing one plays it
//! (Rust checks it belongs to the film).

import { memo } from "react";

import type { VariantSummary } from "@/types";
import { cn } from "@/lib/utils";
import { orderVersions, versionReason } from "./versions";

export const VersionsPanel = memo(function VersionsPanel({
  variants,
  preferredLang,
  onPlay,
}: {
  variants: VariantSummary[];
  preferredLang: string | null;
  onPlay: (variantId: string) => void;
}) {
  if (variants.length === 0) return null;
  const ordered = orderVersions(variants);
  const playable = variants.filter((v) => v.available).length;
  return (
    <aside
      aria-labelledby="movie-versions"
      className="flex w-[384px] flex-col gap-1.5 rounded-2xl border border-white/[0.08] bg-[#111113]/[0.92] p-[18px] shadow-[0_8px_24px_rgba(0,0,0,0.45),inset_0_1px_0_rgba(255,255,255,0.06)]"
    >
      <div className="mx-1.5 mb-1.5 flex items-baseline gap-2">
        <h2 id="movie-versions" className="text-base font-medium">
          Versions
        </h2>
        <span className="text-xs text-white/60">
          {playable} {playable === 1 ? "source" : "sources"} · Nova picks for you
        </span>
      </div>
      {ordered.map((v) => (
        <button
          key={v.variantId}
          type="button"
          disabled={!v.available}
          onClick={() => onPlay(v.variantId)}
          aria-label={`Play ${v.sourceName}${v.badges.length ? ` ${v.badges.join(" ")}` : ""}${v.picked ? " (Nova's pick)" : ""}`}
          className={cn(
            "flex flex-col gap-1.5 rounded-xl border p-3 text-left transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
            v.picked
              ? "border-primary/35 bg-white/[0.06]"
              : "border-transparent hover:bg-white/[0.05]"
          )}
        >
          <span className="flex items-center gap-2">
            <span
              aria-hidden
              className={cn(
                "h-2 w-2 shrink-0 rounded-full",
                v.picked ? "bg-primary shadow-[0_0_0_3px_rgba(224,163,62,0.2)]" : "bg-white/20"
              )}
            />
            <span className="truncate text-sm font-medium">{v.sourceName}</span>
            <span className="flex-1" />
            {v.badges.map((b) => (
              <span
                key={b}
                className="rounded-[5px] bg-white/10 px-1.5 py-px text-xs font-medium text-white/85 shadow-[inset_0_0_0_1px_rgba(255,255,255,0.12)]"
              >
                {b}
              </span>
            ))}
          </span>
          <span className="pl-4 text-xs leading-relaxed text-white/60">
            {versionReason(v, preferredLang)}
          </span>
        </button>
      ))}
    </aside>
  );
});
