import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Hls from "hls.js";
import { XIcon } from "lucide-react";

export type PlayMediaKind = "live" | "vod" | "episode";

export interface PlayMedia {
  kind: PlayMediaKind;
  /// `channels.id` / `vod_items.id` / `series_episodes.id`.
  id: number;
  /// Upstream stream URL — proxied through the local HLS proxy before playback.
  streamUrl: string;
  /// Display title in the player overlay.
  title: string;
  subtitle?: string | null;
  posterUrl?: string | null;
}

interface PlayerModalProps {
  profileId: string;
  media: PlayMedia;
  onClose: () => void;
}

interface ResumeInfo {
  positionSec: number;
  durationSec: number | null;
  completed: boolean;
}

/// How often to push playback position to the backend. Watch history rows
/// are keyed on (kind, id) so updates are idempotent.
const PROGRESS_INTERVAL_MS = 5_000;

/// A programme is considered "watched" past this fraction of duration.
/// Continue Watching hides completed entries.
const COMPLETED_THRESHOLD = 0.95;

export function PlayerModal({ profileId, media, onClose }: PlayerModalProps) {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const hlsRef = useRef<Hls | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);

  // Build the proxied URL once we know the proxy port. The Phase 5 proxy
  // is passthrough; future phases will rewrite manifests for captions but
  // the URL shape stays the same.
  const [proxiedSrc, setProxiedSrc] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const port = await invoke<number>("get_proxy_port");
        if (cancelled) return;
        const src = `http://127.0.0.1:${port}/proxy?url=${encodeURIComponent(media.streamUrl)}`;
        setProxiedSrc(src);
      } catch (e) {
        if (!cancelled) setError(errorMessage(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [media.streamUrl]);

  // Attach hls.js / native source whenever the proxied src resolves.
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !proxiedSrc) return;

    setError(null);
    setReady(false);

    const isHls = media.streamUrl.endsWith(".m3u8") || media.streamUrl.includes(".m3u8?");

    // Safari (and WebKit-based Tauri webviews on macOS) play HLS natively.
    // Fall back to hls.js only when the browser can't, which keeps memory +
    // CPU lower on macOS.
    if (isHls && !video.canPlayType("application/vnd.apple.mpegurl") && Hls.isSupported()) {
      const hls = new Hls({
        // Defaults are sensible; keep them. The aggressive low-latency mode
        // is opt-in per stream and many IPTV providers aren't compatible.
      });
      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (data.fatal) {
          setError(`Stream error: ${data.details}`);
        }
      });
      hls.loadSource(proxiedSrc);
      hls.attachMedia(video);
      hlsRef.current = hls;
    } else {
      // Native: HLS via WebKit, or MP4 / MKV / TS by direct URL.
      video.src = proxiedSrc;
    }

    return () => {
      if (hlsRef.current) {
        hlsRef.current.destroy();
        hlsRef.current = null;
      }
      video.removeAttribute("src");
      video.load();
    };
  }, [proxiedSrc, media.streamUrl]);

  // Seek to the saved resume position once the player can play. Live streams
  // get media_kind='live' for tracking but no seek (no duration).
  useEffect(() => {
    if (!ready) return;
    const video = videoRef.current;
    if (!video) return;

    let cancelled = false;
    void (async () => {
      try {
        const info = await invoke<ResumeInfo | null>("get_resume_position", {
          profileId,
          mediaKind: media.kind,
          mediaId: media.id,
        });
        if (cancelled || !info || info.completed) return;
        // Only seek for finite-duration content; live streams have NaN duration.
        if (media.kind !== "live" && info.positionSec > 5) {
          video.currentTime = info.positionSec;
        }
      } catch {
        /* resume is best-effort */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [ready, profileId, media.kind, media.id]);

  // Periodic watch-history writes. The backend upserts on (kind, id) so we
  // can fire-and-forget without sequencing.
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !ready) return;

    const tick = () => {
      if (video.paused || !Number.isFinite(video.currentTime)) return;
      const duration = Number.isFinite(video.duration) ? video.duration : null;
      const completed = duration !== null && video.currentTime / duration >= COMPLETED_THRESHOLD;
      void invoke("record_watch_progress", {
        profileId,
        mediaKind: media.kind,
        mediaId: media.id,
        positionSec: video.currentTime,
        durationSec: duration,
        completed,
      }).catch(() => {
        /* progress writes are best-effort */
      });
    };

    const interval = setInterval(tick, PROGRESS_INTERVAL_MS);
    // Also flush on pause / close so the resume position is accurate when the
    // user steps away mid-programme.
    const flush = () => tick();
    video.addEventListener("pause", flush);
    return () => {
      clearInterval(interval);
      video.removeEventListener("pause", flush);
      flush();
    };
  }, [ready, profileId, media.kind, media.id]);

  // Esc to close.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // Allow Esc to exit fullscreen first — only close the modal when
        // we're back in windowed mode.
        if (!document.fullscreenElement) onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-black">
      <div className="flex items-center justify-between px-5 py-3 text-white">
        <div className="min-w-0">
          <p className="truncate text-base font-semibold">{media.title}</p>
          {media.subtitle && <p className="truncate text-xs text-white/55">{media.subtitle}</p>}
        </div>
        <button
          onClick={onClose}
          className="rounded-full bg-white/10 p-2 text-white/80 transition-colors hover:bg-white/20 hover:text-white"
          aria-label="Close player"
        >
          <XIcon className="h-4 w-4" />
        </button>
      </div>

      <div className="relative flex-1">
        {error && (
          <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-2 bg-black/80 text-white/75">
            <p className="text-sm">{error}</p>
            <button
              onClick={onClose}
              className="rounded-lg bg-white/10 px-3 py-1.5 text-xs font-medium hover:bg-white/15"
            >
              Close
            </button>
          </div>
        )}
        <video
          ref={videoRef}
          poster={media.posterUrl ?? undefined}
          controls
          autoPlay
          playsInline
          onCanPlay={() => setReady(true)}
          onError={() => setError("Could not play this stream.")}
          className="h-full w-full bg-black"
        />
      </div>
    </div>
  );
}

function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "object" && err !== null && "message" in err) {
    const m = (err as { message?: unknown }).message;
    if (typeof m === "string") return m;
  }
  return String(err);
}
