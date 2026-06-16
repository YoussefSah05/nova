//! Featured hero banner for a tab. Reuses the eager Recently-Added rail query
//! (shared cache key — no extra IPC) and spotlights the first item with a
//! backdrop. Movies/series get a full-bleed poster backdrop; live gets a clean
//! gradient with the channel logo (channel "posters" are logos, not artwork).

import { InfoIcon, PlayIcon, PlusIcon, CheckIcon, Tv2Icon } from "lucide-react";

import { cn } from "@/lib/utils";
import { motion, Stagger, StaggerItem, useReducedMotion } from "@/lib/motion";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { RECENTLY_ADDED_RAIL, useRail, type BrowseItem, type BrowseTab } from "./queries";
import type { PosterCardHandlers } from "./PosterCard";

const KIND_LABEL: Record<BrowseTab, string> = {
  live: "Live channel",
  movies: "Movie",
  series: "Series",
};

export function BrowseHero({
  tab,
  profileId,
  sourceId,
  onActivate,
  onToggleWatchlist,
}: {
  tab: BrowseTab;
  profileId: string;
  sourceId: number;
} & Pick<PosterCardHandlers, "onActivate" | "onToggleWatchlist">) {
  const { data, isLoading } = useRail(tab, profileId, sourceId, RECENTLY_ADDED_RAIL, true);
  const featured: BrowseItem | undefined = data?.find((i) => i.posterUrl) ?? data?.[0];
  const live = tab === "live";
  // A wide TMDB backdrop suits the full-bleed hero better than a portrait poster.
  const heroImage = featured
    ? (featured.vod?.backdropUrl ?? featured.series?.backdropUrl ?? featured.posterUrl)
    : null;

  const reduce = useReducedMotion();

  if (isLoading || !data) {
    return <Skeleton className="h-[42vh] min-h-[300px] w-full rounded-2xl" />;
  }
  if (!featured) return null;

  return (
    // Keyed so the entrance + Ken-Burns replay when the featured item changes.
    <section
      key={featured.key}
      className="relative h-[42vh] min-h-[300px] w-full overflow-hidden rounded-2xl border border-border bg-card"
    >
      {heroImage && !live ? (
        <motion.img
          src={heroImage}
          alt=""
          decoding="async"
          className="absolute inset-0 h-full w-full object-cover object-top"
          initial={reduce ? false : { scale: 1.08 }}
          animate={reduce ? undefined : { scale: 1 }}
          transition={{ duration: 12, ease: "linear" }}
        />
      ) : (
        <div className="absolute inset-0 bg-gradient-to-br from-primary/25 via-card to-background" />
      )}
      {/* Scrims for legibility. */}
      <div className="absolute inset-0 bg-gradient-to-t from-background via-background/55 to-transparent" />
      <div className="absolute inset-0 bg-gradient-to-r from-background/85 via-background/25 to-transparent" />

      <Stagger className="absolute inset-x-0 bottom-0 flex flex-col gap-3 p-6 md:max-w-xl md:p-8">
        <StaggerItem>
          <Badge variant="muted" className="w-fit gap-1.5 backdrop-blur">
            <Tv2Icon className="h-3 w-3" aria-hidden />
            {KIND_LABEL[tab]}
          </Badge>
        </StaggerItem>
        <StaggerItem>
          <h2 className="font-display line-clamp-2 text-3xl font-semibold tracking-tight text-white drop-shadow-lg md:text-4xl">
            {featured.title}
          </h2>
        </StaggerItem>
        {featured.subtitle && (
          <StaggerItem>
            <p className="line-clamp-1 text-sm text-white/70">{featured.subtitle}</p>
          </StaggerItem>
        )}
        <StaggerItem className="mt-1 flex items-center gap-2">
          <Button onClick={() => onActivate(featured)} className="gap-2">
            {live || tab === "movies" ? (
              <>
                <PlayIcon className="h-4 w-4 fill-current" aria-hidden /> Play
              </>
            ) : (
              <>
                <InfoIcon className="h-4 w-4" aria-hidden /> View series
              </>
            )}
          </Button>
          <Button
            variant="secondary"
            onClick={() => onToggleWatchlist(featured, !featured.isInWatchlist)}
            className={cn("gap-2 backdrop-blur", featured.isInWatchlist && "text-primary")}
          >
            {featured.isInWatchlist ? (
              <CheckIcon className="h-4 w-4" aria-hidden />
            ) : (
              <PlusIcon className="h-4 w-4" aria-hidden />
            )}
            Watchlist
          </Button>
        </StaggerItem>
      </Stagger>
    </section>
  );
}
