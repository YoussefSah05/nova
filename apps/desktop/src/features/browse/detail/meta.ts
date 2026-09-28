//! Text for detail pages: the meta line and the series' primary action.

import { episodeCode } from "@/features/player/episodeMedia";
import type { ContinueWatchingItem, EpisodeView, SeriesEpisode, TitleFactsView } from "@/types";

/// `2h 28m`, `45m`.
export function formatRuntime(minutes: number): string {
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m}m`;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

/// `2011–2019`, `2019–` for a show still airing, or a single year.
export function yearSpan(
  facts: TitleFactsView | undefined,
  fallbackYear?: number | null
): string | null {
  const first = facts?.firstYear ?? fallbackYear ?? null;
  if (first === null) return null;
  const last = facts?.lastYear ?? null;
  const airing =
    !!facts?.nextEpisode ||
    facts?.status === "Returning Series" ||
    facts?.status === "In Production";
  if (airing) return `${first}–`;
  if (last !== null && last !== first) return `${first}–${last}`;
  return String(first);
}

export interface MetaLine {
  /** Shown as an outlined badge. */
  certification: string | null;
  parts: string[];
}

/// The line under a title: years, runtime or seasons, rating, genres.
export function metaLine(
  facts: TitleFactsView | undefined,
  opts: {
    kind: "movie" | "series";
    year?: number | null;
    seasonCount?: number;
    genre?: string | null;
  }
): MetaLine {
  const parts: string[] = [];
  const years = yearSpan(facts, opts.year);
  if (years) parts.push(years);
  if (opts.kind === "movie") {
    if (facts?.runtimeMin) parts.push(formatRuntime(facts.runtimeMin));
  } else {
    const seasons = facts?.seasons.filter((s) => s.number > 0).length || opts.seasonCount || 0;
    if (seasons > 0) parts.push(`${seasons} season${seasons === 1 ? "" : "s"}`);
  }
  if (facts?.rating) parts.push(`★ ${facts.rating.toFixed(1)}`);
  const genre = opts.genre ?? facts?.genre;
  if (genre) parts.push(genre);
  return { certification: facts?.certification ?? null, parts };
}

export interface SeriesAction {
  label: string;
  episodeId: number;
  /// The episode's number code (`S02E03`), for the player header.
  episodeCode: string;
}

type Numbered = Pick<SeriesEpisode, "season" | "episode" | "numbering">;

const code = (e: Numbered) => episodeCode(e) ?? "Episode";

/// The series' main button: resume the unfinished episode, move on to the
/// next one, or start from the first.
export function seriesPrimaryAction(
  resume: ContinueWatchingItem | null | undefined,
  first: (Numbered & { id: number }) | undefined
): SeriesAction | null {
  if (resume && resume.season !== null && resume.episode !== null && resume.numbering) {
    const ep = code({
      season: resume.season,
      episode: resume.episode,
      numbering: resume.numbering,
    });
    const target = { episodeId: resume.mediaId, episodeCode: ep };
    if (resume.upNext) return { label: `Next episode · ${ep}`, ...target };
    return { label: `Resume ${ep}`, ...target };
  }
  if (first) return { label: `Play ${code(first)}`, episodeId: first.id, episodeCode: code(first) };
  return null;
}

/// Fraction watched (0–1) of an episode row.
export function episodeProgress(ep: EpisodeView): number {
  if (ep.completed) return 1;
  if (!ep.durationSec || ep.durationSec <= 0) return 0;
  return Math.min(1, ep.positionSec / ep.durationSec);
}

/// `E03 — Title`, whichever parts are known.
export function episodeViewLabel(ep: EpisodeView, withSeason = false): string {
  const code = episodeCode(ep, withSeason);
  if (code && ep.title) return `${code} — ${ep.title}`;
  return code ?? ep.title ?? "Episode";
}
