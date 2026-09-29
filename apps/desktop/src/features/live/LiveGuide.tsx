//! A guide over some channels (a slice of the playlist, or one bundle) with
//! the chosen channel's preview beside it. The Live section and bundle pages
//! both show this.

import { useCallback, useMemo, useState } from "react";
import { TvIcon } from "lucide-react";

import type { Channel } from "@/types";
import { channelToItem } from "@/features/browse/queries";
import { useMinute } from "@/features/browse/guide";
import { useLibraryActions } from "@/app/libraryActions";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";
import { ChannelPreview } from "./ChannelPreview";
import { useGuideChannels, useGuidePrograms, type GuideScope } from "./data";
import { GuideGrid } from "./GuideGrid";
import { guideWindow } from "./timeline";

export function LiveGuide({ scope, emptyLabel }: { scope: GuideScope; emptyLabel: string }) {
  const { profile, activeSourceId } = useSession();
  const { playItem } = usePlayback();
  const { toggleFavorite } = useLibraryActions();
  const minute = useMinute();
  const nowSec = minute * 60;
  const halfHour = Math.floor(nowSec / 1800);
  // The window moves on the half hour only, so its queries stay cached.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const window = useMemo(() => guideWindow(nowSec), [halfHour]);

  const query = useGuideChannels(profile.id, activeSourceId, scope);
  const channels = useMemo(() => query.data?.pages.flat() ?? [], [query.data]);
  const [range, setRange] = useState<[number, number]>([0, -1]);
  const onVisibleRows = useCallback(
    (first: number, last: number) =>
      setRange((prev) => (prev[0] === first && prev[1] === last ? prev : [first, last])),
    []
  );
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const selectedIndex = Math.max(
    0,
    channels.findIndex((c) => c.id === selectedId)
  );
  const selected = channels[selectedIndex] ?? null;
  const programs = useGuidePrograms(
    profile.id,
    channels,
    range[0],
    range[1],
    window,
    selectedIndex
  );
  const onSelect = useCallback((c: Channel) => setSelectedId(c.id), []);
  const onPlay = useCallback((c: Channel) => void playItem(channelToItem(c)), [playItem]);
  const onToggleFavorite = useCallback(
    (c: Channel, next: boolean) => void toggleFavorite(channelToItem(c), next),
    [toggleFavorite]
  );
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = query;
  const onLoadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  if (query.isLoading) {
    return (
      <div className="flex min-h-0 flex-1 gap-4">
        <div className="nova-shimmer flex-1 rounded-2xl bg-white/[0.03]" />
        <div className="nova-shimmer w-[352px] rounded-2xl bg-white/[0.03]" />
      </div>
    );
  }
  if (channels.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 text-center">
        <TvIcon className="h-8 w-8 text-white/40" strokeWidth={1.5} aria-hidden />
        <p className="text-sm text-white/60">{emptyLabel}</p>
      </div>
    );
  }
  return (
    <div className="flex min-h-0 flex-1 gap-4">
      <section
        aria-label="Programme guide"
        className="min-w-0 flex-1 overflow-hidden rounded-2xl border border-white/[0.06] bg-[#0a0a0c]"
      >
        <GuideGrid
          channels={channels}
          programs={programs}
          window={window}
          nowSec={nowSec}
          selectedId={selected?.id ?? null}
          onSelect={onSelect}
          onPlay={onPlay}
          onVisibleRows={onVisibleRows}
          hasMore={!!hasNextPage}
          onLoadMore={onLoadMore}
        />
      </section>
      {selected && (
        <div className="w-[352px] shrink-0">
          <ChannelPreview
            channel={selected}
            programs={programs.get(selected.id)}
            nowSec={nowSec}
            onPlay={onPlay}
            onToggleFavorite={onToggleFavorite}
          />
        </div>
      )}
    </div>
  );
}
