//! "Cast" row on a detail page: photos (through the artwork cache) or
//! initials, with the character, plus a directed-by / created-by line.

import { Artwork } from "@/components/Artwork";
import type { CastEntry } from "@/types";

function initials(name: string): string {
  const words = name.split(/\s+/).filter(Boolean);
  return (words[0]?.[0] ?? "?") + (words.length > 1 ? (words[words.length - 1][0] ?? "") : "");
}

export function CastStrip({
  cast,
  people,
  peopleLabel,
}: {
  cast: CastEntry[];
  people: string[];
  peopleLabel: string;
}) {
  if (cast.length === 0 && people.length === 0) return null;
  return (
    <section className="flex flex-col gap-3">
      {people.length > 0 && (
        <p className="text-sm text-white/60">
          {peopleLabel} <span className="text-white/85">{people.join(", ")}</span>
        </p>
      )}
      {cast.length > 0 && (
        <>
          <h2 className="text-sm font-medium text-white/85">Cast</h2>
          <div className="-mx-1 flex gap-4 overflow-x-auto px-1 pb-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
            {cast.map((c) => (
              <div
                key={`${c.name}-${c.character ?? ""}`}
                className="flex w-20 shrink-0 flex-col items-center gap-1.5 text-center"
              >
                <div className="h-20 w-20 overflow-hidden rounded-full bg-white/10 ring-1 ring-white/10">
                  <Artwork
                    src={c.profileUrl}
                    alt=""
                    loading="lazy"
                    decoding="async"
                    className="h-full w-full object-cover"
                    fallback={
                      <span className="flex h-full w-full items-center justify-center text-sm font-medium text-white/60">
                        {initials(c.name).toUpperCase()}
                      </span>
                    }
                  />
                </div>
                <span className="line-clamp-2 text-xs text-white/85">{c.name}</span>
                {c.character && (
                  <span className="line-clamp-1 text-xs text-white/45">{c.character}</span>
                )}
              </div>
            ))}
          </div>
        </>
      )}
    </section>
  );
}
