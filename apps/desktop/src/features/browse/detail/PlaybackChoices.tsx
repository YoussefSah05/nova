import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, Headphones, Play } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { AudioTrackInfo } from "@/types";
import type { PlaybackLaunchOptions } from "@/features/player/playbackLaunch";
import { useCaptionPrepare, type CaptionPrepareTarget } from "@/features/player/useCaptionPrepare";
import { useProfileSettings } from "@/features/settings/useProfileSettings";
import type { useAudioTrackChoice } from "./useAudioTrackChoice";

function trackLabel(track: AudioTrackInfo, tracks: AudioTrackInfo[]) {
  const language = track.language?.toUpperCase() ?? "Unknown";
  return !track.language || tracks.filter((other) => other.language === track.language).length > 1
    ? `${language} · Track ${track.audioIndex + 1}`
    : language;
}

export function AudioTrackPicker({
  choice,
  resolving = false,
  sourceError = false,
  onRetrySource,
}: {
  choice: ReturnType<typeof useAudioTrackChoice>;
  resolving?: boolean;
  sourceError?: boolean;
  onRetrySource?: () => void;
}) {
  const tracks = choice.query.data?.tracks ?? [];
  const failed = sourceError || choice.query.isError;
  const pending = resolving || choice.query.isFetching;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="secondary" className="gap-2">
          <Headphones className="h-4 w-4" strokeWidth={1.5} aria-hidden />
          Audio: {choice.selected ? trackLabel(choice.selected, tracks) : "Auto"}
          <ChevronDown className="h-4 w-4 opacity-70" strokeWidth={1.5} aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-h-72 w-64 overflow-auto">
        <DropdownMenuLabel>Selected source audio</DropdownMenuLabel>
        <DropdownMenuCheckItem checked={!choice.selected} onSelect={() => choice.choose(null)}>
          Auto
        </DropdownMenuCheckItem>
        {pending ? (
          <p role="status" className="px-2 py-2 text-xs text-muted-foreground">
            Checking audio…
          </p>
        ) : failed ? (
          <DropdownMenuItem
            onSelect={() => {
              if (sourceError) onRetrySource?.();
              else void choice.query.refetch();
            }}
          >
            Audio unavailable · Retry
          </DropdownMenuItem>
        ) : tracks.length ? (
          tracks.map((track) => (
            <DropdownMenuCheckItem
              key={track.audioIndex}
              checked={choice.selected?.audioIndex === track.audioIndex}
              onSelect={() => choice.choose(track.audioIndex)}
            >
              {trackLabel(track, tracks)}
            </DropdownMenuCheckItem>
          ))
        ) : (
          <p className="px-2 py-2 text-xs text-muted-foreground">No audio tracks reported</p>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Prepares captions for the title's Play target (see `useCaptionPrepare`). */
function CaptionPrepare({
  target,
  audioTrack,
  menuOpened,
}: {
  target: CaptionPrepareTarget;
  audioTrack?: PlaybackLaunchOptions["audioTrack"];
  menuOpened: boolean;
}) {
  const profile = useProfileSettings(target.profileId);
  const captionsByDefault = profile.loaded && profile.settings.defaultCaptionMode !== "off";
  const track =
    audioTrack && audioTrack.mediaKind === target.mediaKind && audioTrack.mediaId === target.mediaId
      ? { index: audioTrack.index, language: audioTrack.language }
      : undefined;
  useCaptionPrepare(target, track, menuOpened || captionsByDefault);
  return null;
}

export function PlayWithCaptions({
  label = "Play",
  onPlay,
  audioTrack,
  disabled = false,
  size = "default",
  prepare,
}: {
  label?: string;
  onPlay: (options: PlaybackLaunchOptions) => void;
  audioTrack?: PlaybackLaunchOptions["audioTrack"];
  disabled?: boolean;
  size?: "default" | "lg" | "sm";
  /** The media Play will open: lets captions start before playback does. */
  prepare?: CaptionPrepareTarget | null;
}) {
  const [menuOpened, setMenuOpened] = useState(false);
  const start = (
    captionMode: "off" | "source" | "translation",
    translationTarget?: "en" | "fr" | "ar"
  ) =>
    onPlay({
      captionMode,
      ...(translationTarget ? { translationTarget } : {}),
      ...(audioTrack ? { audioTrack } : {}),
    });
  return (
    <div className="inline-flex items-center">
      {prepare && (
        <CaptionPrepare target={prepare} audioTrack={audioTrack} menuOpened={menuOpened} />
      )}
      <Button
        size={size}
        disabled={disabled}
        className="gap-2 rounded-r-none"
        onClick={() => start("off")}
      >
        <Play className="h-4 w-4 fill-current" strokeWidth={1.5} aria-hidden /> {label}
      </Button>
      <DropdownMenu
        onOpenChange={(open) => {
          if (!open) return;
          setMenuOpened(true);
          // Start the speech model while the user picks a caption option.
          void Promise.resolve()
            .then(() => invoke("prewarm_captions"))
            .catch(() => undefined);
        }}
      >
        <DropdownMenuTrigger asChild>
          <Button
            size={size}
            disabled={disabled}
            aria-label="Playback options"
            className="rounded-l-none border-l border-primary-foreground/20 px-2"
          >
            <ChevronDown className="h-4 w-4" strokeWidth={1.5} aria-hidden />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-64">
          <DropdownMenuLabel>Start playback</DropdownMenuLabel>
          <DropdownMenuItem onSelect={() => start("off")}>Play normally</DropdownMenuItem>
          <DropdownMenuItem onSelect={() => start("source")}>
            Play with transcription
          </DropdownMenuItem>
          <DropdownMenuLabel>Translated captions</DropdownMenuLabel>
          {(["en", "fr", "ar"] as const).map((target) => (
            <DropdownMenuItem key={target} onSelect={() => start("translation", target)}>
              Translated captions · {target.toUpperCase()}
            </DropdownMenuItem>
          ))}
          <p className="px-2 py-2 text-xs text-muted-foreground">
            Video starts immediately. Captions prepare locally.
          </p>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
