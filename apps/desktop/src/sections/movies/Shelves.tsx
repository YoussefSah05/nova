//! Ways into the film library other than a grid: TMDB collections (fanned
//! poster stacks), decades (the year as type over a strip of posters) and
//! genres (sized by how many films they hold). Each applies its facet.

import { memo } from "react";

import type { DecadeCount, FacetCount, LibraryCollection } from "@/types";
import { Artwork } from "@/components/Artwork";
import { cn } from "@/lib/utils";

function Shelf({
  id,
  title,
  note,
  children,
}: {
  id: string;
  title: string;
  note?: string;
  children: React.ReactNode;
}) {
  return (
    <section aria-labelledby={id} className="nova-cv flex flex-col gap-[18px]">
      <div className="flex items-baseline gap-3.5">
        <h2 id={id} className="font-display text-[28px] font-medium">
          {title}
        </h2>
        {note && <span className="text-sm text-white/60">{note}</span>}
      </div>
      {children}
    </section>
  );
}

/// Placeholder while a shelf that is almost always present loads, so the page
/// below it does not jump when it arrives.
function ShelfSkeleton({
  id,
  title,
  children,
}: {
  id: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <Shelf id={id} title={title}>
      <div aria-hidden className="pointer-events-none">
        {children}
      </div>
    </Shelf>
  );
}

export const CollectionsShelf = memo(function CollectionsShelf({
  collections,
  onPick,
}: {
  collections: LibraryCollection[];
  onPick: (c: LibraryCollection) => void;
}) {
  if (collections.length === 0) return null;
  return (
    <Shelf id="movies-collections" title="Collections" note="Sagas and film cycles you own">
      <ul className="grid grid-cols-5 gap-6">
        {collections.slice(0, 10).map((c) => (
          <li key={c.id}>
            <button
              type="button"
              onClick={() => onPick(c)}
              aria-label={`${c.name}, ${c.count} films`}
              className="group flex w-full flex-col items-center gap-3.5 rounded-xl focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <span className="relative block h-[230px] w-full">
                {[0, 1, 2].map((i) => {
                  const src = c.posterUrls[i === 1 ? 0 : i === 0 ? 1 : 2] ?? c.posterUrls[0];
                  const front = i === 1;
                  return (
                    <span
                      key={i}
                      aria-hidden
                      className={cn(
                        "absolute left-1/2 overflow-hidden rounded-lg bg-[#1a1a1e] transition-transform duration-[400ms] ease-[cubic-bezier(0.16,1,0.3,1)]",
                        front
                          ? "top-2.5 z-10 -ml-[63px] h-[189px] w-[126px] shadow-[0_12px_30px_rgba(0,0,0,0.5),inset_0_0_0_1px_rgba(255,255,255,0.1)]"
                          : "top-[18px] -ml-[59px] h-[177px] w-[118px] shadow-[0_8px_24px_rgba(0,0,0,0.45)]",
                        i === 0 &&
                          "-translate-x-2 -rotate-[4deg] group-hover:-translate-x-3.5 group-hover:-rotate-[7deg]",
                        i === 2 &&
                          "translate-x-2 rotate-[4deg] group-hover:translate-x-3.5 group-hover:rotate-[7deg]"
                      )}
                    >
                      <Artwork
                        src={src}
                        alt=""
                        loading="lazy"
                        decoding="async"
                        className="h-full w-full object-cover"
                      />
                    </span>
                  );
                })}
              </span>
              <span className="flex flex-col items-center gap-1 text-center">
                <span className="text-base font-medium">{c.name}</span>
                <span className="text-xs text-white/60">{c.count} films in your library</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </Shelf>
  );
});

export const DecadesShelf = memo(function DecadesShelf({
  decades,
  loading = false,
  active,
  onPick,
}: {
  /** Undefined until loaded (or if the read failed). */
  decades: DecadeCount[] | undefined;
  /** True only while the first read is in flight; a failed read shows nothing. */
  loading?: boolean;
  active: number | null;
  onPick: (decade: number) => void;
}) {
  if (decades === undefined && loading)
    return (
      <ShelfSkeleton id="movies-decades" title="By decade">
        <div className="grid grid-cols-6 gap-4">
          {Array.from({ length: 6 }, (_, i) => (
            <div key={i} className="nova-shimmer h-[150px] rounded-[14px] bg-[#111113]" />
          ))}
        </div>
      </ShelfSkeleton>
    );
  if (!decades || decades.length === 0) return null;
  return (
    <Shelf id="movies-decades" title="By decade" note="Choose one to see its films">
      <ul className="grid grid-cols-6 gap-4">
        {decades.slice(0, 6).map((d) => {
          const on = active === d.decade;
          return (
            <li key={d.decade}>
              <button
                type="button"
                onClick={() => onPick(d.decade)}
                aria-pressed={on}
                className={cn(
                  "flex h-[150px] w-full flex-col gap-1 rounded-[14px] bg-[#111113] p-4 text-left transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  on
                    ? "border border-primary/60 shadow-[0_0_0_1px_rgba(224,163,62,0.35),0_6px_24px_rgba(224,163,62,0.18)]"
                    : "border border-white/[0.06] hover:border-white/20 hover:bg-[#16161a]"
                )}
              >
                <span className="font-display text-[36px] font-medium leading-none tracking-[-0.02em]">
                  {d.decade}s
                </span>
                <span className="text-xs text-white/60">
                  {d.count.toLocaleString()} {d.count === 1 ? "film" : "films"}
                </span>
                <span aria-hidden className="mt-auto flex gap-1">
                  {d.posterUrls.map((p, i) => (
                    <span
                      key={i}
                      className="h-[39px] w-[26px] overflow-hidden rounded-[3px] bg-white/[0.06]"
                    >
                      <Artwork
                        src={p}
                        alt=""
                        loading="lazy"
                        decoding="async"
                        className="h-full w-full object-cover"
                      />
                    </span>
                  ))}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </Shelf>
  );
});

/// Type size by share of the library: the big genres read first.
function sizeFor(count: number, max: number): string {
  const share = count / Math.max(1, max);
  if (share > 0.66) return "text-[36px]";
  if (share > 0.4) return "text-[28px]";
  if (share > 0.2) return "text-[22px]";
  return "text-[18px]";
}

export const GenresShelf = memo(function GenresShelf({
  genres,
  loading = false,
  active,
  onPick,
}: {
  /** Undefined until loaded (or if the read failed). */
  genres: FacetCount[] | undefined;
  /** True only while the first read is in flight; a failed read shows nothing. */
  loading?: boolean;
  active: string | null;
  onPick: (genre: string) => void;
}) {
  if (genres === undefined && loading)
    return (
      <ShelfSkeleton id="movies-genres" title="By genre">
        <div className="flex flex-wrap gap-x-[26px] gap-y-3">
          {[96, 72, 120, 64, 88, 110, 76, 92].map((w, i) => (
            <div
              key={i}
              className="nova-shimmer h-7 rounded-md bg-white/[0.05]"
              style={{ width: w }}
            />
          ))}
        </div>
      </ShelfSkeleton>
    );
  if (!genres || genres.length === 0) return null;
  const max = genres[0]?.count ?? 1;
  return (
    <Shelf id="movies-genres" title="By genre">
      <ul className="flex flex-wrap items-baseline gap-x-[26px] gap-y-1.5">
        {genres.slice(0, 16).map((g) => {
          const on = active === g.name;
          return (
            <li key={g.name}>
              <button
                type="button"
                onClick={() => onPick(g.name)}
                aria-pressed={on}
                className={cn(
                  "rounded-md font-display font-medium tracking-[-0.01em] transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  sizeFor(g.count, max),
                  on ? "text-primary" : "text-white/85 hover:text-[var(--color-brand-hover)]"
                )}
              >
                {g.name}
                <sup className="ml-1 font-sans text-xs font-normal text-white/50">
                  {g.count.toLocaleString()}
                </sup>
              </button>
            </li>
          );
        })}
      </ul>
    </Shelf>
  );
});
