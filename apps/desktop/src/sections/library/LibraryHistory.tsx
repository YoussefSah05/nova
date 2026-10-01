import { useState } from "react";
import { FilmIcon, PlayIcon } from "lucide-react";
import { usePlayback } from "@/app/playback";
import { useScreenScroll } from "@/app/screenContext";
import { useNavActions } from "@/app/navigation/useNav";
import { Artwork } from "@/components/Artwork";
import { Button } from "@/components/ui/button";
import { SimpleSelect } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import type { LibraryHistoryItem } from "@/types/generated/LibraryHistoryItem";
import {
  HISTORY_PAGE_SIZE,
  historyDay,
  historyProgress,
  historySubtitle,
  useLibraryHistory,
  type HistoryKind,
} from "./history";

export function LibraryHistory({ profileId, sourceId }: { profileId: string; sourceId: number }) {
  const [kind, setKind] = useState<HistoryKind>("all");
  const [offset, setOffset] = useState(0);
  const history = useLibraryHistory(profileId, sourceId, kind, offset);
  const { switchTo } = useNavActions();
  const scrollRef = useScreenScroll();
  const items = history.data?.slice(0, HISTORY_PAGE_SIZE) ?? [];
  const groups = new Map<string, LibraryHistoryItem[]>();
  for (const item of items) {
    const day = historyDay(item.watchedAt);
    groups.set(day, [...(groups.get(day) ?? []), item]);
  }
  function page(next: number) {
    setOffset(next);
    scrollRef.current?.scrollTo({ top: 0 });
  }
  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <p className="text-sm text-muted-foreground">
          Your most recent watch of each title in this playlist.
        </p>
        <SimpleSelect
          value={kind}
          onChange={(next) => {
            setKind(next);
            setOffset(0);
          }}
          ariaLabel="History media type"
          options={[
            { value: "all", label: "All media" },
            { value: "vod", label: "Films" },
            { value: "episode", label: "Episodes" },
            { value: "live", label: "Channels" },
          ]}
        />
      </div>
      {history.isError ? (
        <div
          role="alert"
          className="flex flex-col items-center gap-4 rounded-xl border border-border py-16"
        >
          <p>Couldn’t load your history.</p>
          <Button variant="outline" onClick={() => void history.refetch()}>
            Try again
          </Button>
        </div>
      ) : history.isPending ? (
        <div aria-label="Loading history" className="space-y-4">
          {Array.from({ length: 5 }, (_, i) => (
            <Skeleton key={i} className="h-24 w-full" />
          ))}
        </div>
      ) : items.length === 0 ? (
        <div className="flex flex-col items-center gap-4 rounded-xl border border-border py-16 text-center">
          <p className="text-lg font-medium">
            {offset ? "You’ve reached the end" : "Your next watch starts the story"}
          </p>
          <p className="text-sm text-muted-foreground">
            {offset
              ? "There are no more titles on this page."
              : "Films, episodes and channels you watch will appear here."}
          </p>
          <Button
            variant="outline"
            onClick={() =>
              offset
                ? page(0)
                : switchTo(kind === "episode" ? "series" : kind === "live" ? "live" : "movies")
            }
          >
            {offset ? "Back to latest" : "Find something to watch"}
          </Button>
        </div>
      ) : (
        <div className="flex flex-col gap-8">
          {[...groups].map(([day, rows]) => (
            <section key={day} aria-label={day} className="flex flex-col gap-3">
              <h2 className="text-sm font-medium text-muted-foreground">{day}</h2>
              <ul className="divide-y divide-border rounded-xl border border-border bg-card/40">
                {rows.map((item) => (
                  <HistoryRow key={item.key} item={item} />
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
      {!history.isPending && !history.isError && (items.length > 0 || offset > 0) && (
        <nav aria-label="History pages" className="flex items-center justify-between gap-4">
          <p role="status" className="text-xs text-muted-foreground">
            Page {Math.floor(offset / HISTORY_PAGE_SIZE) + 1} · {items.length} titles
          </p>
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={offset === 0}
              onClick={() => page(Math.max(0, offset - HISTORY_PAGE_SIZE))}
            >
              Previous page
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={(history.data?.length ?? 0) <= HISTORY_PAGE_SIZE}
              onClick={() => page(offset + HISTORY_PAGE_SIZE)}
            >
              Next page
            </Button>
          </div>
        </nav>
      )}
    </div>
  );
}

function HistoryRow({ item }: { item: LibraryHistoryItem }) {
  const { playMedia } = usePlayback();
  const subtitle = historySubtitle(item);
  const action =
    item.mediaKind === "live" ? "Watch live" : item.completed ? "Watch again" : "Resume";
  const progress =
    item.durationSec && item.durationSec > 0
      ? Math.min(100, Math.max(0, (item.positionSec / item.durationSec) * 100))
      : null;
  return (
    <li className="flex flex-wrap items-center gap-4 p-4">
      <div className="flex h-20 w-14 shrink-0 items-center justify-center overflow-hidden rounded-md bg-muted">
        <Artwork
          src={item.posterUrl}
          className="h-full w-full object-cover"
          loading="lazy"
          fallback={<FilmIcon className="text-muted-foreground" strokeWidth={1.5} aria-hidden />}
        />
      </div>
      <div className="min-w-0 flex-1 basis-40">
        <p className="truncate text-base font-medium" title={item.title}>
          {item.title}
        </p>
        <p className="truncate text-sm text-muted-foreground" title={subtitle}>
          {subtitle}
        </p>
        <div className="mt-2 flex flex-wrap gap-x-3 text-xs text-muted-foreground">
          <span>{historyProgress(item)}</span>
          <time dateTime={new Date(item.watchedAt * 1000).toISOString()}>
            {new Date(item.watchedAt * 1000).toLocaleTimeString(undefined, {
              hour: "2-digit",
              minute: "2-digit",
            })}
          </time>
        </div>
        {progress !== null && item.mediaKind !== "live" && (
          <div
            role="progressbar"
            aria-label={`${item.title} progress`}
            aria-valuenow={item.completed ? 100 : Math.round(progress)}
            aria-valuemin={0}
            aria-valuemax={100}
            className="mt-2 h-1 max-w-48 overflow-hidden rounded bg-white/10"
          >
            <div
              className="h-full bg-primary"
              style={{ width: `${item.completed ? 100 : progress}%` }}
            />
          </div>
        )}
      </div>
      {item.mediaId === null ? (
        <span className="text-xs text-muted-foreground">Unavailable in this playlist</span>
      ) : (
        <Button
          variant="outline"
          size="sm"
          aria-label={`${action} ${item.title} ${subtitle}`}
          onClick={() => {
            if (item.mediaId === null) return;
            playMedia({
              kind: item.mediaKind,
              id: item.mediaId,
              title: item.title,
              subtitle,
              posterUrl: item.posterUrl,
              startSec:
                item.mediaKind === "live" ? undefined : item.completed ? 0 : item.positionSec,
            });
          }}
        >
          <PlayIcon size={14} strokeWidth={1.5} aria-hidden />
          {action}
        </Button>
      )}
    </li>
  );
}
