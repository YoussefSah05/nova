//! Home's hero. With something in progress: that title, full bleed, ready to
//! resume, and the next few in progress beside it. Otherwise a spotlight on a
//! title new in the library.

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { InfoIcon, PlayIcon } from "lucide-react";

import type { ContinueWatchingItem } from "@/types";
import { Artwork } from "@/components/Artwork";
import { Button } from "@/components/ui/button";
import { AmbientGlow, useAmbientColor } from "@/lib/ambient";
import { tmdbSized, useArtworkSrc } from "@/lib/artwork";
import { cn } from "@/lib/utils";
import { continueEpisodeLabel } from "@/features/browse/continueWatching";
import { canonicalMovieQuery } from "@/features/browse/detail/data";
import { movieToItem, type BrowseItem } from "@/features/browse/queries";
import { TitleLogo } from "@/features/browse/TitleLogo";
import { useMinute } from "@/features/browse/guide";
import { useTitleArt, useTitleArtFor } from "@/features/browse/titleArt";
import { useActivate } from "@/app/useActivate";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";
import { clockTime, continueMedia, greeting, progressPct, timeLeft } from "./homeFeed";
import { useScreenActive } from "@/lib/screenActive";

function useGreetingLine(name: string): string {
  useMinute();
  const now = new Date();
  const day = now.toLocaleDateString(undefined, { weekday: "long" });
  const time = now.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return `${greeting(now)}, ${name} · ${day} ${time}`;
}

export function ResumeHero({
  item,
  alsoInProgress,
  spotlight,
}: {
  item: ContinueWatchingItem | null;
  alsoInProgress: ContinueWatchingItem[];
  spotlight: BrowseItem | null;
}) {
  if (item) return <ResumeFrame item={item} alsoInProgress={alsoInProgress} />;
  if (spotlight) return <SpotlightFrame item={spotlight} />;
  return <GreetingOnly />;
}

/// Backdrop, ambient tint and scrims. The image fades in once decoded, over
/// a fixed box, so nothing below it moves.
function HeroBackdrop({ src }: { src: string | null | undefined }) {
  const [usable, onError] = useArtworkSrc(src);
  const ambient = useAmbientColor(usable);
  const [loaded, setLoaded] = useState(false);
  return (
    <div aria-hidden className="absolute inset-0 overflow-hidden">
      {usable ? (
        <img
          src={usable}
          alt=""
          decoding="async"
          fetchPriority="high"
          onLoad={() => setLoaded(true)}
          onError={onError}
          className={cn(
            "h-full w-full object-cover object-top transition-opacity duration-500 ease-[cubic-bezier(0.16,1,0.3,1)]",
            loaded ? "opacity-100" : "opacity-0"
          )}
        />
      ) : (
        <div className="h-full w-full bg-gradient-to-br from-primary/15 via-[#111113] to-[#0a0a0c]" />
      )}
      <AmbientGlow color={ambient} className="h-[80%] mix-blend-screen" />
      <div className="absolute inset-0 bg-gradient-to-r from-[#0a0a0c]/[0.88] via-[#0a0a0c]/40 to-transparent" />
      <div className="absolute inset-x-0 bottom-0 h-[55%] bg-gradient-to-t from-[#0a0a0c] to-transparent" />
    </div>
  );
}

const FRAME = "relative h-[min(78vh,780px)] min-h-[560px] w-full overflow-hidden";

function ResumeFrame({
  item,
  alsoInProgress,
}: {
  item: ContinueWatchingItem;
  alsoInProgress: ContinueWatchingItem[];
}) {
  const { profile } = useSession();
  const { playMedia } = usePlayback();
  const activate = useActivate();
  const line = useGreetingLine(profile.name);
  const entityId = item.mediaKind === "vod" ? item.entityId : null;
  const art = useTitleArtFor(profile.id, entityId ? { kind: "movie", entityId } : null);
  // Shares the movie page's cache: "Details" opens it already loaded.
  const { data: detail } = useQuery({
    subscribed: useScreenActive(),
    ...canonicalMovieQuery(profile.id, entityId),
    enabled: entityId !== null,
    staleTime: 5 * 60 * 1000,
  });
  const pct = progressPct(item);
  const left = timeLeft(item);
  const sub = item.mediaKind === "episode" ? continueEpisodeLabel(item) : null;

  return (
    <section aria-label="Continue watching" className={FRAME}>
      <HeroBackdrop src={art?.backdropUrl ?? item.posterUrl} />
      <div className="absolute bottom-[88px] left-6 right-6 flex max-w-[620px] flex-col gap-4 md:left-[88px] xl:max-w-[min(620px,calc(100%-560px))]">
        <p className="text-sm uppercase tracking-[0.14em] text-white/70">{line}</p>
        <TitleLogo
          title={item.title}
          logoUrl={art?.logoUrl}
          as="h1"
          className="font-display text-[36px] font-medium leading-tight tracking-[-0.015em] drop-shadow-lg"
          logoClassName="max-h-32 md:max-h-36 max-w-[min(30rem,90%)]"
        />
        {sub && <p className="text-base text-white/80">{sub}</p>}
        {pct > 0 && (
          <div className="flex w-full max-w-[360px] flex-col gap-2">
            <div className="flex justify-between text-xs text-white/70">
              <span>Stopped at {clockTime(item.positionSec)}</span>
              {left && <span>{left}</span>}
            </div>
            <div className="h-1 overflow-hidden rounded-full bg-white/[0.18]">
              <div className="h-full rounded-full bg-primary" style={{ width: `${pct}%` }} />
            </div>
          </div>
        )}
        <div className="mt-1.5 flex gap-3">
          <Button size="lg" onClick={() => playMedia(continueMedia(item))} className="gap-2.5">
            <PlayIcon className="h-4 w-4 fill-current" strokeWidth={1.5} aria-hidden />
            {item.upNext ? "Play next episode" : "Resume"}
          </Button>
          {detail && (
            <Button variant="secondary" size="lg" onClick={() => activate(movieToItem(detail))}>
              <InfoIcon className="h-4 w-4" strokeWidth={1.5} aria-hidden />
              Details
            </Button>
          )}
        </div>
      </div>
      {alsoInProgress.length > 0 && <AlsoInProgress items={alsoInProgress} />}
    </section>
  );
}

function AlsoInProgress({ items }: { items: ContinueWatchingItem[] }) {
  const { playMedia } = usePlayback();
  return (
    <aside
      aria-label="Also in progress"
      className="absolute bottom-20 right-14 hidden w-[336px] flex-col gap-1 rounded-2xl border border-white/[0.08] bg-[#111113]/[0.92] p-3.5 shadow-[0_8px_24px_rgba(0,0,0,0.45)] xl:flex"
    >
      <p className="mx-1.5 mb-1.5 mt-0.5 text-xs uppercase tracking-[0.12em] text-white/60">
        Also in progress
      </p>
      {items.map((it) => {
        const pct = progressPct(it);
        const sub =
          it.mediaKind === "episode"
            ? [continueEpisodeLabel(it), timeLeft(it)].filter(Boolean).join(" · ")
            : (timeLeft(it) ?? it.subtitle);
        return (
          <button
            data-card
            key={`${it.mediaKind}-${it.mediaId}`}
            type="button"
            onClick={() => playMedia(continueMedia(it))}
            className="flex items-center gap-3 rounded-[10px] p-2 text-left transition-colors hover:bg-white/[0.05] focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <span className="relative h-[50px] w-[88px] shrink-0 overflow-hidden rounded-md bg-white/[0.06] shadow-[inset_0_0_0_1px_rgba(255,255,255,.08)]">
              <Artwork
                src={tmdbSized(it.posterUrl, "w342")}
                alt=""
                loading="lazy"
                decoding="async"
                className="h-full w-full object-cover"
              />
            </span>
            <span className="flex min-w-0 flex-1 flex-col gap-1">
              <span className="truncate text-sm font-medium">{it.title}</span>
              {sub && <span className="truncate text-xs text-white/60">{sub}</span>}
              {pct > 0 && (
                <span className="h-[3px] overflow-hidden rounded-full bg-white/[0.14]">
                  <span className="block h-full bg-primary" style={{ width: `${pct}%` }} />
                </span>
              )}
            </span>
          </button>
        );
      })}
    </aside>
  );
}

function SpotlightFrame({ item }: { item: BrowseItem }) {
  const { profile } = useSession();
  const { playItem } = usePlayback();
  const activate = useActivate();
  const line = useGreetingLine(profile.name);
  const art = useTitleArt(profile.id, item);
  const overview = item.movie?.overview ?? item.series?.overview ?? null;
  const backdrop = item.movie?.backdropUrl ?? item.series?.backdropUrl ?? null;

  return (
    <section aria-label="Spotlight" className={FRAME}>
      <HeroBackdrop src={art?.backdropUrl ?? backdrop ?? item.posterUrl} />
      <div className="absolute bottom-[88px] left-6 right-6 flex max-w-[620px] flex-col gap-4 md:left-[88px]">
        <p className="text-sm uppercase tracking-[0.14em] text-white/70">
          {line} · New in your library
        </p>
        <TitleLogo
          title={item.title}
          logoUrl={art?.logoUrl}
          as="h1"
          className="font-display text-[36px] font-medium leading-tight tracking-[-0.015em] drop-shadow-lg"
          logoClassName="max-h-32 md:max-h-36 max-w-[min(30rem,90%)]"
        />
        {item.subtitle && (
          <p className="text-sm text-white/75">{item.subtitle.replace(" / ", " · ")}</p>
        )}
        {overview && (
          <p className="line-clamp-3 max-w-xl text-base leading-relaxed text-white/75">
            {overview}
          </p>
        )}
        <div className="mt-1.5 flex gap-3">
          <Button
            size="lg"
            onClick={() => (item.kind === "series" ? activate(item) : void playItem(item))}
            className="gap-2.5"
          >
            <PlayIcon className="h-4 w-4 fill-current" strokeWidth={1.5} aria-hidden />
            {item.kind === "series" ? "Choose episode" : "Play"}
          </Button>
          <Button variant="secondary" size="lg" onClick={() => activate(item)}>
            <InfoIcon className="h-4 w-4" strokeWidth={1.5} aria-hidden />
            Details
          </Button>
        </div>
      </div>
    </section>
  );
}

function GreetingOnly() {
  const { profile } = useSession();
  const line = useGreetingLine(profile.name);
  return (
    <section aria-label="Welcome" className="px-[88px] pb-4 pt-[140px]">
      <p className="text-sm uppercase tracking-[0.14em] text-white/70">{line}</p>
      <h1 className="mt-3 font-display text-[36px] font-medium tracking-[-0.015em]">Tonight</h1>
    </section>
  );
}
