import { useMemo, useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { errorMessage } from "@/app/session";
import type { PlaybackLaunchOptions } from "@/features/player/playbackLaunch";
import type { LiveChannelSummary } from "@/types";
import { ChannelTile } from "./ChannelTile";
import { GuideLine } from "./GuideViews";
import { useChannelGuide } from "./guide";
import { fetchLiveChannels, GRID_PAGE_SIZE, liveChannelToItem, type BrowseItem } from "./queries";

export function LiveBundleDetail({
  profileId,
  sourceId,
  item,
  onBack,
  onPlay,
}: {
  profileId: string;
  sourceId: number | null;
  item: BrowseItem;
  onBack: () => void;
  onPlay: (item: BrowseItem, options?: PlaybackLaunchOptions) => void;
}) {
  const [edition, setEdition] = useState("All editions");
  const query = useInfiniteQuery({
    queryKey: ["liveBundle", profileId, sourceId, item.groupKey],
    queryFn: ({ pageParam }) =>
      fetchLiveChannels(profileId, sourceId ?? -1, item.groupKey ?? "", pageParam),
    initialPageParam: 0,
    getNextPageParam: (last, _pages, offset) =>
      last.length === GRID_PAGE_SIZE ? offset + GRID_PAGE_SIZE : undefined,
    enabled: sourceId !== null && !!item.groupKey,
  });
  const matches = useQuery({
    queryKey: ["liveBundleMatches", profileId, sourceId, item.groupKey, item.searchQuery],
    queryFn: () =>
      fetchLiveChannels(profileId, sourceId ?? -1, item.groupKey ?? "", 0, item.searchQuery),
    enabled: !!item.searchQuery && sourceId !== null,
  });
  const channels = useMemo(() => query.data?.pages.flat() ?? [], [query.data]);
  const editions = useMemo(
    () => [...new Set([...(item.editions ?? []), ...channels.map((c) => c.edition)])].sort(),
    [channels, item.editions]
  );
  const matched = new Set(matches.data?.map((c) => c.key));
  const visible = channels.filter((c) => edition === "All editions" || c.edition === edition);
  return (
    <div className="min-h-screen bg-background px-6 pb-8 pt-24 md:px-[88px]">
      <Button variant="ghost" size="sm" onClick={onBack} className="gap-2">
        <ArrowLeft className="h-4 w-4" />
        Back
      </Button>
      <div className="mt-4 flex items-center gap-4">
        <div className="h-16 w-28 overflow-hidden rounded-xl">
          <ChannelTile logoUrl={item.posterUrl} title={item.title} />
        </div>
        <div>
          <h1 className="font-display text-[28px] font-medium">{item.title}</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {item.memberCount ?? channels.length} channels ·{" "}
            {item.streamCount ?? channels.reduce((n, c) => n + c.variants.length, 0)} streams
          </p>
        </div>
      </div>
      <div className="my-6 flex flex-wrap gap-2" aria-label="Channel editions">
        {["All editions", ...editions].map((e) => (
          <Button
            key={e}
            variant={edition === e ? "secondary" : "ghost"}
            size="sm"
            aria-pressed={edition === e}
            onClick={() => setEdition(e)}
          >
            {e}
          </Button>
        ))}
      </div>
      {item.searchQuery && (
        <p className="mb-4 text-sm text-muted-foreground">
          Matches for “{item.searchQuery}” are highlighted. All channels remain available.
        </p>
      )}
      {query.isPending ? (
        <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
          {Array.from({ length: 8 }, (_, i) => (
            <div key={i} className="aspect-video animate-pulse rounded-xl bg-muted" />
          ))}
        </div>
      ) : query.isError ? (
        <div role="alert">
          <p>{errorMessage(query.error)}</p>
          <Button onClick={() => void query.refetch()}>Retry</Button>
        </div>
      ) : (
        <div className="grid grid-cols-2 gap-4 md:grid-cols-4 lg:grid-cols-5">
          {visible.map((c) => (
            <LiveChannelCard
              key={c.key}
              channel={c}
              matched={matched.has(c.key)}
              onPlay={() => onPlay(liveChannelToItem(c))}
            />
          ))}
        </div>
      )}
      {!query.isPending && !query.isError && visible.length === 0 && (
        <div>
          <p>No channels in this edition.</p>
          <Button onClick={() => setEdition("All editions")}>Show all editions</Button>
        </div>
      )}
      {query.hasNextPage && (
        <Button
          className="mt-6"
          disabled={query.isFetchingNextPage}
          onClick={() => void query.fetchNextPage()}
        >
          Load more channels
        </Button>
      )}
    </div>
  );
}
function LiveChannelCard({
  channel,
  matched,
  onPlay,
}: {
  channel: LiveChannelSummary;
  matched: boolean;
  onPlay: () => void;
}) {
  const guide = useChannelGuide(channel.representativeId);
  const qualities = [...new Set(channel.variants.map((v) => v.quality ?? "Unknown"))];
  return (
    <button
      onClick={onPlay}
      className={cn(
        "rail-contain group flex flex-col gap-2 rounded-xl text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        matched && "ring-1 ring-primary"
      )}
    >
      <div className="aspect-video w-full overflow-hidden rounded-xl transition-transform duration-200 motion-reduce:transition-none group-hover:-translate-y-1">
        <ChannelTile logoUrl={channel.logoUrl} title={channel.title} />
      </div>
      <span className="text-sm font-medium">{channel.title}</span>
      <span className="text-xs text-muted-foreground">
        {channel.edition} · {qualities.join(" / ")} · {channel.variants.length} streams
      </span>
      <GuideLine guide={guide} />
    </button>
  );
}
