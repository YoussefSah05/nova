//! Two editorial modules side by side: the episodes to watch next, and what
//! just arrived in the library (one feature, then a short list — not another
//! poster rail).

import { memo } from "react";

import type { ContinueWatchingItem } from "@/types";
import { Artwork } from "@/components/Artwork";
import { tmdbSized } from "@/lib/artwork";
import { PosterFallback } from "@/features/browse/PosterFallback";
import { continueEpisodeLabel } from "@/features/browse/continueWatching";
import { RECENTLY_ADDED_RAIL, useRail, type BrowseItem } from "@/features/browse/queries";
import { useActivate } from "@/app/useActivate";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";
import { continueMedia, timeLeft } from "./homeFeed";

export function UpNextAndNew({ upNext }: { upNext: ContinueWatchingItem[] }) {
  const { profile, activeSourceId } = useSession();
  const recent =
    useRail("movies", profile.id, activeSourceId, RECENTLY_ADDED_RAIL, true).data ?? [];
  const hasNew = recent.length > 0;
  if (upNext.length === 0 && !hasNew) return null;

  return (
    <div
      className={
        upNext.length > 0 && hasNew
          ? "nova-cv grid grid-cols-[minmax(0,5fr)_minmax(0,7fr)] gap-12 px-[88px]"
          : "nova-cv px-[88px]"
      }
    >
      {upNext.length > 0 && (
        <section aria-labelledby="home-up-next" className="flex flex-col gap-[18px]">
          <div className="flex items-baseline gap-3.5">
            <h2 id="home-up-next" className="font-display text-[28px] font-medium">
              Up next
            </h2>
            <span className="text-sm text-white/60">Your shows</span>
          </div>
          {upNext.map((e) => (
            <EpisodeCard key={`${e.mediaKind}-${e.mediaId}`} item={e} />
          ))}
        </section>
      )}
      {hasNew && <NewInLibrary items={recent} />}
    </div>
  );
}

const EpisodeCard = memo(function EpisodeCard({ item }: { item: ContinueWatchingItem }) {
  const { playMedia } = usePlayback();
  const left = timeLeft(item);
  return (
    <button
      data-card
      type="button"
      onClick={() => playMedia(continueMedia(item))}
      className="flex items-center gap-4 rounded-[14px] bg-[#111113] p-2.5 text-left shadow-[inset_0_0_0_1px_rgba(255,255,255,.06)] transition-transform duration-200 ease-[cubic-bezier(0.16,1,0.3,1)] hover:-translate-y-1 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span className="relative h-[117px] w-[208px] shrink-0 overflow-hidden rounded-[10px] bg-white/[0.06]">
        <Artwork
          src={tmdbSized(item.posterUrl, "w342")}
          alt=""
          loading="lazy"
          decoding="async"
          className="h-full w-full object-cover"
          fallback={<PosterFallback title={item.title} />}
        />
        <span className="absolute inset-0 bg-gradient-to-t from-black/60 to-transparent" />
        <span className="absolute bottom-2.5 left-2.5 right-2.5 truncate font-display text-lg font-medium [text-shadow:0_1px_3px_rgba(0,0,0,0.55)]">
          {item.title}
        </span>
      </span>
      <span className="flex min-w-0 flex-col gap-1.5">
        <span className="text-xs uppercase tracking-[0.08em] text-primary">
          {item.upNext ? "Up next" : "Continue"}
        </span>
        <span className="truncate text-base font-medium">{continueEpisodeLabel(item)}</span>
        {left && <span className="text-sm text-white/60">{left}</span>}
      </span>
    </button>
  );
});

function NewInLibrary({ items }: { items: BrowseItem[] }) {
  const activate = useActivate();
  const [feature, ...rest] = items;
  const featureBackdrop = feature.movie?.backdropUrl ?? null;
  return (
    <section aria-labelledby="home-new" className="flex flex-col gap-[18px]">
      <div className="flex items-baseline gap-3.5">
        <h2 id="home-new" className="font-display text-[28px] font-medium">
          New in your library
        </h2>
        <span className="text-sm text-white/60">Recently added films</span>
      </div>
      <div className="grid grid-cols-2 gap-5">
        <button
          data-card
          type="button"
          onClick={() => activate(feature)}
          aria-label={feature.title}
          className="group relative h-[332px] overflow-hidden rounded-2xl bg-[#111113] text-left shadow-[inset_0_0_0_1px_rgba(255,255,255,.08)] transition-transform duration-200 ease-[cubic-bezier(0.16,1,0.3,1)] hover:-translate-y-1 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Artwork
            src={tmdbSized(featureBackdrop ?? feature.posterUrl, "w780")}
            alt=""
            loading="lazy"
            decoding="async"
            className="absolute inset-0 h-full w-full object-cover"
            fallback={<PosterFallback title={feature.title} />}
          />
          <span className="absolute inset-0 bg-gradient-to-t from-[#0a0a0c]/[0.92] via-[#0a0a0c]/30 to-transparent" />
          <span className="absolute inset-x-5 bottom-[18px] flex flex-col gap-2">
            <span className="text-xs uppercase tracking-[0.12em] text-white/70">Just added</span>
            <span className="font-display text-[28px] font-medium leading-tight">
              {feature.title}
            </span>
            {feature.subtitle && (
              <span className="text-sm text-white/70">
                {feature.subtitle.replace(" / ", " · ")}
              </span>
            )}
          </span>
        </button>
        <ol className="flex flex-col gap-0.5">
          {rest.slice(0, 5).map((it) => (
            <li key={it.key}>
              <button
                data-card
                type="button"
                onClick={() => activate(it)}
                className="flex w-full items-center gap-3.5 rounded-[10px] p-2 text-left transition-colors hover:bg-white/[0.05] focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="h-[60px] w-10 shrink-0 overflow-hidden rounded-[5px] bg-white/[0.06] shadow-[inset_0_0_0_1px_rgba(255,255,255,.08)]">
                  <Artwork
                    src={tmdbSized(it.posterUrl, "w185")}
                    alt=""
                    loading="lazy"
                    decoding="async"
                    className="h-full w-full object-cover"
                  />
                </span>
                <span className="flex min-w-0 flex-col gap-1">
                  <span className="truncate text-sm font-medium">{it.title}</span>
                  {it.subtitle && (
                    <span className="truncate text-xs text-white/60">
                      {it.subtitle.replace(" / ", " · ")}
                    </span>
                  )}
                </span>
              </button>
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}
