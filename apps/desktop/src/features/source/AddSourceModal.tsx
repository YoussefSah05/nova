import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ImportProgress } from "@/types";
import { cn } from "@/lib/utils";
import { motion, EASE_ENTER } from "@/lib/motion";
import { Button } from "@/components/ui/button";

interface AddSourceModalProps {
  profileId: string;
  onDone: () => void;
  onCancel: () => void;
}

type Phase = "form" | "importing" | "ready";

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
    const sourceExists = activeSourceIdRef.current !== null;
    activeSourceIdRef.current = null;
    if (sourceExists) onDone();
    else onCancel();
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
        if (event.payload.stage === "queued") clearWatchdog();
        else armWatchdog();
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
            setPhase("ready");
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
  const title = phase === "ready" ? "Ready to browse" : "Adding library";
  const stage = progress?.stage ?? "connecting";
  const stages = [
    {
      title: "Connect to provider",
      complete: stage !== "connecting" && stage !== "queued",
      active: stage === "connecting" || stage === "queued",
    },
    {
      title: "Read library entries",
      complete: stage === "persisting" || phase === "ready",
      active: stage === "receiving",
    },
    {
      title: "Save and publish library",
      complete: phase === "ready",
      active: stage === "persisting",
    },
  ];

  // Connecting sub-message: explain the wait after 10s so users know the
  // server is generating the playlist, not that the app is broken.
  const connectingDetail =
    stage === "queued"
      ? "Waiting for available resources…"
      : elapsedSecs === 0
        ? "Fetching playlist…"
        : elapsedSecs < 10
          ? `Waiting for server… ${elapsedSecs}s`
          : `Server is generating your playlist… ${elapsedSecs}s`;

  return (
    <motion.div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm"
      onClick={(e) => e.target === e.currentTarget && phase === "form" && onCancel()}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.15 }}
    >
      <motion.div
        className="nova-glass flex w-96 flex-col gap-5 rounded-2xl p-6"
        initial={{ opacity: 0, scale: 0.98, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.2, ease: EASE_ENTER }}
      >
        {phase === "form" ? (
          <form onSubmit={handleSubmit} className="flex flex-col gap-5">
            <h2 className="font-display text-lg font-semibold text-white">Add Playlist</h2>

            <div className="flex flex-col gap-1.5">
              <label
                htmlFor="source-name"
                className="text-xs font-medium text-white/50 uppercase tracking-wider"
              >
                Name (optional)
              </label>
              <input
                id="source-name"
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="My IPTV"
                className="rounded-lg border border-input bg-white/[0.04] px-3 py-2 text-sm text-white placeholder-white/30 outline-none transition-colors focus:ring-2 focus:ring-ring"
              />
            </div>

            <div className="flex flex-col gap-1.5">
              <label
                htmlFor="source-url"
                className="text-xs font-medium text-white/50 uppercase tracking-wider"
              >
                M3U URL
              </label>
              <input
                id="source-url"
                autoFocus
                type="url"
                value={url}
                onChange={(e) => {
                  setUrl(e.target.value);
                  setError(null);
                }}
                placeholder="http://provider.com/playlist.m3u"
                className={cn(
                  "rounded-lg border border-input bg-white/[0.04] px-3 py-2 text-sm text-white placeholder-white/30",
                  "outline-none transition-colors focus:ring-2 focus:ring-ring",
                  error && "border-red-500"
                )}
              />
              {error && <p className="text-xs text-red-400">{error}</p>}
            </div>

            <div className="flex gap-3">
              <Button type="button" variant="secondary" onClick={onCancel} className="flex-1">
                Cancel
              </Button>
              <Button type="submit" className="flex-1">
                Import
              </Button>
            </div>
          </form>
        ) : (
          <div className="flex flex-col gap-5">
            <h2 className="font-display text-lg font-semibold text-white">{title}</h2>

            <ol className="flex flex-col gap-3" aria-label="Import stages">
              {stages.map((step) => (
                <li
                  key={step.title}
                  className="flex items-center gap-3 text-sm"
                  aria-current={step.active && phase !== "ready" ? "step" : undefined}
                >
                  <span
                    className={cn(
                      "flex h-6 w-6 items-center justify-center rounded-full border text-xs",
                      step.complete
                        ? "border-primary bg-primary/10 text-primary"
                        : step.active
                          ? "border-primary text-primary"
                          : "border-border text-muted-foreground"
                    )}
                    aria-hidden
                  >
                    {step.complete ? "✓" : step.active ? "•" : "–"}
                  </span>
                  <span
                    className={
                      step.active || step.complete ? "text-foreground" : "text-muted-foreground"
                    }
                  >
                    {step.title}
                  </span>
                  <span className="sr-only">
                    {step.complete ? "Complete" : step.active ? "In progress" : "Waiting"}
                  </span>
                </li>
              ))}
            </ol>
            <div className="flex flex-col gap-2" role="status" aria-live="polite">
              {phase !== "ready" && (
                <div
                  role="progressbar"
                  aria-valuetext="Total entry count unknown"
                  aria-label="Import in progress; total size unknown"
                  className="h-1 w-full animate-pulse rounded-full bg-primary/40 motion-reduce:animate-none"
                />
              )}
              <p className="text-xs text-muted-foreground">
                {parsed === 0
                  ? connectingDetail
                  : `${parsed.toLocaleString()} entries read · ${(progress?.inserted ?? 0).toLocaleString()} saved`}
              </p>
              <p className="text-xs text-muted-foreground">
                {phase === "ready"
                  ? "Your library is available now. Optional metadata improvements do not block browsing."
                  : "Library size is provided by the source. Browsing becomes available after publication."}
              </p>
            </div>

            {/* Per-type metrics — visible once entries start arriving */}
            {parsed > 0 && (
              <div className="grid grid-cols-3 divide-x divide-white/10 rounded-xl bg-white/[0.04] py-3">
                <MetricPill label="Live" value={progress?.liveCount ?? 0} color="text-primary" />
                <MetricPill label="Movies" value={progress?.movieCount ?? 0} color="text-primary" />
                <MetricPill
                  label="Series"
                  value={progress?.seriesCount ?? 0}
                  color="text-primary"
                />
              </div>
            )}

            {progress?.warning && <p className="text-xs text-amber-300">{progress.warning}</p>}

            <Button
              type="button"
              variant={phase === "ready" ? "default" : "secondary"}
              onClick={phase === "ready" ? onDone : handleCancel}
            >
              {phase === "ready" ? "Browse library" : "Continue in background"}
            </Button>
          </div>
        )}
      </motion.div>
    </motion.div>
  );
}
