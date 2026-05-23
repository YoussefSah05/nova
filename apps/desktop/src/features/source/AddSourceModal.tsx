import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ImportProgress } from "@/types";
import { cn } from "@/lib/utils";

interface AddSourceModalProps {
  profileId: string;
  onDone: () => void;
  onCancel: () => void;
}

type Phase = "form" | "importing";

const ESTIMATED_TOTAL = 50_000;
const IMPORT_STALL_MS = 75_000;

function errorMessage(err: unknown): string {
  if (err instanceof Error) {
    return err.message;
  }
  if (typeof err === "object" && err !== null && "message" in err) {
    const message = (err as { message?: unknown }).message;
    if (typeof message === "string") {
      return message;
    }
  }
  return String(err);
}

function MetricPill({ label, value, color }: { label: string; value: number; color: string }) {
  return (
    <div className="flex flex-col items-center gap-0.5">
      <span className={cn("text-base font-semibold tabular-nums", color)}>
        {value.toLocaleString()}
      </span>
      <span className="text-[10px] font-medium text-white/40 uppercase tracking-widest">
        {label}
      </span>
    </div>
  );
}

export function AddSourceModal({ profileId, onDone, onCancel }: AddSourceModalProps) {
  const [phase, setPhase] = useState<Phase>("form");
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<ImportProgress | null>(null);
  const [elapsedSecs, setElapsedSecs] = useState(0);
  const unlistenRef = useRef<(() => void) | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const watchdogRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeSourceIdRef = useRef<number | null>(null);

  // Elapsed-time counter — runs while connecting (parsed === 0)
  useEffect(() => {
    if (phase !== "importing") {
      setElapsedSecs(0);
      if (timerRef.current) clearInterval(timerRef.current);
      return;
    }
    timerRef.current = setInterval(() => setElapsedSecs((s) => s + 1), 1000);
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [phase]);

  // Stop the timer once the first entry arrives
  useEffect(() => {
    if ((progress?.parsed ?? 0) > 0 && timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
  }, [progress?.parsed]);

  // Cleanup listener on unmount
  useEffect(() => {
    return () => {
      unlistenRef.current?.();
      if (timerRef.current) clearInterval(timerRef.current);
      if (watchdogRef.current) clearTimeout(watchdogRef.current);
    };
  }, []);

  function clearWatchdog() {
    if (watchdogRef.current) {
      clearTimeout(watchdogRef.current);
      watchdogRef.current = null;
    }
  }

  function armWatchdog() {
    clearWatchdog();
    watchdogRef.current = setTimeout(() => {
      unlistenRef.current?.();
      unlistenRef.current = null;
      setError("Import stalled. Check the playlist URL or try again.");
      setPhase("form");
    }, IMPORT_STALL_MS);
  }

  function handleCancel() {
    clearWatchdog();
    unlistenRef.current?.();
    unlistenRef.current = null;
    activeSourceIdRef.current = null;
    onCancel();
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const trimUrl = url.trim();
    if (!trimUrl) {
      setError("URL is required.");
      return;
    }
    let parsedUrl: URL;
    try {
      parsedUrl = new URL(trimUrl);
    } catch {
      setError("Enter a valid playlist URL.");
      return;
    }
    const trimName = name.trim() || parsedUrl.hostname;
    setError(null);
    setProgress(null);
    setElapsedSecs(0);
    activeSourceIdRef.current = null;
    setPhase("importing");

    let unlisten: (() => void) | null = null;
    try {
      unlisten = await listen<ImportProgress>("playlist:import:progress", (event) => {
        if (
          activeSourceIdRef.current !== null &&
          event.payload.sourceId !== activeSourceIdRef.current
        ) {
          return;
        }
        armWatchdog();
        setProgress(event.payload);
        if (event.payload.done) {
          clearWatchdog();
          unlisten?.();
          unlistenRef.current = null;
          const completedSourceId = activeSourceIdRef.current;
          activeSourceIdRef.current = null;
          if (event.payload.error) {
            setError(event.payload.error);
            setPhase("form");
          } else {
            // Kick off EPG refresh for the source we just imported. Runs in
            // the background — no need to await before closing the modal. The
            // worker is a no-op when no EPG URLs are registered.
            if (completedSourceId !== null) {
              void invoke("epg_refresh", {
                profileId,
                sourceId: completedSourceId,
              }).catch(() => {
                // Silent — EPG is decorative; the import itself succeeded.
              });
            }
            onDone();
          }
        }
      });
      unlistenRef.current = unlisten;
      armWatchdog();

      const sourceId = await invoke<number>("add_source", {
        profileId,
        name: trimName,
        url: trimUrl,
      });
      activeSourceIdRef.current = sourceId;
    } catch (err) {
      clearWatchdog();
      unlisten?.();
      unlistenRef.current = null;
      activeSourceIdRef.current = null;
      setError(errorMessage(err));
      setPhase("form");
    }
  }

  const parsed = progress?.parsed ?? 0;
  const fillPct = parsed === 0 ? 0 : Math.min(100, (parsed / ESTIMATED_TOTAL) * 100);
  const title =
    progress?.stage === "persisting"
      ? "Saving…"
      : progress?.stage === "failed"
        ? "Import failed"
        : parsed > 0
          ? "Importing…"
          : "Connecting…";

  // Connecting sub-message: explain the wait after 10s so users know the
  // server is generating the playlist, not that the app is broken.
  const connectingDetail =
    elapsedSecs === 0
      ? "Fetching playlist…"
      : elapsedSecs < 10
        ? `Waiting for server… ${elapsedSecs}s`
        : `Server is generating your playlist… ${elapsedSecs}s`;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm"
      onClick={(e) => e.target === e.currentTarget && phase === "form" && onCancel()}
    >
      <div className="w-96 rounded-2xl bg-neutral-900 p-6 flex flex-col gap-5 shadow-2xl">
        {phase === "form" ? (
          <form onSubmit={handleSubmit} className="flex flex-col gap-5">
            <h2 className="text-lg font-semibold text-white">Add Playlist</h2>

            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-white/50 uppercase tracking-wider">
                Name (optional)
              </label>
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="My IPTV"
                className="rounded-lg bg-neutral-800 px-3 py-2 text-sm text-white placeholder-white/30 border border-transparent outline-none focus:border-indigo-500 transition-colors"
              />
            </div>

            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-white/50 uppercase tracking-wider">
                M3U URL
              </label>
              <input
                autoFocus
                type="url"
                value={url}
                onChange={(e) => {
                  setUrl(e.target.value);
                  setError(null);
                }}
                placeholder="http://provider.com/playlist.m3u"
                className={cn(
                  "rounded-lg bg-neutral-800 px-3 py-2 text-sm text-white placeholder-white/30",
                  "border border-transparent outline-none focus:border-indigo-500 transition-colors",
                  error && "border-red-500"
                )}
              />
              {error && <p className="text-xs text-red-400">{error}</p>}
            </div>

            <div className="flex gap-3">
              <button
                type="button"
                onClick={onCancel}
                className="flex-1 rounded-lg bg-neutral-800 py-2 text-sm font-medium text-white/70 hover:text-white transition-colors"
              >
                Cancel
              </button>
              <button
                type="submit"
                className="flex-1 rounded-lg bg-indigo-600 py-2 text-sm font-semibold text-white hover:bg-indigo-500 transition-colors"
              >
                Import
              </button>
            </div>
          </form>
        ) : (
          <div className="flex flex-col gap-5">
            <h2 className="text-lg font-semibold text-white">{title}</h2>

            {/* Progress bar */}
            <div className="flex flex-col gap-2">
              <div className="h-2 w-full rounded-full bg-neutral-800 overflow-hidden">
                {parsed === 0 ? (
                  <div className="h-full w-full bg-indigo-500/60 rounded-full animate-pulse" />
                ) : (
                  <div
                    className="h-full bg-indigo-500 rounded-full transition-all duration-300 ease-out"
                    style={{ width: `${fillPct}%` }}
                  />
                )}
              </div>
              <div className="flex justify-between text-xs text-white/40">
                {parsed === 0 ? (
                  <span>{connectingDetail}</span>
                ) : (
                  <>
                    <span>{parsed.toLocaleString()} parsed</span>
                    <span>{(progress?.inserted ?? 0).toLocaleString()} saved</span>
                  </>
                )}
              </div>
            </div>

            {/* Per-type metrics — visible once entries start arriving */}
            {parsed > 0 && (
              <div className="grid grid-cols-3 divide-x divide-white/10 rounded-xl bg-neutral-800 py-3">
                <MetricPill label="Live" value={progress?.liveCount ?? 0} color="text-sky-400" />
                <MetricPill
                  label="Movies"
                  value={progress?.movieCount ?? 0}
                  color="text-violet-400"
                />
                <MetricPill
                  label="Series"
                  value={progress?.seriesCount ?? 0}
                  color="text-emerald-400"
                />
              </div>
            )}

            {progress?.warning && <p className="text-xs text-amber-300">{progress.warning}</p>}

            {/* Cancel button — always visible while a slow server is being handled */}
            <button
              type="button"
              onClick={handleCancel}
              className="rounded-lg bg-neutral-800 py-2 text-sm font-medium text-white/50 hover:text-white transition-colors"
            >
              Cancel
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
