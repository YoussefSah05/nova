//! The Cinémathèque spotlight: one film framed the way cinema is, in a
//! 2.39:1 letterbox — a film on your watchlist when there is one with art,
//! else the newest film with art.

import { InfoIcon, PlayIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useArtworkSrc } from "@/lib/artwork";
import { formatRuntime } from "@/features/browse/detail/meta";
import {
  RECENTLY_ADDED_RAIL,
  WATCHLIST_RAIL,
  useRail,
  type BrowseItem,
} from "@/features/browse/queries";
import { TitleLogo } from "@/features/browse/TitleLogo";
import { useTitleArt } from "@/features/browse/titleArt";
import { useActivate } from "@/app/useActivate";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";

function withArt(items: BrowseItem[] | undefined): BrowseItem | null {
  return items?.find((i) => i.movie?.backdropUrl) ?? null;
}

export function Spotlight() {
  const { profile, activeSourceId } = useSession();
  const watchlist = useRail("movies", profile.id, activeSourceId, WATCHLIST_RAIL, true);
  const recent = useRail("movies", profile.id, activeSourceId, RECENTLY_ADDED_RAIL, true);
  const fromWatchlist = withArt(watchlist.data);
  const item = fromWatchlist ?? withArt(recent.data);
  // Hold the frame while loading so the shelves below do not jump down.
  if (!item && (watchlist.isPending || recent.isPending))
    return <div aria-hidden className="nova-shimmer h-[450px] rounded-2xl bg-white/[0.03]" />;
  if (!item) return null;
  return (
    <SpotlightFrame
      item={item}
      eyebrow={fromWatchlist ? "On your watchlist" : "New in your library"}
    />
  );
}

function SpotlightFrame({ item, eyebrow }: { item: BrowseItem; eyebrow: string }) {
  const { profile } = useSession();
  const { playItem } = usePlayback();
  const activate = useActivate();
  const art = useTitleArt(profile.id, item);
  const [src, onError] = useArtworkSrc(art?.backdropUrl ?? item.movie?.backdropUrl ?? null);
  const m = item.movie;
  const meta = [
    m?.year ? String(m.year) : null,
    m?.runtimeMin ? formatRuntime(m.runtimeMin) : null,
    m?.genres?.slice(0, 2).join(", ") || m?.category || null,
    m?.rating ? `★ ${m.rating.toFixed(1)}` : null,
  ].filter(Boolean);

  return (
    <section
      aria-label={`Spotlight: ${item.title}`}
      className="relative h-[450px] overflow-hidden rounded-2xl bg-black"
    >
      {/* The picture sits between letterbox bars, as in a cinema. */}
      <div aria-hidden className="absolute inset-x-0 top-[44px] bottom-[44px] overflow-hidden">
        {src && (
          <img
            src={src}
            alt=""
            loading="lazy"
            decoding="async"
            onError={onError}
            className="h-full w-full object-cover"
          />
        )}
        <div className="absolute inset-0 bg-gradient-to-r from-black/[0.88] via-black/40 to-transparent" />
      </div>
      <div className="absolute left-14 top-[88px] flex w-[560px] flex-col gap-3.5">
        <p className="text-xs uppercase tracking-[0.16em] text-white/70">Spotlight · {eyebrow}</p>
        <TitleLogo
          title={item.title}
          logoUrl={art?.logoUrl}
          as="h2"
          className="font-display text-[36px] font-medium leading-tight"
          logoClassName="max-h-24 max-w-[min(26rem,90%)]"
        />
        {meta.length > 0 && <p className="text-sm text-white/80">{meta.join(" · ")}</p>}
        {m?.overview && (
          <p className="line-clamp-2 text-base leading-relaxed text-white/75">{m.overview}</p>
        )}
        <div className="mt-1.5 flex gap-3">
          <Button onClick={() => void playItem(item)} className="gap-2.5">
            <PlayIcon className="h-4 w-4 fill-current" strokeWidth={1.5} aria-hidden />
            Play
          </Button>
          <Button variant="secondary" onClick={() => activate(item)}>
            <InfoIcon className="h-4 w-4" strokeWidth={1.5} aria-hidden />
            Details
          </Button>
        </div>
      </div>
    </section>
  );
}
