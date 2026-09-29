//! "Continue your shows": the shows in progress as wide cards — the show's
//! text-free backdrop and logo, the episode to resume or play next, and one
//! tick per episode for how far through the show the profile is. The card
//! opens the show; its play button plays the episode.

import { memo, useMemo } from "react";
import { PlayIcon } from "lucide-react";

import type { ContinueWatchingItem, SeriesSummary } from "@/types";
import { Artwork } from "@/components/Artwork";
import { PosterFallback } from "@/features/browse/PosterFallback";
import { seriesToItem } from "@/features/browse/queries";
import {
  episodeCode,
  progressTicks,
  showMeta,
  statusLabel,
  watchedLine,
} from "@/features/browse/shows";
import { TitleLogo } from "@/features/browse/TitleLogo";
import { useTitleArt } from "@/features/browse/titleArt";
import { tmdbSized } from "@/lib/artwork";
import { useActivate } from "@/app/useActivate";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";
import { continueMedia, progressPct } from "@/sections/home/homeFeed";

export function ContinueShows({
  items,
  cards,
}: {
  items: ContinueWatchingItem[];
  cards: Map<string, SeriesSummary>;
}) {
  if (items.length === 0) return null;
  return (
    <section aria-labelledby="shows-continue" className="flex flex-col gap-[18px]">
      <h2 id="shows-continue" className="font-display text-[28px] font-medium">
        Continue your shows
      </h2>
      <ul className="grid grid-cols-3 gap-6">
        {items.map((item) => (
          <li key={`${item.mediaKind}-${item.mediaId}`}>
            <ContinueCard item={item} show={cards.get(item.seriesTitle!) ?? null} />
          </li>
        ))}
      </ul>
    </section>
  );
}

const ContinueCard = memo(function ContinueCard({
  item,
  show,
}: {
  item: ContinueWatchingItem;
  show: SeriesSummary | null;
}) {
  const { profile } = useSession();
  const { playMedia } = usePlayback();
  const activate = useActivate();
  const browseItem = useMemo(() => (show ? seriesToItem(show) : null), [show]);
  const art = useTitleArt(profile.id, browseItem);
  const code =
    item.season !== null && item.episode !== null && item.numbering !== "unknown"
      ? episodeCode(item.season, item.episode)
      : (item.episodeTitle ?? "Episode");
  const next = item.upNext ? `${code} next` : `Resume ${code}`;
  const ticks = show ? progressTicks(show) : null;
  const watched = show ? watchedLine(show) : null;
  const status = show ? statusLabel(show) : null;
  const meta = show ? [status, showMeta(show)].filter(Boolean).join(" · ") : null;
  const backdrop =
    art?.backdropUrl ?? tmdbSized(show?.backdropUrl ?? null, "w780") ?? item.posterUrl;
  const pct = item.upNext ? 0 : progressPct(item);
  // Until the show's card loads there is no page to open: the play button
  // still works.
  const open = () => browseItem && activate(browseItem);

  return (
    <div className="group relative flex flex-col overflow-hidden rounded-2xl bg-[#111113] shadow-[inset_0_0_0_1px_rgba(255,255,255,.06)] transition-transform duration-200 ease-[cubic-bezier(0.16,1,0.3,1)] hover:-translate-y-1">
      <button
        type="button"
        onClick={open}
        disabled={!browseItem}
        aria-label={[item.title, next, watched].filter(Boolean).join(", ")}
        className="flex flex-col text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      >
        <span className="relative block h-[230px] overflow-hidden bg-white/[0.04]">
          <Artwork
            src={backdrop}
            alt=""
            loading="lazy"
            decoding="async"
            className="absolute inset-0 h-full w-full object-cover"
            fallback={<PosterFallback title={item.title} />}
          />
          <span
            aria-hidden
            className="absolute inset-0 bg-gradient-to-t from-[#0a0a0c]/[0.85] via-transparent to-transparent"
          />
          <span className="absolute bottom-4 left-5 right-16">
            <TitleLogo
              title={item.title}
              logoUrl={art?.logoUrl}
              as="span"
              className="line-clamp-2 font-display text-[28px] font-medium leading-none tracking-[-0.01em]"
              logoClassName="max-h-14 max-w-[min(15rem,90%)]"
            />
          </span>
          {pct > 0 && (
            <span aria-hidden className="absolute inset-x-0 bottom-0 h-[3px] bg-white/10">
              <span className="block h-full bg-primary" style={{ width: `${pct}%` }} />
            </span>
          )}
        </span>
        <span className="flex flex-col gap-2.5 px-5 pb-[18px] pt-4">
          <span className="flex items-baseline justify-between gap-3">
            <span className="truncate text-base font-medium">{next}</span>
            {watched && <span className="shrink-0 text-xs text-white/60">{watched}</span>}
          </span>
          {ticks ? (
            <span aria-hidden className="flex gap-[3px]">
              {ticks.map((done, i) => (
                <span
                  key={i}
                  className={
                    done
                      ? "h-1 flex-1 rounded-sm bg-primary"
                      : "h-1 flex-1 rounded-sm bg-white/[0.14]"
                  }
                />
              ))}
            </span>
          ) : show && show.episodeCount > 0 ? (
            <span aria-hidden className="h-1 overflow-hidden rounded-sm bg-white/[0.14]">
              <span
                className="block h-full bg-primary"
                style={{
                  width: `${Math.min(100, ((show.watchedCount ?? 0) / show.episodeCount) * 100)}%`,
                }}
              />
            </span>
          ) : null}
          {meta && <span className="truncate text-xs text-white/60">{meta}</span>}
        </span>
      </button>
      <button
        type="button"
        onClick={() => playMedia(continueMedia(item))}
        aria-label={`Play ${item.title} ${code}`}
        className="absolute right-4 top-[174px] flex h-11 w-11 items-center justify-center rounded-full bg-white text-neutral-950 shadow-[0_8px_24px_rgba(0,0,0,0.45)] transition-transform duration-200 hover:scale-105 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <PlayIcon className="h-5 w-5 fill-current" strokeWidth={1.5} aria-hidden />
      </button>
    </div>
  );
});
