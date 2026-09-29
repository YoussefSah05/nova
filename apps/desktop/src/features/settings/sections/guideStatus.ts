//! Plain-language summary of a source's guide feeds (Settings → Sources).

import { formatRelative } from "@/lib/formatRelative";
import type { EpgFeedStatus } from "@/types";

/// One line summarising a source's guide feeds.
export function describeGuide(feeds: EpgFeedStatus[], nowMs: number = Date.now()): string {
  if (feeds.length === 0) return "No guide feed yet — add an XMLTV URL below.";
  const programmes = feeds.reduce((n, f) => n + f.programmeCount, 0);
  const refreshed = feeds.map((f) => f.lastRefreshUtc).filter((t): t is number => t != null);
  const failed = feeds.filter((f) => f.lastError);
  const parts: string[] = [];
  parts.push(
    programmes > 0
      ? `${programmes.toLocaleString()} programmes from ${feeds.length} ${feeds.length === 1 ? "feed" : "feeds"}`
      : "No programmes yet"
  );
  if (refreshed.length > 0) parts.push(`updated ${formatRelative(Math.max(...refreshed), nowMs)}`);
  if (failed.length > 0) {
    const which =
      failed.length === feeds.length
        ? "Last refresh failed"
        : `${failed.length} of ${feeds.length} feeds failed`;
    parts.push(`${which}: ${failed[0].lastError}`);
  }
  return parts.join(" · ");
}
