//! A film's page: full-bleed text-free backdrop with the TMDB logo, the facts
//! that matter (with where the rating comes from), resume, the film's sources
//! with Nova's pick explained, cast and credits, its saga, and more like it.

import { memo, useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, Headphones, Languages, Play } from "lucide-react";

import type { CanonicalMediaDetail, CanonicalMediaSummary, TitleFactsView } from "@/types";
import { Artwork } from "@/components/Artwork";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { AmbientGlow, useAmbientColor } from "@/lib/ambient";
import { useArtworkSrc } from "@/lib/artwork";
import { useScreenActive } from "@/lib/screenActive";
import { cn } from "@/lib/utils";
import { FavoriteButton } from "@/features/browse/FavoriteButton";
import { WatchlistButton } from "@/features/browse/WatchlistButton";
import { PosterCard } from "@/features/browse/PosterCard";
import { PosterFallback } from "@/features/browse/PosterFallback";
import { TitleLogo } from "@/features/browse/TitleLogo";
import { CastStrip } from "@/features/browse/detail/CastStrip";
import { TrailerButton } from "@/features/browse/detail/TrailerButton";
import {
  canonicalMovieQuery,
  useRelatedTitles,
  useTitleFacts,
} from "@/features/browse/detail/data";
import { formatRuntime } from "@/features/browse/detail/meta";
import { movieToItem, type BrowseItem } from "@/features/browse/queries";
import { titleArtSubject, useTitleArt } from "@/features/browse/titleArt";
import { useProfileSettings } from "@/features/settings/useProfileSettings";
import { WhyChip } from "@/ui/WhyChip";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";
import { VersionsPanel } from "./movie/VersionsPanel";

const AUDIO_LANGS: { value: string | null; label: string }[] = [
  { value: null, label: "Original" },
  { value: "en", label: "English" },
  { value: "fr", label: "French" },
  { value: "ar", label: "Arabic" },
  { value: "es", label: "Spanish" },
  { value: "de", label: "German" },
  { value: "it", label: "Italian" },
  { value: "pt", label: "Portuguese" },
];

/// `1:34:10` / `34:10`.
function clock(totalSec: number): string {
  const s = Math.max(0, Math.floor(totalSec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

function formatVotes(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(n >= 10000 ? 0 : 1)}k votes` : `${n} votes`;
}

function formatRelease(date: string): string {
  const d = new Date(`${date}T00:00:00`);
  return /^\d{4}-\d{2}-\d{2}$/.test(date) && !Number.isNaN(d.getTime())
    ? d.toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" })
    : date;
}

export function MoviePage({ item }: { item: BrowseItem }) {
  const { profile, activeSourceId } = useSession();
  const qc = useQueryClient();
  const { playItem, playVariant } = usePlayback();
  const activate = useActivate();
  const { toggleFavorite, toggleWatchlist } = useLibraryActions();
  const { settings, setSettings } = useProfileSettings(profile.id);
  const { data: detail } = useQuery({
    subscribed: useScreenActive(),
    ...canonicalMovieQuery(profile.id, item.entityId),
    enabled: !!item.entityId,
    staleTime: 0,
  });
  const movie: CanonicalMediaSummary | undefined = detail ?? item.movie;
  const subject = titleArtSubject(item);
  const art = useTitleArt(profile.id, item);
  const { data: facts } = useTitleFacts(profile.id, subject);
  const { data: related } = useRelatedTitles(profile.id, activeSourceId, subject);

  const setAudio = (value: string | null) => {
    setSettings({ preferredAudioLang: value });
    // Nova's pick depends on the audio language.
    void qc.invalidateQueries({
      queryKey: canonicalMovieQuery(profile.id, item.entityId).queryKey,
    });
  };

  return (
    <article className="relative min-h-screen bg-background pb-24">
      <MovieHero
        item={item}
        movie={movie}
        detail={detail}
        facts={facts}
        logoUrl={art?.logoUrl ?? null}
        backdropUrl={art?.backdropUrl ?? movie?.backdropUrl ?? item.posterUrl}
        audio={settings.preferredAudioLang ?? null}
        onAudio={setAudio}
        onPlay={() => void playItem(item)}
        onToggleFavorite={(next) => void toggleFavorite(item, next)}
        onToggleWatchlist={(next) => void toggleWatchlist(item, next)}
        versions={
          detail ? (
            <VersionsPanel
              variants={detail.variants}
              preferredLang={settings.preferredAudioLang ?? null}
              onPlay={(variantId) => void playVariant(item, variantId)}
            />
          ) : null
        }
      />
      <div className="relative z-10 flex flex-col gap-12 px-[88px]">
        <Credits facts={facts} detail={detail} movie={movie} />
        {related && related.collection.length > 1 && (
          <CollectionStrip
            name={related.collectionName ?? "The collection"}
            films={related.collection}
            currentId={item.entityId ?? null}
            onOpen={(m) => activate(movieToItem(m))}
          />
        )}
        {related && related.movies.length > 0 && (
          <section aria-labelledby="movie-more" className="flex flex-col gap-4">
            <div className="flex items-center gap-3.5">
              <h2 id="movie-more" className="font-display text-[28px] font-medium">
                More like this
              </h2>
              <WhyChip reason="TMDB recommends these, and they are in your library" />
            </div>
            <div className="-mx-1 flex snap-x gap-4 overflow-x-auto px-1 pb-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
              {related.movies
                .slice(0, 16)
                .map(movieToItem)
                .map((m) => (
                  <PosterCard
                    key={m.key}
                    item={m}
                    onActivate={activate}
                    onToggleFavorite={toggleFavorite}
                    onToggleWatchlist={toggleWatchlist}
                  />
                ))}
            </div>
          </section>
        )}
      </div>
    </article>
  );
}

function MovieHero({
  item,
  movie,
  detail,
  facts,
  logoUrl,
  backdropUrl,
  audio,
  onAudio,
  onPlay,
  onToggleFavorite,
  onToggleWatchlist,
  versions,
}: {
  item: BrowseItem;
  movie: CanonicalMediaSummary | undefined;
  detail: CanonicalMediaDetail | undefined;
  facts: TitleFactsView | undefined;
  logoUrl: string | null;
  backdropUrl: string | null | undefined;
  audio: string | null;
  onAudio: (value: string | null) => void;
  onPlay: () => void;
  onToggleFavorite: (next: boolean) => void;
  onToggleWatchlist: (next: boolean) => void;
  versions: React.ReactNode;
}) {
  const [src, onError] = useArtworkSrc(backdropUrl);
  const ambient = useAmbientColor(src);
  const resume = detail?.resume && !detail.resume.completed ? detail.resume : null;
  const pct =
    resume?.durationSec && resume.durationSec > 0
      ? Math.min(100, Math.round((resume.positionSec / resume.durationSec) * 100))
      : 0;
  const rating = facts?.rating ?? movie?.rating ?? null;
  const ratingSource = facts?.ratingSource ?? movie?.ratingSource ?? null;
  const runtime = facts?.runtimeMin ?? movie?.runtimeMin ?? null;
  const genres = movie?.genres?.length ? movie.genres : movie?.category ? [movie.category] : [];
  const audioLabel = AUDIO_LANGS.find((l) => l.value === audio)?.label ?? "Original";
  const overview = movie?.overview ?? facts?.plot ?? null;

  return (
    <header className="relative min-h-[min(86vh,860px)] w-full overflow-hidden">
      <div aria-hidden className="absolute inset-0">
        {src ? (
          <img
            src={src}
            alt=""
            decoding="async"
            fetchPriority="high"
            onError={onError}
            className="h-full w-full object-cover object-top"
          />
        ) : (
          <div className="h-full w-full bg-gradient-to-br from-primary/15 via-[#111113] to-[#0a0a0c]" />
        )}
        <AmbientGlow color={ambient} className="h-[80%] mix-blend-screen" />
        <div className="absolute inset-0 bg-gradient-to-r from-[#0a0a0c]/[0.92] via-[#0a0a0c]/55 to-[#0a0a0c]/10" />
        <div className="absolute inset-x-0 bottom-0 h-[50%] bg-gradient-to-t from-[#0a0a0c] to-transparent" />
      </div>

      <div className="relative z-10 flex items-start justify-between gap-12 px-[88px] pb-16 pt-[180px]">
        <div className="flex min-w-0 max-w-[640px] flex-col gap-[18px]">
          <TitleLogo
            title={item.title}
            logoUrl={logoUrl}
            as="h1"
            className="font-display text-[36px] font-medium leading-tight tracking-[-0.015em]"
            logoClassName="max-h-36 max-w-[min(32rem,95%)]"
          />
          <p className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-sm text-white/80">
            {movie?.year && <span>{movie.year}</span>}
            {runtime && (
              <>
                <span aria-hidden className="text-white/30">
                  ·
                </span>
                <span>{formatRuntime(runtime)}</span>
              </>
            )}
            {facts?.certification && (
              <span className="rounded border border-white/35 px-1.5 text-xs leading-5">
                {facts.certification}
              </span>
            )}
            {rating !== null && (
              <>
                <span aria-hidden className="text-white/30">
                  ·
                </span>
                <span>
                  ★ {rating.toFixed(1)}{" "}
                  <span className="text-white/55">
                    {ratingSource === "tmdb" ? "TMDB" : "Provider"}
                    {detail?.voteCount ? ` · ${formatVotes(detail.voteCount)}` : ""}
                  </span>
                </span>
              </>
            )}
          </p>
          {genres.length > 0 && (
            <ul className="flex flex-wrap gap-2">
              {genres.slice(0, 4).map((g) => (
                <li
                  key={g}
                  className="rounded-full bg-white/[0.08] px-2.5 py-1 text-xs text-white/85"
                >
                  {g}
                </li>
              ))}
            </ul>
          )}
          {facts?.tagline && (
            <p className="font-display text-[22px] text-white/90">{facts.tagline}</p>
          )}
          {overview && (
            <p className="line-clamp-4 max-w-[560px] text-base leading-relaxed text-white/75">
              {overview}
            </p>
          )}
          {resume && pct > 0 && (
            <div className="flex w-[360px] flex-col gap-2">
              <div className="flex justify-between text-xs text-white/70">
                <span>Stopped at {clock(resume.positionSec)}</span>
                {resume.durationSec && (
                  <span>
                    {Math.max(1, Math.round((resume.durationSec - resume.positionSec) / 60))} min
                    left
                  </span>
                )}
              </div>
              <div className="h-1 overflow-hidden rounded-full bg-white/[0.18]">
                <div className="h-full rounded-full bg-primary" style={{ width: `${pct}%` }} />
              </div>
            </div>
          )}
          <div className="mt-1 flex flex-wrap items-center gap-2.5">
            <Button size="lg" onClick={onPlay} className="gap-2.5">
              <Play className="h-4 w-4 fill-current" strokeWidth={1.5} aria-hidden />
              {resume ? `Resume · ${clock(resume.positionSec)}` : "Play"}
            </Button>
            <WatchlistButton
              inWatchlist={item.isInWatchlist}
              onToggle={onToggleWatchlist}
              label={item.title}
            />
            <FavoriteButton
              isFavorite={item.isFavorite}
              onToggle={onToggleFavorite}
              label={item.title}
            />
            <TrailerButton url={facts?.trailerUrl} />
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="secondary" className="gap-2">
                  <Headphones className="h-4 w-4" strokeWidth={1.5} aria-hidden /> Audio:{" "}
                  {audioLabel}
                  <ChevronDown className="h-4 w-4 opacity-70" strokeWidth={1.5} aria-hidden />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-48">
                <DropdownMenuLabel>Preferred audio</DropdownMenuLabel>
                {AUDIO_LANGS.map((l) => (
                  <DropdownMenuCheckItem
                    key={l.label}
                    checked={audio === l.value}
                    onSelect={() => onAudio(l.value)}
                  >
                    {l.label}
                  </DropdownMenuCheckItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
            <Button variant="secondary" disabled className="gap-2 opacity-60">
              <Languages className="h-4 w-4" strokeWidth={1.5} aria-hidden /> Translate
              <span className="rounded bg-white/15 px-1 text-xs uppercase">Soon</span>
            </Button>
          </div>
        </div>
        {versions}
      </div>
    </header>
  );
}

function Credits({
  facts,
  detail,
  movie,
}: {
  facts: TitleFactsView | undefined;
  detail: CanonicalMediaDetail | undefined;
  movie: CanonicalMediaSummary | undefined;
}) {
  const audio = useMemo(() => {
    const tags = new Set<string>();
    for (const v of detail?.variants ?? []) {
      for (const b of v.badges) if (["MULTI", "VF", "VOSTFR", "VO"].includes(b)) tags.add(b);
    }
    return [...tags];
  }, [detail]);
  const rows: { k: string; v: string }[] = [];
  if (facts?.directors?.length) rows.push({ k: "Directed by", v: facts.directors.join(", ") });
  if (facts?.studios?.length) rows.push({ k: "Studio", v: facts.studios.slice(0, 2).join(", ") });
  if (detail?.releaseDate) rows.push({ k: "Released", v: formatRelease(detail.releaseDate) });
  else if (movie?.year) rows.push({ k: "Released", v: String(movie.year) });
  if (detail?.originalTitle) rows.push({ k: "Original title", v: detail.originalTitle });
  const runtime = facts?.runtimeMin ?? movie?.runtimeMin;
  if (runtime) rows.push({ k: "Runtime", v: formatRuntime(runtime) });
  if (audio.length) rows.push({ k: "Audio in your versions", v: audio.join(" · ") });

  if (!facts?.cast?.length && rows.length === 0 && !detail?.keywords?.length) return null;
  return (
    <section aria-label="Credits" className="flex flex-col gap-8">
      {(facts?.cast?.length ?? 0) > 0 && (
        <div className="flex flex-col gap-4">
          <h2 className="font-display text-[28px] font-medium">Cast</h2>
          <CastStrip cast={facts?.cast ?? []} people={[]} peopleLabel="" />
        </div>
      )}
      {rows.length > 0 && (
        <dl className="grid grid-cols-3 gap-x-8 gap-y-5 border-t border-white/[0.08] pt-6">
          {rows.map((r) => (
            <div key={r.k} className="flex flex-col gap-1">
              <dt className="text-xs uppercase tracking-[0.08em] text-white/55">{r.k}</dt>
              <dd className="text-sm text-white/90">{r.v}</dd>
            </div>
          ))}
        </dl>
      )}
      {(detail?.keywords?.length ?? 0) > 0 && (
        <ul aria-label="Keywords" className="flex flex-wrap gap-2">
          {detail!.keywords.map((k) => (
            <li
              key={k}
              className="rounded-full border border-white/10 px-2.5 py-1 text-xs text-white/70"
            >
              {k}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

const CollectionStrip = memo(function CollectionStrip({
  name,
  films,
  currentId,
  onOpen,
}: {
  name: string;
  films: CanonicalMediaSummary[];
  currentId: string | null;
  onOpen: (m: CanonicalMediaSummary) => void;
}) {
  return (
    <section aria-labelledby="movie-collection" className="flex flex-col gap-4">
      <div className="flex items-baseline gap-3.5">
        <h2 id="movie-collection" className="font-display text-[28px] font-medium">
          {name}
        </h2>
        <span className="text-sm text-white/60">{films.length} in your library</span>
      </div>
      <div className="-mx-1 flex gap-5 overflow-x-auto px-1 pb-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {[...films]
          .sort((a, b) => (a.year ?? 0) - (b.year ?? 0))
          .map((m) => {
            const here = m.entityId === currentId;
            return (
              <button
                key={m.entityId}
                type="button"
                onClick={() => !here && onOpen(m)}
                aria-current={here ? "page" : undefined}
                aria-label={here ? `${m.title}, this film` : m.title}
                className={cn(
                  "group relative h-[236px] w-[420px] shrink-0 overflow-hidden rounded-[14px] bg-[#111113] text-left transition-transform duration-200 ease-[cubic-bezier(0.16,1,0.3,1)] focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  here
                    ? "shadow-[0_0_0_1px_rgba(224,163,62,0.45)]"
                    : "shadow-[inset_0_0_0_1px_rgba(255,255,255,0.08)] hover:-translate-y-1"
                )}
              >
                <Artwork
                  src={m.backdropUrl ?? m.posterUrl}
                  alt=""
                  loading="lazy"
                  decoding="async"
                  className="absolute inset-0 h-full w-full object-cover"
                  fallback={<PosterFallback title={m.title} />}
                />
                <span className="absolute inset-0 bg-gradient-to-t from-[#0a0a0c]/90 to-transparent" />
                <span className="absolute inset-x-[18px] bottom-4 flex flex-col gap-1.5">
                  <span className="font-display text-[22px] font-medium">
                    {m.title}
                    {m.year ? ` · ${m.year}` : ""}
                  </span>
                  {here && (
                    <span className="text-xs text-[var(--color-brand-hover)]">You're here</span>
                  )}
                </span>
              </button>
            );
          })}
      </div>
    </section>
  );
});
