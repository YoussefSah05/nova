//! Compact relative-time formatting for "last watched" hints on the profile
//! picker. Input is an epoch in **seconds** (Nova's `now_ts()` unit). Pure, so it
//! unit-tests without mocking the clock — pass `nowMs` to pin "now".

export function formatRelative(epochSeconds: number, nowMs: number = Date.now()): string {
  const diffSec = Math.floor(nowMs / 1000) - epochSeconds;
  if (diffSec < 0) return "just now";
  if (diffSec < 60) return "just now";

  const min = Math.floor(diffSec / 60);
  if (min < 60) return `${min}m ago`;

  const hours = Math.floor(min / 60);
  if (hours < 24) return `${hours}h ago`;

  const days = Math.floor(hours / 24);
  if (days === 1) return "yesterday";
  if (days < 7) return `${days} days ago`;

  const weeks = Math.floor(days / 7);
  if (weeks < 5) return `${weeks}w ago`;

  // Older than a month → an absolute, locale-formatted date.
  return new Date(epochSeconds * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}
