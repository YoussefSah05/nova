//! How a show is described on its cards: status, a meta line, progress, and
//! the week's air dates bucketed by the viewer's local day. Pure, so the
//! wording stays tied to the evidence the backend returns.

import type { SeriesSummary } from "@/types";

const UPCOMING = new Set(["In Production", "Planned", "Pilot"]);

/// Yesterday's local date: TMDB air dates are in the broadcaster's zone, so
/// a day of slack before a scheduled episode counts as past.
function yesterday(now: Date): string {
  return localDate(new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1));
}

/// A short status for a card, only from TMDB's status or schedule. A
/// scheduled episode counts while it hasn't aired; stored facts can be old.
export function statusLabel(
  s: SeriesSummary,
  now: Date = new Date()
): "Returning" | "In production" | "Ended" | "Cancelled" | null {
  const scheduled = !!s.nextEpisode && s.nextEpisode.airDate >= yesterday(now);
  if (s.status === "Returning Series" || (scheduled && !UPCOMING.has(s.status ?? "")))
    return "Returning";
  if (s.status && UPCOMING.has(s.status)) return "In production";
  if (s.status === "Ended") return "Ended";
  if (s.status === "Canceled") return "Cancelled";
  return null;
}

function years(s: SeriesSummary): string | null {
  if (!s.firstYear) return null;
  if (statusLabel(s) === "Returning") return `${s.firstYear}–`;
  if (s.lastYear && s.lastYear !== s.firstYear) return `${s.firstYear}–${s.lastYear}`;
  return String(s.firstYear);
}

/// `3 seasons · 2008–2013 · Drama, Crime`.
export function showMeta(s: SeriesSummary): string {
  const seasons =
    s.seasonCount > 0 ? `${s.seasonCount} ${s.seasonCount === 1 ? "season" : "seasons"}` : null;
  const genres = (s.genres?.length ? s.genres : s.category ? [s.category] : [])
    .slice(0, 2)
    .join(", ");
  return [seasons, years(s), genres || null].filter(Boolean).join(" · ");
}

/// `14 of 19 watched`, or null before the first finished episode.
export function watchedLine(s: SeriesSummary): string | null {
  const watched = Math.min(s.watchedCount ?? 0, s.episodeCount);
  if (watched <= 0 || s.episodeCount <= 0) return null;
  return `${watched} of ${s.episodeCount} watched`;
}

/// One tick per episode (watched first) while they stay legible; past
/// `max` episodes a continuous bar reads better, so null.
export function progressTicks(s: SeriesSummary, max = 60): boolean[] | null {
  if (s.episodeCount <= 0 || s.episodeCount > max) return null;
  const watched = Math.min(s.watchedCount ?? 0, s.episodeCount);
  return Array.from({ length: s.episodeCount }, (_, i) => i < watched);
}

/// `S3 · E1`.
export function episodeCode(season: number, episode: number): string {
  return `S${season} · E${episode}`;
}

/// A local calendar date as `YYYY-MM-DD` (TMDB air dates are dates, not times).
export function localDate(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export interface WeekDay {
  date: string;
  /// `Today`, `Tomorrow`, or the weekday (`Thu`).
  label: string;
  dayOfMonth: number;
  today: boolean;
  shows: SeriesSummary[];
}

/// Seven days from `today`, each with the shows whose next episode airs then.
export function weekOf(today: Date, shows: SeriesSummary[]): WeekDay[] {
  return Array.from({ length: 7 }, (_, i) => {
    const d = new Date(today.getFullYear(), today.getMonth(), today.getDate() + i);
    const date = localDate(d);
    return {
      date,
      label:
        i === 0
          ? "Today"
          : i === 1
            ? "Tomorrow"
            : d.toLocaleDateString(undefined, { weekday: "short" }),
      dayOfMonth: d.getDate(),
      today: i === 0,
      shows: shows.filter((s) => s.nextEpisode?.airDate === date),
    };
  });
}
