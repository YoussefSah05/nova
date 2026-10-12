import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { MonitorCog } from "lucide-react";
import type { LiveChannelSummary, LivePlaybackTarget } from "@/types";
import { errorMessage } from "@/app/session";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuCheckItem,
} from "@/components/ui/dropdown-menu";
import type { PlayMedia } from "./PlayerModal";

export function LiveVariantPicker({
  profileId,
  media,
  open,
  onOpenChange,
  onPlay,
  measured,
}: {
  profileId: string;
  media: PlayMedia;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPlay?: (media: PlayMedia) => void;
  measured: string | null;
}) {
  const request = useRef(0);
  const current = useRef(`${profileId}:${media.id}`);
  current.current = `${profileId}:${media.id}`;
  const [error, setError] = useState<string | null>(null);
  const query = useQuery({
    queryKey: ["liveChannel", profileId, media.id],
    queryFn: () =>
      invoke<LiveChannelSummary | null>("get_live_channel", { profileId, channelId: media.id }),
    staleTime: 0,
  });
  async function choose(key: string) {
    const channel = query.data;
    if (!channel || !onPlay) return;
    const sequence = ++request.current;
    const owner = current.current;
    setError(null);
    try {
      const target = await invoke<LivePlaybackTarget>("resolve_live_channel", {
        profileId,
        sourceId: channel.sourceId,
        channelKey: channel.key,
        variantKey: key,
      });
      if (sequence !== request.current || owner !== current.current) return;
      onPlay({
        ...media,
        id: target.channelId,
        title: target.channel.title,
        subtitle: target.channel.edition,
        channelGroup: target.channel.family,
        liveChannelKey: target.channel.key,
        liveVariantKey: target.variantKey,
      });
      onOpenChange(false);
    } catch (e: unknown) {
      if (sequence === request.current && owner === current.current) setError(errorMessage(e));
    }
  }
  return (
    <DropdownMenu open={open} onOpenChange={onOpenChange}>
      <DropdownMenuTrigger
        aria-label="Channel variants"
        className="rounded-full p-2 text-white/70 transition-colors hover:bg-white/10 hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <MonitorCog className="h-5 w-5" />
      </DropdownMenuTrigger>
      <DropdownMenuContent className="max-h-80 w-80 overflow-y-auto" align="end">
        <DropdownMenuLabel>Channel variants</DropdownMenuLabel>
        <p className="px-3 py-2 text-xs text-muted-foreground">
          Provider-advertised quality{measured ? ` · Playing ${measured}` : ""}
        </p>
        {query.isPending && <p className="px-3 py-2 text-sm">Loading variants…</p>}
        {query.isError && (
          <button className="px-3 py-2 text-sm" onClick={() => void query.refetch()}>
            Could not load variants. Retry
          </button>
        )}
        {query.data?.variants.map((v) => (
          <DropdownMenuCheckItem
            key={`${v.variantKey}:${v.channelId}`}
            checked={v.channelId === media.id}
            onSelect={() => {
              if (v.channelId !== media.id) void choose(v.variantKey);
            }}
          >
            <span className="flex flex-col gap-1">
              <span>
                {v.quality ?? "Quality unknown"}
                {v.codecs.length ? ` · ${v.codecs.join(" / ")}` : ""}
                {v.recentlyFailed ? " · Recently failed" : ""}
              </span>
              <span className="text-xs text-muted-foreground">{v.rawName}</span>
            </span>
          </DropdownMenuCheckItem>
        ))}
        {query.isSuccess && !query.data && (
          <p className="px-3 py-2 text-sm">This channel is no longer available.</p>
        )}
        {error && (
          <p role="alert" className="px-3 py-2 text-sm text-destructive">
            {error}
          </p>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
