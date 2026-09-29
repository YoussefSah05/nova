//! "On air now": what the channels you watch are showing, with how much is
//! left. Recent channels first, then favourites; the guide lines are batched
//! into one request by `useChannelGuide`.

import { memo } from "react";
import { ArrowRightIcon } from "lucide-react";

import type { ContinueWatchingItem } from "@/types";
import { ChannelTile } from "@/features/browse/ChannelTile";
import { useChannelGuide, useMinute, programmeProgress } from "@/features/browse/guide";
import { FAVORITES_RAIL, useRail } from "@/features/browse/queries";
import { useNavActions } from "@/app/navigation/useNav";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";
import { HomeModule } from "./HomeModule";

const MAX_TILES = 6;

interface OnAirChannel {
  id: number;
  title: string;
  logoUrl: string | null;
}

export function OnAirNow({ recent }: { recent: ContinueWatchingItem[] }) {
  const { profile, activeSourceId } = useSession();
  const { switchTo } = useNavActions();
  const favourites = useRail("live", profile.id, activeSourceId, FAVORITES_RAIL, true).data ?? [];

  const channels: OnAirChannel[] = [];
  const seen = new Set<number>();
  for (const c of recent) {
    if (!seen.has(c.mediaId)) {
      seen.add(c.mediaId);
      channels.push({ id: c.mediaId, title: c.title, logoUrl: c.posterUrl });
    }
  }
  for (const f of favourites) {
    // A bundle card stands for several channels: it has no single programme.
    if ((f.memberCount ?? 1) > 1 || seen.has(f.id)) continue;
    seen.add(f.id);
    channels.push({ id: f.id, title: f.title, logoUrl: f.posterUrl });
  }
  if (channels.length === 0) return null;

  return (
    <HomeModule
      id="home-on-air"
      title="On air now"
      note="From the channels you watch"
      action={
        <button
          type="button"
          onClick={() => switchTo("live")}
          className="flex items-center gap-1.5 rounded-md text-sm text-white/70 transition-colors hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          All channels
          <ArrowRightIcon className="h-3.5 w-3.5" strokeWidth={1.5} aria-hidden />
        </button>
      }
    >
      <div className="grid grid-cols-6 gap-4">
        {channels.slice(0, MAX_TILES).map((c) => (
          <OnAirTile key={c.id} channel={c} />
        ))}
      </div>
    </HomeModule>
  );
}

const OnAirTile = memo(function OnAirTile({ channel }: { channel: OnAirChannel }) {
  const { playMedia } = usePlayback();
  const guide = useChannelGuide(channel.id);
  useMinute();
  const now = guide?.now ?? null;
  const minutesLeft = now
    ? Math.max(0, Math.round((now.stopUtc * 1000 - Date.now()) / 60000))
    : null;
  const left =
    minutesLeft === null
      ? null
      : minutesLeft >= 60
        ? `${Math.floor(minutesLeft / 60)}h ${minutesLeft % 60}m left`
        : `${minutesLeft} min left`;

  return (
    <button
      type="button"
      onClick={() =>
        playMedia({
          kind: "live",
          id: channel.id,
          title: channel.title,
          posterUrl: channel.logoUrl,
        })
      }
      className="group flex flex-col overflow-hidden rounded-[14px] bg-[#111113] text-left shadow-[inset_0_0_0_1px_rgba(255,255,255,.06)] transition-transform duration-200 ease-[cubic-bezier(0.16,1,0.3,1)] hover:-translate-y-1 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span className="relative block h-[104px]">
        <ChannelTile logoUrl={channel.logoUrl} title={channel.title} />
        <span className="absolute left-2.5 top-2.5 flex items-center gap-1.5 rounded-[5px] bg-black/45 px-1.5 py-0.5 text-xs font-medium text-white/90">
          <span className="h-1.5 w-1.5 rounded-full bg-[#ff5a4f]" aria-hidden />
          LIVE
        </span>
      </span>
      <span className="flex flex-col gap-1.5 px-3.5 pb-3.5 pt-3">
        <span className="truncate text-sm font-medium">{now?.title ?? channel.title}</span>
        <span className="truncate text-xs text-white/60">
          {now ? [channel.title, left].filter(Boolean).join(" · ") : "No guide for this channel"}
        </span>
        <span className="h-[3px] overflow-hidden rounded-full bg-white/[0.12]">
          {now && (
            <span
              className="block h-full bg-primary"
              style={{ width: `${programmeProgress(now)}%` }}
            />
          )}
        </span>
      </span>
    </button>
  );
});
