//! Search: one field over the whole library. What Nova understood ("90s",
//! "comedies", "shows") shows as chips that can be undone; results come in
//! groups — films, shows, channels — and Moments, the lines of dialogue in
//! transcribed titles, which play from where they were said. When
//! meaning-based search is off, a note says exact search still works.

import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MessageSquareQuoteIcon, PlayIcon, SearchIcon } from "lucide-react";

import type { MomentHit } from "@/types";
import { Artwork } from "@/components/Artwork";
import { PosterCard } from "@/features/browse/PosterCard";
import { PosterFallback } from "@/features/browse/PosterFallback";
import { ShowCard } from "@/features/browse/ShowCard";
import { useMovieGenres, useSeriesGenres, type BrowseItem } from "@/features/browse/queries";
import {
  channelsApply,
  momentsApply,
  planFrom,
  seriesApply,
  shows,
  useSearchChannels,
  useSearchMoments,
  useSearchMovies,
  useSearchSeries,
  useSemanticSearchAvailable,
} from "@/features/search/data";
import { cn } from "@/lib/utils";
import { CapabilityNotice } from "@/ui/CapabilityNotice";
import type { Facet } from "@/ui/facets";
import { facetId, interpret } from "@/ui/interpret";
import { InterpretationChips } from "@/ui/InterpretationChips";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";
import { useShell } from "@/app/shell";

/// Typing settles for this long before searching.
const DEBOUNCE_MS = 250;

function useDebounced<T>(value: T, ms: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return debounced;
}

export function SearchPage({ initialQuery }: { initialQuery: string }) {
  const { profile, activeSourceId } = useSession();
  const shell = useShell();
  const [input, setInput] = useState(initialQuery);
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set());
  const query = useDebounced(input.trim(), DEBOUNCE_MS);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => inputRef.current?.focus(), []);
  // A new search forgets what was undone in the last one.
  useEffect(() => setDismissed(new Set()), [query]);

  const movieGenres = useMovieGenres(profile.id, activeSourceId).data;
  const seriesGenres = useSeriesGenres(profile.id, activeSourceId).data;
  const genres = useMemo(
    () => [...new Set([...(movieGenres ?? []), ...(seriesGenres ?? [])].map((g) => g.name))],
    [movieGenres, seriesGenres]
  );
  const understood = useMemo(() => interpret(query, genres, dismissed), [query, genres, dismissed]);
  const plan = useMemo(() => planFrom(understood.text, understood.facets), [understood]);
  const dismiss = useCallback((f: Facet) => setDismissed((d) => new Set([...d, facetId(f)])), []);

  const movies = useSearchMovies(profile.id, activeSourceId, plan);
  const series = useSearchSeries(profile.id, activeSourceId, plan);
  const channels = useSearchChannels(profile.id, activeSourceId, plan);
  const moments = useSearchMoments(profile.id, activeSourceId, plan);
  const semantic = useSemanticSearchAvailable().data;

  const active = query.length > 0;
  const groups = [
    shows(plan, "movies") ? movies : null,
    seriesApply(plan) ? series : null,
    channelsApply(plan) ? channels : null,
    momentsApply(plan) ? moments : null,
  ].filter((g) => g !== null);
  const settled = groups.every((g) => !g.isFetching);
  const nothing =
    active &&
    settled &&
    (movies.data?.length ?? 0) +
      (series.data?.length ?? 0) +
      (channels.data?.length ?? 0) +
      (moments.data?.length ?? 0) ===
      0;

  return (
    <div className="flex flex-col gap-8 px-[88px] pb-24 pt-[108px]">
      <div className="flex flex-col gap-4">
        <label className="flex items-center gap-4 border-b border-white/[0.12] pb-3 focus-within:border-white/35">
          <SearchIcon className="h-7 w-7 shrink-0 text-white/50" strokeWidth={1.5} aria-hidden />
          <input
            ref={inputRef}
            type="search"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Films, shows, channels, or a line you remember"
            aria-label="Search the library"
            className="min-w-0 flex-1 bg-transparent font-display text-[36px] font-medium tracking-[-0.015em] text-white outline-none placeholder:text-white/30"
          />
        </label>
        <InterpretationChips facets={understood.facets} onDismiss={dismiss} />
        {semantic === false && (
          <CapabilityNotice
            title="Meaning-based search is off"
            detail="exact words still find titles, channels and lines. Install the search model to find things by what they're about."
            action={
              <button
                type="button"
                onClick={shell.openSettings}
                className="shrink-0 rounded-lg px-2.5 py-1 text-sm text-white/85 hover:bg-white/[0.08] focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                Settings
              </button>
            }
          />
        )}
      </div>

      {!active ? (
        <p className="text-base text-white/55">
          Try “90s crime films”, “sci-fi shows”, a channel, or words someone said.
        </p>
      ) : nothing ? (
        <p className="text-base text-white/60">Nothing in this playlist matches “{query}”.</p>
      ) : (
        <div className="flex flex-col gap-12">
          {shows(plan, "movies") && (
            <ResultGroup
              id="search-movies"
              title="Films"
              items={movies.data}
              loading={movies.isFetching && !movies.data}
              layout="poster"
            />
          )}
          {seriesApply(plan) && (
            <ResultGroup
              id="search-series"
              title="Shows"
              items={series.data}
              loading={series.isFetching && !series.data}
              layout="landscape"
            />
          )}
          {channelsApply(plan) && (
            <ResultGroup
              id="search-live"
              title="Channels"
              items={channels.data}
              loading={channels.isFetching && !channels.data}
              layout="channel"
            />
          )}
          {momentsApply(plan) && (
            <Moments
              hits={moments.data}
              loading={moments.isFetching && !moments.data}
              query={plan.text}
            />
          )}
        </div>
      )}
    </div>
  );
}

const ResultGroup = memo(function ResultGroup({
  id,
  title,
  items,
  loading,
  layout,
}: {
  id: string;
  title: string;
  items: BrowseItem[] | undefined;
  loading: boolean;
  layout: "poster" | "landscape" | "channel";
}) {
  const activate = useActivate();
  const { toggleFavorite, toggleWatchlist } = useLibraryActions();
  if (!loading && (!items || items.length === 0)) return null;
  const Card = layout === "landscape" ? ShowCard : PosterCard;
  return (
    <section aria-labelledby={id} className="flex flex-col gap-4">
      <h2 id={id} className="font-display text-[28px] font-medium">
        {title}
        {items && items.length > 0 && (
          <span className="ml-3 font-sans text-sm font-normal text-white/50">{items.length}</span>
        )}
      </h2>
      {loading ? (
        <div className="nova-shimmer h-40 rounded-xl bg-white/[0.03]" />
      ) : (
        <ul
          className={cn(
            "grid gap-4",
            layout === "poster" && "grid-cols-[repeat(auto-fill,minmax(144px,1fr))]",
            layout === "landscape" && "grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-5",
            layout === "channel" && "grid-cols-[repeat(auto-fill,minmax(176px,1fr))]"
          )}
        >
          {items!.map((item) => (
            <li key={item.key}>
              <Card
                item={item}
                onActivate={activate}
                onToggleFavorite={toggleFavorite}
                onToggleWatchlist={toggleWatchlist}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
});

/// `1:02:03` or `12:04`.
function clock(sec: number): string {
  const s = Math.max(0, Math.floor(sec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

function where(hit: MomentHit): string {
  const code =
    hit.mediaKind === "episode" && hit.numbered && hit.season !== null && hit.episode !== null
      ? `S${hit.season} · E${hit.episode}`
      : hit.mediaKind === "episode"
        ? (hit.episodeTitle ?? "Episode")
        : null;
  return [hit.title, code, clock(hit.startSec)].filter(Boolean).join(" · ");
}

const Moments = memo(function Moments({
  hits,
  loading,
  query,
}: {
  hits: MomentHit[] | undefined;
  loading: boolean;
  query: string;
}) {
  const { playMedia } = usePlayback();
  return (
    <section aria-labelledby="search-moments" className="flex flex-col gap-4">
      <div className="flex items-baseline gap-3.5">
        <h2 id="search-moments" className="font-display text-[28px] font-medium">
          Moments
        </h2>
        <span className="text-sm text-white/60">Where it was said, in transcribed titles</span>
      </div>
      {loading ? (
        <div className="nova-shimmer h-24 rounded-xl bg-white/[0.03]" />
      ) : !hits || hits.length === 0 ? (
        <p className="flex items-center gap-2.5 text-sm text-white/55">
          <MessageSquareQuoteIcon className="h-4 w-4 shrink-0" strokeWidth={1.5} aria-hidden />
          No line with “{query}” in the titles transcribed so far. Transcribe a title from the
          player to search what's said in it.
        </p>
      ) : (
        <ol className="flex flex-col gap-1">
          {hits.map((hit) => (
            <li key={hit.segmentId}>
              <button
                type="button"
                onClick={() =>
                  playMedia({
                    kind: hit.mediaKind,
                    id: hit.mediaId,
                    title: hit.title,
                    subtitle: where(hit),
                    posterUrl: hit.posterUrl,
                    startSec: hit.startSec,
                  })
                }
                aria-label={`Play ${where(hit)}: “${hit.text}”`}
                className="group flex w-full items-center gap-4 rounded-xl p-2 text-left transition-colors hover:bg-white/[0.05] focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="relative h-[72px] w-12 shrink-0 overflow-hidden rounded-md bg-white/[0.06]">
                  <Artwork
                    src={hit.posterUrl}
                    alt=""
                    loading="lazy"
                    decoding="async"
                    className="h-full w-full object-cover"
                    fallback={<PosterFallback title={hit.title} />}
                  />
                </span>
                <span className="flex min-w-0 flex-1 flex-col gap-1">
                  <span className="line-clamp-2 text-base text-white/90">“{hit.text}”</span>
                  <span className="text-xs tabular-nums text-white/55">
                    {where(hit)}
                    {hit.kind === "translation" && " · translated line"}
                  </span>
                </span>
                <PlayIcon
                  className="h-5 w-5 shrink-0 text-white/40 transition-colors group-hover:text-white"
                  strokeWidth={1.5}
                  aria-hidden
                />
              </button>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
});
