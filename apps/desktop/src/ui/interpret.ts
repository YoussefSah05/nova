//! Reading a typed search as facets, without a model: decades ("90s",
//! "1990s"), years ("2001"), the genres the library has ("comedies",
//! "sci-fi"), and what to look for ("movies", "shows", "channels"). What's
//! left is searched as text. Each facet can be dismissed, which searches its
//! words as text again. Interpreted facets share the model hand-picked ones
//! use, so a future on-device intent parser can replace this rule set.

import type { Facet } from "./facets";

export type SearchType = "movies" | "series" | "live";

export interface Interpretation {
  /// The words left to search as text.
  text: string;
  /// What was understood, in reading order.
  facets: Facet[];
}

/// A facet's identity (for dismissing it).
export function facetId(f: Pick<Facet, "kind" | "value">): string {
  return `${f.kind}:${f.value}`;
}

const TYPE_WORDS: [RegExp, SearchType, string][] = [
  [/\b(movies|movie|films|film)\b/, "movies", "Movies"],
  [/\b(tv shows|tv show|shows|show|series)\b/, "series", "Shows"],
  [/\b(channels|channel|live tv|live)\b/, "live", "Channels"],
];

/// Words that only join understood parts ("comedies from the 90s").
const JOINERS = new Set(["from", "the", "in", "of", "and", "with", "a", "an", "about"]);

const GENRE_ALIASES: Record<string, string> = {
  "sci-fi": "science fiction",
  scifi: "science fiction",
  "rom-com": "romance",
  romcom: "romance",
  cartoons: "animation",
  docs: "documentary",
};

function escape(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/// Word forms of a genre: "Comedy" → comedy, comedies; "Drama" → drama, dramas.
function genreForms(genre: string): string[] {
  const g = genre.toLowerCase();
  const forms = [g, `${g}s`];
  if (g.endsWith("y")) forms.push(`${g.slice(0, -1)}ies`);
  return forms;
}

export function interpret(
  query: string,
  genres: string[],
  dismissed: ReadonlySet<string> = new Set()
): Interpretation {
  // "'90s" and "90's" read as "90s".
  let rest = ` ${query
    .toLowerCase()
    .replace(/'(\d0s)\b/g, "$1")
    .replace(/(\d0)'s\b/g, "$1s")} `;
  const found: { at: number; facet: Facet }[] = [];
  const take = (re: RegExp, facet: (m: RegExpExecArray) => Facet | null) => {
    const m = re.exec(rest);
    if (!m) return;
    const f = facet(m);
    if (!f || dismissed.has(facetId(f)) || found.some((x) => facetId(x.facet) === facetId(f)))
      return;
    // The list API accepts one genre. Extra genre words stay searchable,
    // rather than displaying chips whose restrictions would be discarded.
    if (f.kind === "genre" && found.some((x) => x.facet.kind === "genre")) return;
    found.push({ at: m.index, facet: f });
    rest = rest.slice(0, m.index) + " ".repeat(m[0].length) + rest.slice(m.index + m[0].length);
  };

  // Decades before years: "1990s" is a decade, not the year 1990.
  take(/\b(?:(19|20)(\d)0|'?(\d)0)'?s\b/, (m) => {
    const digit = Number(m[2] ?? m[3]);
    const decade = m[1]
      ? Number(`${m[1]}${digit}0`)
      : digit <= 2
        ? 2000 + digit * 10
        : 1900 + digit * 10;
    return { kind: "decade", value: String(decade), label: `${decade}s`, origin: "intent" };
  });
  for (const [re, type, label] of TYPE_WORDS)
    take(re, () => ({ kind: "type", value: type, label, origin: "intent" }));

  const byLower = new Map(genres.map((g) => [g.toLowerCase(), g]));
  for (const [alias, target] of Object.entries(GENRE_ALIASES)) {
    const genre = byLower.get(target);
    if (genre)
      take(new RegExp(`\\b${escape(alias)}\\b`), () => ({
        kind: "genre",
        value: genre,
        origin: "intent",
      }));
  }
  // Longer genres first: "science fiction" before "fiction".
  for (const genre of [...genres].sort((a, b) => b.length - a.length)) {
    const forms = genreForms(genre).map(escape).join("|");
    take(new RegExp(`\\b(?:${forms})\\b`), () => ({
      kind: "genre",
      value: genre,
      origin: "intent",
    }));
  }

  // A year is a filter only in context ("from 2001", "sci-fi 2001"); on its
  // own it is more likely part of a title ("Blade Runner 2049", "1917").
  const maxYear = new Date().getFullYear() + 1;
  const yearIn = /\b(?:(?:from|in|of)\s+)?(19[2-9]\d|20\d\d)\b/.exec(rest);
  if (
    yearIn &&
    !found.some((x) => x.facet.kind === "decade") &&
    Number(yearIn[1]) <= maxYear &&
    (found.length > 0 || yearIn[0] !== yearIn[1])
  )
    take(new RegExp(`\\b${escape(yearIn[0])}\\b`), () => ({
      kind: "year",
      value: yearIn[1],
      label: yearIn[1],
      origin: "intent",
    }));

  const facets = found.sort((a, b) => a.at - b.at).map((x) => x.facet);
  const words = rest.split(/\s+/).filter(Boolean);
  const text = (facets.length > 0 ? words.filter((w) => !JOINERS.has(w)) : words).join(" ");
  return { text, facets };
}
