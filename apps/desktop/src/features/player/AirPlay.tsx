//! AirPlay UI components: the control-bar button slot the native route picker
//! overlays, and the "Playing on AirPlay" placeholder shown while engaged.
//! The handoff logic lives in `useAirPlay`.

import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Airplay, Pause, Play, SkipBack, SkipForward } from "lucide-react";

import { cn } from "@/lib/utils";
import type { AirPlayStatus } from "./useAirPlay";

/// Control-bar slot the native `AVRoutePickerView` overlays. We render a faint
/// AirPlay glyph as a visual anchor; the native picker sits on top and handles
/// the actual click + system route sheet.
export function AirPlayButton({ slotRef }: { slotRef: React.RefObject<HTMLDivElement | null> }) {
  return (
    <div
      ref={slotRef}
      data-controls
      aria-label="AirPlay"
      title="AirPlay"
      className="flex h-9 w-9 items-center justify-center rounded-full text-white/85"
    >
      <Airplay className="h-5 w-5" />
    </div>
  );
}

function fmt(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const s = Math.floor(seconds % 60);
  const m = Math.floor((seconds / 60) % 60);
  const h = Math.floor(seconds / 3600);
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/// "Playing on AirPlay" placeholder shown over the (paused, blanked) mpv
/// surface while engaged. Transport drives the `airplay_*` commands and reads
/// position/duration from the polled status.
export function AirPlayOverlay({
  title,
  isLive,
  status,
  onStop,
}: {
  title: string;
  isLive: boolean;
  status: AirPlayStatus | null;
  onStop: () => void;
}) {
  const position = status?.positionSec ?? 0;
  const duration = status?.durationSec ?? null;
  const paused = status?.paused ?? false;
  const pct = duration && duration > 0 ? Math.min(100, (position / duration) * 100) : 0;

  const seekTo = useCallback((sec: number) => {
    void invoke("airplay_seek_absolute", { positionSec: Math.max(0, sec) }).catch(() => undefined);
  }, []);

  return (
    <div className="absolute inset-0 z-30 flex flex-col items-center justify-center gap-6 bg-black text-white">
      <Airplay className="h-16 w-16 text-white/80" />
      <div className="text-center">
        <p className="text-sm uppercase tracking-wide text-white/50">Playing on AirPlay</p>
        <p className="mt-1 max-w-md truncate px-6 text-lg font-medium">{title}</p>
      </div>

      <div className="flex items-center gap-6">
        {!isLive && (
          <button
            onClick={() => seekTo(position - 10)}
            aria-label="Back 10 seconds"
            className="flex h-11 w-11 items-center justify-center rounded-full text-white/85 transition-all hover:bg-white/15 hover:text-white active:scale-95"
          >
            <SkipBack className="h-6 w-6" />
          </button>
        )}
        <button
          onClick={() => void invoke("airplay_toggle_pause").catch(() => undefined)}
          aria-label={paused ? "Play" : "Pause"}
          className="flex h-16 w-16 items-center justify-center rounded-full bg-white/15 text-white backdrop-blur-md transition-all hover:bg-white/25 hover:scale-105 active:scale-95"
        >
          {paused ? <Play className="h-8 w-8 fill-current ml-1" /> : <Pause className="h-8 w-8" />}
        </button>
        {!isLive && (
          <button
            onClick={() => seekTo(position + 10)}
            aria-label="Forward 10 seconds"
            className="flex h-11 w-11 items-center justify-center rounded-full text-white/85 transition-all hover:bg-white/15 hover:text-white active:scale-95"
          >
            <SkipForward className="h-6 w-6" />
          </button>
        )}
      </div>

      {!isLive && duration && duration > 0 && (
        <div className="flex w-full max-w-lg items-center gap-3 px-8 text-xs tabular-nums text-white/70">
          <span>{fmt(position)}</span>
          <div className="relative h-1 flex-1 overflow-hidden rounded-full bg-white/20">
            <div
              className={cn("absolute inset-y-0 left-0 rounded-full bg-white")}
              style={{ width: `${pct}%` }}
            />
          </div>
          <span>{fmt(duration)}</span>
        </div>
      )}

      <button
        onClick={onStop}
        className="mt-2 rounded-full bg-white/10 px-5 py-2 text-sm font-medium transition-colors hover:bg-white/20"
      >
        Stop AirPlay
      </button>
    </div>
  );
}
