//! Programme guide views: the provider that scopes guides to the active
//! profile, a one-line "now" for cards and a now/next summary.

import { useEffect, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";

import { cn } from "@/lib/utils";
import type { ChannelNowAndNext, Program } from "@/types";
import { formatGuideTime, GuideProfile, guideKeys, programmeProgress, useMinute } from "./guide";

/// Makes the active profile's guide available to catalog cards, and drops
/// cached guides once a source's guide refresh job finishes.
export function GuideProvider({ profileId, children }: { profileId: string; children: ReactNode }) {
  const qc = useQueryClient();
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen("epg:refresh:done", () => {
      void qc.invalidateQueries({ queryKey: guideKeys.all });
      // Guide coverage in the Live header.
      void qc.invalidateQueries({ predicate: (q) => q.queryKey.includes("liveOverview") });
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [qc]);
  return <GuideProfile.Provider value={profileId}>{children}</GuideProfile.Provider>;
}

// ── Presentation ──────────────────────────────────────────────────────────────
// Inline elements only: these render inside card and tile buttons.

function ProgressBar({ programme, className }: { programme: Program; className?: string }) {
  useMinute();
  const pct = Math.round(programmeProgress(programme));
  return (
    <span
      role="progressbar"
      aria-label={`${programme.title}: ${pct}% elapsed`}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct}
      className={cn("block h-0.5 w-full overflow-hidden rounded-full bg-white/10", className)}
    >
      <span
        className="block h-full rounded-full bg-primary/80 transition-[width] duration-500"
        style={{ width: `${pct}%` }}
      />
    </span>
  );
}

/// One line for a card: the current programme and its elapsed share. Renders
/// nothing without a current programme.
export function GuideLine({
  guide,
  className,
}: {
  guide: ChannelNowAndNext | undefined;
  className?: string;
}) {
  if (!guide?.now) return null;
  return (
    <span className={cn("flex min-w-0 flex-col gap-1", className)}>
      <span className="block truncate text-xs text-muted-foreground" title={guide.now.title}>
        <span className="text-foreground/80">Now</span> · {guide.now.title}
      </span>
      <ProgressBar programme={guide.now} />
    </span>
  );
}

/// Now (with its time slot and progress) and next, for the hover preview and
/// the player. Renders nothing when the channel has no guide.
export function GuideSummary({
  guide,
  className,
  tone = "muted",
}: {
  guide: ChannelNowAndNext | undefined;
  className?: string;
  /// `overlay` for text over video.
  tone?: "muted" | "overlay";
}) {
  if (!guide?.now && !guide?.next) return null;
  const { now, next } = guide;
  const dim = tone === "overlay" ? "text-white/65" : "text-muted-foreground";
  const strong = tone === "overlay" ? "text-white/90" : "text-foreground";
  return (
    <span className={cn("flex min-w-0 flex-col gap-1.5 text-xs", className)}>
      {now && (
        <span className="flex min-w-0 flex-col gap-1">
          <span className={cn("block truncate", strong)} title={now.title}>
            <span className="font-medium text-primary">Now</span> {now.title}
          </span>
          <span className="flex items-center gap-2">
            <span className={cn("shrink-0 tabular-nums", dim)}>
              {formatGuideTime(now.startUtc)}–{formatGuideTime(now.stopUtc)}
            </span>
            <ProgressBar programme={now} className="max-w-40" />
          </span>
        </span>
      )}
      {next && (
        <span className={cn("block truncate", dim)} title={next.title}>
          <span className="tabular-nums">{formatGuideTime(next.startUtc)}</span> Next · {next.title}
        </span>
      )}
    </span>
  );
}
