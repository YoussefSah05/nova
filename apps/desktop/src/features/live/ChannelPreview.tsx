//! The chosen channel beside the guide: its logo on its brand colour, what's
//! on now (time slot, time left, progress, description), what's later, and
//! Watch / favourite. Reads the guide rows' programmes; a channel without a
//! guide says so plainly.

import { memo } from "react";
import { PlayIcon } from "lucide-react";

import type { Channel, Program } from "@/types";
import { Button } from "@/components/ui/button";
import { ChannelTile } from "@/features/browse/ChannelTile";
import { FavoriteButton } from "@/features/browse/FavoriteButton";
import { formatGuideTime, programmeProgress } from "@/features/browse/guide";
import { nowAndLater, timeLeft } from "./timeline";

export const ChannelPreview = memo(function ChannelPreview({
  channel,
  programs,
  failed = false,
  nowSec,
  onPlay,
  onToggleFavorite,
}: {
  channel: Channel;
  /// The channel's programmes in the guide window; undefined while loading.
  programs: Program[] | undefined;
  /// The guide could not be read.
  failed?: boolean;
  nowSec: number;
  onPlay: (c: Channel) => void;
  onToggleFavorite: (c: Channel, next: boolean) => void;
}) {
  const { now, later } = nowAndLater(programs ?? [], nowSec);
  const pct = now ? Math.round(programmeProgress(now, nowSec * 1000)) : 0;
  return (
    <aside
      aria-label={`${channel.name}, selected channel`}
      className="flex h-full min-h-0 flex-col gap-4 overflow-hidden rounded-2xl border border-white/[0.06] bg-[#111113] p-4"
    >
      <div className="relative h-[180px] shrink-0 overflow-hidden rounded-xl">
        <ChannelTile
          logoUrl={channel.logoUrl}
          title={channel.name}
          logoClassName="max-h-[52%] max-w-[60%]"
        />
        <span className="absolute left-3 top-3 flex items-center gap-1.5 rounded-md bg-black/55 px-2 py-0.5 text-xs font-medium text-white/90">
          <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-[#ff5a4f]" />
          LIVE
        </span>
        {channel.category && (
          <span className="absolute right-3 top-3 max-w-[50%] truncate rounded-md bg-black/55 px-2 py-0.5 text-xs text-white/85">
            {channel.category}
          </span>
        )}
      </div>

      {now ? (
        <div className="flex flex-col gap-1.5">
          <span className="truncate text-xs uppercase tracking-[0.1em] text-white/60">
            {channel.name} · on now
          </span>
          <h2 className="line-clamp-2 font-display text-[22px] font-medium leading-tight">
            {now.title}
          </h2>
          <span className="text-xs tabular-nums text-white/60">
            {formatGuideTime(now.startUtc)}–{formatGuideTime(now.stopUtc)} · {timeLeft(now, nowSec)}
          </span>
          <span
            role="progressbar"
            aria-label={`${now.title}: ${pct}% elapsed`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
            className="mt-1 block h-1 overflow-hidden rounded-full bg-white/[0.12]"
          >
            <span className="block h-full bg-primary" style={{ width: `${pct}%` }} />
          </span>
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          <h2 className="font-display text-[22px] font-medium leading-tight">{channel.name}</h2>
          <span className="text-sm text-white/60">
            {failed
              ? "Couldn't load the guide. The channel plays all the same."
              : programs === undefined
                ? "Loading the guide…"
                : "No guide for this channel. It plays all the same."}
          </span>
        </div>
      )}

      {now?.description && (
        <p className="line-clamp-5 text-sm leading-relaxed text-white/70">{now.description}</p>
      )}

      {later.length > 0 && (
        <div className="flex flex-col gap-0.5 border-t border-white/[0.08] pt-3">
          <span className="mb-1.5 text-xs uppercase tracking-[0.1em] text-white/50">Later</span>
          <ol className="flex flex-col">
            {later.map((p) => (
              <li key={`${p.startUtc}-${p.title}`} className="flex gap-3 py-1 text-sm">
                <span className="w-12 shrink-0 tabular-nums text-white/60">
                  {formatGuideTime(p.startUtc)}
                </span>
                <span className="truncate">{p.title}</span>
              </li>
            ))}
          </ol>
        </div>
      )}

      <div className="flex-1" />
      <div className="flex shrink-0 items-center gap-2.5">
        <Button onClick={() => onPlay(channel)} className="h-11 flex-1 gap-2 rounded-full">
          <PlayIcon className="h-4 w-4 fill-current" strokeWidth={1.5} aria-hidden />
          Watch
        </Button>
        <FavoriteButton
          isFavorite={channel.isFavorite}
          onToggle={(next) => onToggleFavorite(channel, next)}
          label={channel.name}
        />
      </div>
    </aside>
  );
});
