import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Hls, { type ErrorData } from "hls.js";
import {
  AudioLines,
  Clapperboard,
  ScrollText,
  Gauge,
  Maximize,
  MonitorCog,
  Minimize,
  Pause,
  PictureInPicture,
  Play,
  Radio,
  SkipBack,
  SkipForward,
  Volume1,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";

import { cn } from "@/lib/utils";
import { Slider } from "@/components/ui/slider";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuCheckItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { ChannelGroup, SeriesEpisode } from "@/types";
import { fetchChannelsInGroup, type BrowseItem } from "@/features/browse/queries";
import { useProfileSettings } from "@/features/settings/useProfileSettings";

import { CaptionOverlay, type CaptionMode, type Cue } from "./CaptionOverlay";
import { CaptionButton, type TargetLanguage } from "./CaptionPicker";
import { TranscriptPanel } from "./TranscriptPanel";
import { CaptionStyleButton } from "./CaptionStylePanel";
import { CinemaEpisodeBrowser } from "./CinemaEpisodeBrowser";
import { episodeToMedia } from "./episodeMedia";
import { sourceCaptionNote, useSourceCaptions } from "./useSourceCaptions";
import { translatedCaptionNote, useTranslatedCaptions } from "./useTranslatedCaptions";
import { useCaptionStyle } from "./useCaptionStyle";
import { useAirPlay } from "./useAirPlay";
import { AirPlayButton, AirPlayOverlay } from "./AirPlay";

export type PlayMediaKind = "live" | "vod" | "episode";

export interface PlayMedia {
  kind: PlayMediaKind;
  /// `channels.id` / `vod_items.id` / `series_episodes.id`.
  id: number;
  /// Display title in the player overlay.
  title: string;
  subtitle?: string | null;
  posterUrl?: string | null;
  /// Live context for cinema mode (layer 1 = brand bundle, layer 2 = same type).
  channelGroup?: string | null;
  category?: string | null;
  region?: string | null;
}

interface PlayerModalProps {
  profileId: string;
  media: PlayMedia;
  onClose: () => void;
  /// Active source — enables the cinema-mode live channel strip.
  sourceId?: number | null;
  /// Switch the player to other media (used by the cinema live channel strip).
  onPlayMedia?: (media: PlayMedia) => void;
}

interface ResumeInfo {
  positionSec: number;
  durationSec: number | null;
  completed: boolean;
}

interface ResolvedStream {
  /// URL the player should consume. For `kind === "hls"` it's a master.m3u8
  /// served by the local proxy; for `kind === "file"` it's a direct-MP4
  /// passthrough URL with byte-range support.
  url: string;
  kind: "hls" | "file";
  route: PlaybackRoute;
  /// Session token; passed to `stop_playback` on unmount so the proxy can
  /// release any ffmpeg remux subprocess.
  token: string;
  supportsSeek: boolean;
  durationSec: number | null;
  sourceLang: string | null;
  container: string | null;
  videoCodec: string | null;
  audioCodec: string | null;
  width: number | null;
  height: number | null;
  fallbackReason: string | null;
  captionState: CaptionState;
}

interface NativeOpenResult {
  token: string;
  engine: "mpv";
  embedded: boolean;
}

type NativePlayerPhase = "opening" | "buffering" | "playing" | "paused" | "ended" | "error";

interface NativeTrack {
  id: number;
  kind: "audio" | "video" | "sub" | string;
  title: string | null;
  lang: string | null;
  selected: boolean;
}

interface NativePlayerState {
  token: string | null;
  engine: "mpv";
  phase: NativePlayerPhase;
  positionSec: number;
  durationSec: number | null;
  volume: number;
  muted: boolean;
  seekable: boolean;
  speed: number;
  audioTrackId: number | null;
  subTrackId: number | null;
  videoTrackId: number | null;
  tracks: NativeTrack[];
  error: string | null;
}

type PlaybackRoute = "hls_passthrough" | "file_passthrough" | "remux_hls" | "transcode_hls";
type PlaybackRouteOverride = "remux_hls" | "transcode_hls";

type CaptionAssetState = "unavailable" | "pending" | "ready";

interface CaptionState {
  source: CaptionAssetState;
  translation: CaptionAssetState;
}

type LoadingPhase = "resolving" | "connecting" | "manifest" | "buffering" | "ready";

const PROGRESS_INTERVAL_MS = 5_000;
const COMPLETED_THRESHOLD = 0.95;
const CONTROLS_HIDE_DELAY_MS = 2_800;
const DOUBLE_TAP_WINDOW_MS = 280;
const SKIP_SECONDS = 10;
const ARROW_SECONDS = 5;
const VOLUME_STEP = 0.1;
const BUFFER_TARGET_SECONDS = 8;
const USE_MPV_ENGINE = true;
const SPEED_OPTIONS = [0.5, 0.75, 1, 1.25, 1.5, 2];

// hls.js tuning notes:
//  * `lowLatencyMode: false` - IPTV providers rarely ship LL-HLS playlists
//    and enabling it on a non-LL stream slows startup.
//  * `backBufferLength: 15` - keep a short tail for quick re-seek without
//    pinning the entire session in memory.
//  * `*MaxRetry` is intentionally generous since IPTV CDNs flap.
//  * `abrEwmaDefaultEstimate` seeds the bitrate estimator at ~2.5 Mbps so
//    the very first segment isn't forced to the lowest rendition.
const HLS_CONFIG = {
  lowLatencyMode: false,
  enableWorker: true,
  startFragPrefetch: true,
  testBandwidth: false,
  backBufferLength: 15,
  maxBufferLength: 12,
  maxMaxBufferLength: 30,
  liveSyncDurationCount: 2,
  liveMaxLatencyDurationCount: 10,
  manifestLoadingTimeOut: 15_000,
  manifestLoadingMaxRetry: 4,
  levelLoadingTimeOut: 15_000,
  levelLoadingMaxRetry: 4,
  fragLoadingTimeOut: 25_000,
  fragLoadingMaxRetry: 6,
  abrEwmaDefaultEstimate: 2_500_000,
} as const;

type PiPVideoElement = HTMLVideoElement & {
  requestPictureInPicture?: () => Promise<unknown>;
  disablePictureInPicture?: boolean;
};

type PiPDocument = Document & {
  pictureInPictureEnabled?: boolean;
  pictureInPictureElement?: Element | null;
  exitPictureInPicture?: () => Promise<unknown>;
};

export function PlayerModal({
  profileId,
  media,
  onClose,
  sourceId,
  onPlayMedia,
}: PlayerModalProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const videoRegionRef = useRef<HTMLDivElement | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const hlsRef = useRef<Hls | null>(null);

  const isLive = media.kind === "live";

  // -- playback / stream state --------------------------------------------
  const [resolved, setResolved] = useState<ResolvedStream | null>(null);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [playing, setPlaying] = useState(false);
  const [buffering, setBuffering] = useState(true);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [bufferedEnd, setBufferedEnd] = useState(0);
  const [bufferedAheadSec, setBufferedAheadSec] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [cinema, setCinema] = useState(false);
  const [pipActive, setPipActive] = useState(false);
  const [pipSupported, setPipSupported] = useState(false);
  const [loadingPhase, setLoadingPhase] = useState<LoadingPhase>("resolving");
  const [loadingProgress, setLoadingProgress] = useState(8);
  const [forceHlsJs, setForceHlsJs] = useState(false);
  const [nativeToken, setNativeToken] = useState<string | null>(null);
  // Mirrors `nativeToken` so the stale-event guard and the unmount-only close
  // always see the latest token, even during the async open window.
  const nativeTokenRef = useRef<string | null>(null);
  const [nativeEmbedded, setNativeEmbedded] = useState(false);
  // False until the current load paints its first frame. Drives an opaque
  // cover so the previous file's frame can't show during a switch, while still
  // keeping the last frame visible during mid-stream rebuffering.
  const [hasPainted, setHasPainted] = useState(false);
  const mediaKey = `${profileId}:${media.kind}:${media.id}`;
  const [routeOverride, setRouteOverride] = useState<{
    mediaKey: string;
    route: PlaybackRouteOverride;
  } | null>(null);
  const activeRouteOverride = routeOverride?.mediaKey === mediaKey ? routeOverride.route : null;

  // -- transient overlay state (skip indicator, controls visibility) ------
  const [skipIndicator, setSkipIndicator] = useState<{ dir: -1 | 1; key: number } | null>(null);
  const [controlsVisible, setControlsVisible] = useState(true);
  const hideTimerRef = useRef<number | null>(null);
  const lastTapRef = useRef<{ t: number; x: number } | null>(null);

  // -- caption state (Phase 5: inert; Phase 6/7 will populate the cue arrays) --
  const [captionMode, setCaptionMode] = useState<CaptionMode>("off");
  const [captionMenuOpen, setCaptionMenuOpen] = useState(false);
  const [transcriptOpen, setTranscriptOpen] = useState(false);
  const [captionStyleOpen, setCaptionStyleOpen] = useState(false);
  const captionStyleState = useCaptionStyle(profileId);

  // -- track / speed state (driven by mpv:state) -------------------------
  const [tracks, setTracks] = useState<NativeTrack[]>([]);
  const [audioTrackId, setAudioTrackId] = useState<number | null>(null);
  const [videoTrackId, setVideoTrackId] = useState<number | null>(null);
  const [speed, setSpeed] = useState(1);
  const [audioMenuOpen, setAudioMenuOpen] = useState(false);
  const [speedMenuOpen, setSpeedMenuOpen] = useState(false);
  const [qualityMenuOpen, setQualityMenuOpen] = useState(false);
  const audioTracks = tracks.filter((t) => t.kind === "audio");
  const videoTracks = tracks.filter((t) => t.kind === "video");
  const latestPlaybackRef = useRef({ currentTime: 0, duration: 0, playing: false });
  // Mirror the caption selection into a ref so the watch-progress interval
  // (which captures its closure once) always records the current mode.
  const captionModeRef = useRef<CaptionMode>("off");

  useEffect(() => {
    latestPlaybackRef.current = { currentTime, duration, playing };
  }, [currentTime, duration, playing]);

  // Source-language captions (Phase 6): a streaming session transcribes ~30s
  // windows at the playhead and emits cues, rendered through CaptionOverlay.
  // The selected audio track as a 0-based index among audio tracks (ffmpeg
  // `0:a:<index>`) + its language, so captions transcribe the track the viewer
  // is hearing and re-target when they switch it.
  const selectedAudioTrack = useMemo(() => {
    const idx = audioTracks.findIndex((t) => t.id === audioTrackId);
    const i = idx >= 0 ? idx : 0;
    return { index: i, lang: audioTracks[i]?.lang ?? null };
  }, [audioTracks, audioTrackId]);

  // Source captions run for translation/dual too — the translation session
  // consumes the source segments — so the session is active whenever captions
  // are on (the hook ignores live).
  const sourceCaptions = useSourceCaptions(
    profileId,
    { kind: media.kind, id: media.id },
    captionMode !== "off",
    () => latestPlaybackRef.current.currentTime,
    () => latestPlaybackRef.current.duration,
    selectedAudioTrack
  );
  const sourceCues: Cue[] = sourceCaptions.cues;

  useEffect(() => {
    captionModeRef.current = captionMode;
  }, [captionMode]);

  // -- per-profile settings + autoplay-next-episode -----------------------
  const profileSettings = useProfileSettings(profileId);

  // Translated captions (Phase 7): reading language is the persisted profile
  // preference (default English). A translation session translates the source
  // segments at the playhead and emits `translation:cues`.
  const targetLang = profileSettings.settings.preferredSubtitleLang || "en";
  const setTargetLang = useCallback(
    (code: string) => profileSettings.setSettings({ preferredSubtitleLang: code }),
    [profileSettings]
  );
  const [targetLanguages, setTargetLanguages] = useState<TargetLanguage[]>([]);
  useEffect(() => {
    void invoke<TargetLanguage[]>("list_target_languages")
      .then(setTargetLanguages)
      .catch(() => undefined);
  }, []);
  const translatedCaptions = useTranslatedCaptions(
    profileId,
    { kind: media.kind, id: media.id },
    captionMode === "translation" || captionMode === "dual",
    () => latestPlaybackRef.current.currentTime,
    () => latestPlaybackRef.current.duration,
    selectedAudioTrack,
    targetLang
  );
  const translatedCues: Cue[] = translatedCaptions.cues;

  // The next episode to roll into when this one ends (autoplay). Resolved from
  // the sibling list, ordered by (season, episode).
  const [nextEpisode, setNextEpisode] = useState<PlayMedia | null>(null);
  useEffect(() => {
    if (media.kind !== "episode") {
      setNextEpisode(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const eps = await invoke<SeriesEpisode[]>("list_player_episodes", {
          profileId,
          episodeId: media.id,
        });
        if (cancelled) return;
        const sorted = [...eps].sort((a, b) => a.season - b.season || a.episode - b.episode);
        const idx = sorted.findIndex((e) => e.id === media.id);
        const next = idx >= 0 && idx + 1 < sorted.length ? sorted[idx + 1] : null;
        setNextEpisode(next ? episodeToMedia(next, media.posterUrl) : null);
      } catch {
        if (!cancelled) setNextEpisode(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [media.kind, media.id, profileId, media.posterUrl]);

  // Read by the native-state handler (which has empty deps) without re-binding.
  const autoplayRef = useRef<{ enabled: boolean; next: PlayMedia | null }>({
    enabled: true,
    next: null,
  });
  autoplayRef.current = { enabled: profileSettings.settings.autoplayNext, next: nextEpisode };
  const onPlayMediaRef = useRef(onPlayMedia);
  onPlayMediaRef.current = onPlayMedia;
  // Fire autoplay at most once per playthrough.
  const endedHandledRef = useRef(false);
  useEffect(() => {
    endedHandledRef.current = false;
  }, [media.kind, media.id]);

  // Apply the profile's default caption mode once, when the player opens (not on
  // every episode switch, so a mid-session manual toggle isn't overridden).
  const defaultCaptionApplied = useRef(false);
  useEffect(() => {
    if (!profileSettings.loaded || defaultCaptionApplied.current) return;
    defaultCaptionApplied.current = true;
    const def = profileSettings.settings.defaultCaptionMode as CaptionMode;
    if (def !== "off" && media.kind !== "live") setCaptionMode(def);
  }, [profileSettings.loaded, profileSettings.settings.defaultCaptionMode, media.kind]);

  // -- AirPlay (parallel AVPlayer route) ----------------------------------
  // When an AirPlay device is selected, the backend AVPlayer streams the proxy
  // HLS to it; we pause mpv while engaged and resume locally on disconnect.
  const airplay = useAirPlay({
    enabled: USE_MPV_ENGINE,
    profileId,
    mediaKind: media.kind,
    mediaId: media.id,
    layoutKey: `${isFullscreen}:${cinema}`,
    controlsVisible,
    getPosition: () => latestPlaybackRef.current.currentTime,
    getPlaying: () => latestPlaybackRef.current.playing,
    onEngage: () => {
      // Pause the local session so its audio/video stop while the TV plays.
      if (nativeTokenRef.current && latestPlaybackRef.current.playing) {
        void invoke("mpv_toggle_pause", { token: nativeTokenRef.current }).catch(() => undefined);
      }
    },
    onDisengage: (positionSec, wasPlaying) => {
      const token = nativeTokenRef.current;
      if (!token) return;
      void invoke("mpv_seek_absolute", { token, positionSec }).catch(() => undefined);
      if (wasPlaying && !latestPlaybackRef.current.playing) {
        void invoke("mpv_toggle_pause", { token }).catch(() => undefined);
      }
    },
  });

  // -- Native libmpv playback path ---------------------------------------
  useEffect(() => {
    if (!USE_MPV_ENGINE) return;

    let cancelled = false;
    setNativeToken(null);
    setNativeEmbedded(false);
    setHasPainted(false);
    setResolved(null);
    setReady(false);
    setBuffering(true);
    setError(null);
    setLoadingPhase("resolving");
    setLoadingProgress(12);

    void (async () => {
      try {
        // The engine reuses the live session and swaps files via `loadfile
        // replace`, so switching content is one call — no close/teardown here.
        const result = await invoke<NativeOpenResult>("mpv_open_stream", {
          profileId,
          mediaKind: media.kind,
          mediaId: media.id,
          title: media.title,
        });
        if (cancelled) return;
        nativeTokenRef.current = result.token;
        setNativeToken(result.token);
        setNativeEmbedded(result.embedded);
        setLoadingPhase("connecting");
        setLoadingProgress(45);
      } catch (e) {
        if (!cancelled) setError(errorMessage(e));
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [profileId, media.kind, media.id, media.title]);

  // Tear the libmpv session down only when the player actually closes — not on
  // every content switch (those reuse the session). Uses the ref so the latest
  // token is closed regardless of which media was loaded last.
  useEffect(() => {
    if (!USE_MPV_ENGINE) return;
    return () => {
      const token = nativeTokenRef.current;
      if (token) {
        void invoke("mpv_close", { token }).catch(() => undefined);
        nativeTokenRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (!USE_MPV_ENGINE || !nativeEmbedded) return;
    document.documentElement.classList.add("nova-native-player-active");
    // Make the native window transparent only while the video surface is on
    // screen; opaque again on close so fast scroll can't reveal the GL layer.
    void invoke("set_player_surface_active", { active: true }).catch(() => undefined);
    return () => {
      document.documentElement.classList.remove("nova-native-player-active");
      void invoke("set_player_surface_active", { active: false }).catch(() => undefined);
    };
  }, [nativeEmbedded]);

  const applyNativeState = useCallback((state: NativePlayerState) => {
    // Only apply state belonging to the current load. Drop stale tokens and
    // the no-session (token === null) snapshots so a switch never regresses
    // the UI. The ref tracks the newest token even mid-switch.
    if (!nativeTokenRef.current || state.token !== nativeTokenRef.current) return;

    setCurrentTime(state.positionSec);
    setDuration(state.durationSec ?? 0);
    setVolume(state.volume);
    setMuted(state.muted);
    setSpeed(state.speed);
    setTracks(state.tracks);
    setAudioTrackId(state.audioTrackId);
    setVideoTrackId(state.videoTrackId);

    if (state.phase === "error") {
      setReady(false);
      setBuffering(false);
      setPlaying(false);
      setError(state.error ?? "Native playback failed.");
      return;
    }

    setError(null);
    setPlaying(state.phase === "playing" || state.phase === "buffering");
    setReady(state.phase === "playing" || state.phase === "paused" || state.phase === "ended");
    setBuffering(state.phase === "opening" || state.phase === "buffering");
    // A real frame exists once libmpv reports playing/paused — reveal the
    // GL surface and keep it revealed for the rest of this load.
    if (state.phase === "playing" || state.phase === "paused") setHasPainted(true);

    if (state.phase === "opening") {
      setLoadingPhase("resolving");
      setLoadingProgress((p) => Math.max(p, 28));
    } else if (state.phase === "buffering") {
      setLoadingPhase("buffering");
      setLoadingProgress((p) => Math.min(Math.max(p, 68), 88));
    } else {
      setLoadingPhase("ready");
      setLoadingProgress(100);
    }

    // Autoplay the next episode when this one ends.
    if (state.phase === "ended" && !endedHandledRef.current) {
      const { enabled, next } = autoplayRef.current;
      if (enabled && next) {
        endedHandledRef.current = true;
        onPlayMediaRef.current?.(next);
      }
    }
  }, []);

  const snapshotNativeState = useCallback(async () => {
    // Fallback for the event stream. A transient failure here (e.g. a poll
    // racing a content switch) must NOT blank the player — real playback
    // errors arrive via the event stream as `phase === "error"`.
    try {
      const state = await invoke<NativePlayerState>("mpv_player_state", {
        token: nativeTokenRef.current,
      });
      applyNativeState(state);
    } catch {
      /* non-fatal: event stream is the source of truth for errors */
    }
  }, [applyNativeState]);

  // Subscribe to mpv:state ONCE for the modal's lifetime. Re-subscribing per
  // load (the old behavior) dropped the first events of a warm `loadfile`
  // reuse — which loads near-instantly — so the UI got stuck on "opening"
  // until the slow heartbeat caught up. The token gate handles relevance.
  useEffect(() => {
    if (!USE_MPV_ENGINE) return;
    const unlistenPromise = listen<NativePlayerState>("mpv:state", (event) => {
      applyNativeState(event.payload);
    });
    const heartbeat = window.setInterval(() => void snapshotNativeState(), 2500);
    return () => {
      window.clearInterval(heartbeat);
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [applyNativeState, snapshotNativeState]);

  // Immediate catch-up whenever the active load changes, with a couple quick
  // retries so a missed first event self-heals in well under a second.
  useEffect(() => {
    if (!USE_MPV_ENGINE || !nativeToken) return;
    void snapshotNativeState();
    const t1 = window.setTimeout(() => void snapshotNativeState(), 250);
    const t2 = window.setTimeout(() => void snapshotNativeState(), 700);
    return () => {
      window.clearTimeout(t1);
      window.clearTimeout(t2);
    };
  }, [nativeToken, snapshotNativeState]);

  // -- 1. Resolve the playback URL through the backend --------------------
  // The backend probes the upstream codec/container, optionally starts a
  // ffmpeg remuxer, and tells us whether to attach hls.js or use the native
  // video element. The frontend never sees the raw upstream URL.
  useEffect(() => {
    if (USE_MPV_ENGINE) return;

    let cancelled = false;
    let activeToken: string | null = null;
    setResolved(null);
    setReady(false);
    setBuffering(true);
    setError(null);
    setLoadingPhase("resolving");
    setLoadingProgress(8);
    setForceHlsJs(false);
    void (async () => {
      try {
        const stream = await invoke<ResolvedStream>("resolve_stream", {
          profileId,
          mediaKind: media.kind,
          mediaId: media.id,
          routeOverride: activeRouteOverride,
        });
        if (cancelled) {
          void invoke("stop_playback", { token: stream.token }).catch(() => undefined);
          return;
        }
        activeToken = stream.token;
        setResolved(stream);
        setLoadingPhase("connecting");
        setLoadingProgress((p) => Math.max(p, stream.kind === "file" ? 58 : 44));
      } catch (e) {
        if (!cancelled) setError(errorMessage(e));
      }
    })();
    return () => {
      cancelled = true;
      if (activeToken) {
        void invoke("stop_playback", { token: activeToken }).catch(() => undefined);
      }
    };
  }, [profileId, media.kind, media.id, activeRouteOverride]);

  const requestPlaybackFallback = useCallback(
    (reason: string): boolean => {
      if (!resolved) return false;
      const nextRoute = nextFallbackRoute(resolved.route);
      if (!nextRoute || activeRouteOverride === nextRoute) return false;

      console.warn("[player fallback]", reason, {
        from: resolved.route,
        to: nextRoute,
      });
      hlsRef.current?.destroy();
      hlsRef.current = null;
      setRouteOverride({ mediaKey, route: nextRoute });
      setResolved(null);
      setReady(false);
      setBuffering(true);
      setError(null);
      setForceHlsJs(false);
      setLoadingPhase("resolving");
      setLoadingProgress(nextRoute === "remux_hls" ? 18 : 12);
      return true;
    },
    [activeRouteOverride, mediaKey, resolved]
  );

  // -- 2. Attach hls.js (preferred for HLS) or native source --------------
  useEffect(() => {
    if (USE_MPV_ENGINE) return;

    const video = videoRef.current;
    if (!video || !resolved) return;

    setError(null);
    setReady(false);
    setBuffering(true);

    const proxiedSrc = resolved.url;
    const useHls = resolved.kind === "hls";
    const canUseNativeHls =
      useHls && !forceHlsJs && video.canPlayType("application/vnd.apple.mpegurl").length > 0;
    setLoadingPhase(useHls ? "manifest" : "buffering");
    setLoadingProgress((p) => Math.max(p, useHls ? 50 : 62));

    let nativeFallbackTimer: number | null = null;

    // Native HLS is fastest in macOS WebKit. Use hls.js only when native HLS
    // is not available so non-WebKit webviews still work.
    if (useHls && !canUseNativeHls && Hls.isSupported()) {
      const hls = new Hls(HLS_CONFIG);
      hls.on(Hls.Events.MANIFEST_LOADING, () => {
        setLoadingPhase("manifest");
        setLoadingProgress((p) => Math.max(p, 54));
      });
      hls.on(Hls.Events.MANIFEST_PARSED, () => {
        setLoadingPhase("buffering");
        setLoadingProgress((p) => Math.max(p, 68));
      });
      hls.on(Hls.Events.LEVEL_LOADED, () => {
        setLoadingProgress((p) => Math.max(p, 74));
      });
      hls.on(Hls.Events.FRAG_LOADING, () => {
        setLoadingPhase("buffering");
        setLoadingProgress((p) => Math.max(p, 80));
      });
      hls.on(Hls.Events.FRAG_BUFFERED, () => {
        setLoadingProgress((p) => Math.max(p, 90));
      });
      hls.on(Hls.Events.ERROR, (_event, data: ErrorData) => {
        // Log every error (even non-fatal) so diagnosis isn't blind.
        // hls.js drops ~10-20 NON_FATAL events per buffer hiccup; that's fine,
        // the console filter handles volume. Without this we have no signal
        // when "everything fails."
        const reason = formatHlsError(data);
        if (data.fatal) {
          console.error("[hls.js fatal]", reason, data);
        } else {
          console.warn("[hls.js]", reason);
        }
        if (!data.fatal) return;
        switch (data.type) {
          case Hls.ErrorTypes.NETWORK_ERROR:
            hls.startLoad();
            return;
          case Hls.ErrorTypes.MEDIA_ERROR:
            if (requestPlaybackFallback(reason)) return;
            hls.recoverMediaError();
            return;
          default:
            hls.destroy();
            setError(reason);
        }
      });
      hls.loadSource(proxiedSrc);
      hls.attachMedia(video);
      hlsRef.current = hls;
    } else {
      // Native HLS is the fastest path in macOS WebKit. hls.js remains the
      // fallback for webviews that cannot play HLS directly.
      video.src = proxiedSrc;
      if (useHls && canUseNativeHls && Hls.isSupported()) {
        nativeFallbackTimer = window.setTimeout(() => {
          const current = videoRef.current;
          if (!current || current.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) return;
          setLoadingPhase("manifest");
          setLoadingProgress((p) => Math.min(Math.max(p, 72), 94));
          setForceHlsJs(true);
        }, 7_000);
      }
    }

    return () => {
      if (nativeFallbackTimer !== null) window.clearTimeout(nativeFallbackTimer);
      if (hlsRef.current) {
        hlsRef.current.destroy();
        hlsRef.current = null;
      }
      video.removeAttribute("src");
      video.load();
    };
  }, [forceHlsJs, requestPlaybackFallback, resolved]);

  // -- 3. PiP capability detection (must run after mount) -----------------
  useEffect(() => {
    if (USE_MPV_ENGINE) {
      setPipSupported(false);
      return;
    }

    const video = videoRef.current as PiPVideoElement | null;
    const doc = document as PiPDocument;
    setPipSupported(
      Boolean(doc.pictureInPictureEnabled) &&
        Boolean(video?.requestPictureInPicture) &&
        !video?.disablePictureInPicture
    );
  }, [resolved]);

  // -- 4. Video element event wiring --------------------------------------
  useEffect(() => {
    if (USE_MPV_ENGINE) return;

    const video = videoRef.current;
    if (!video) return;

    const updateBuffered = () => {
      const buf = video.buffered;
      if (buf.length === 0) {
        setBufferedEnd(0);
        setBufferedAheadSec(0);
        return;
      }
      // Use the buffered range that contains currentTime; fall back to last.
      let end = buf.end(buf.length - 1);
      for (let i = 0; i < buf.length; i++) {
        if (video.currentTime >= buf.start(i) && video.currentTime <= buf.end(i)) {
          end = buf.end(i);
          break;
        }
      }
      setBufferedEnd(end);
      setBufferedAheadSec(Math.max(0, end - video.currentTime));
    };

    const onPlay = () => setPlaying(true);
    const onPause = () => setPlaying(false);
    const onWaiting = () => {
      setBuffering(true);
      setLoadingPhase("buffering");
      setLoadingProgress((p) => Math.min(Math.max(p, 72), 94));
      updateBuffered();
    };
    const onLoadedMetadata = () => {
      setLoadingProgress((p) => Math.max(p, 72));
      updateBuffered();
    };
    const onLoadedData = () => {
      setLoadingProgress((p) => Math.max(p, 86));
      updateBuffered();
    };
    const onPlaying = () => {
      setLoadingPhase("ready");
      setLoadingProgress(100);
      setBuffering(false);
    };
    const onCanPlay = () => {
      setReady(true);
      setLoadingPhase("ready");
      setLoadingProgress(100);
      setBuffering(false);
      updateBuffered();
    };
    const onTimeUpdate = () => {
      setCurrentTime(video.currentTime);
      updateBuffered();
    };
    const onDurationChange = () => {
      setDuration(Number.isFinite(video.duration) ? video.duration : 0);
    };
    const onVolumeChange = () => {
      setVolume(video.volume);
      setMuted(video.muted);
    };
    const onErr = () => {
      // Fall through to hls.js error handler when present; only set a
      // generic error if there's nothing else to say.
      if (hlsRef.current) return;
      const me = video.error;
      const code = me?.code;
      const codeName =
        code === 1
          ? "ABORTED"
          : code === 2
            ? "NETWORK"
            : code === 3
              ? "DECODE"
              : code === 4
                ? "SRC_NOT_SUPPORTED"
                : "UNKNOWN";
      const detail = me?.message ? `: ${me.message}` : "";
      console.error("[<video> error]", codeName, code, me);
      if ((code === 3 || code === 4) && requestPlaybackFallback(`${codeName}${detail}`)) {
        return;
      }
      setError(`Could not play this stream (${codeName}${detail}).`);
    };
    const onPipEnter = () => setPipActive(true);
    const onPipLeave = () => setPipActive(false);

    video.addEventListener("play", onPlay);
    video.addEventListener("pause", onPause);
    video.addEventListener("waiting", onWaiting);
    video.addEventListener("loadedmetadata", onLoadedMetadata);
    video.addEventListener("loadeddata", onLoadedData);
    video.addEventListener("playing", onPlaying);
    video.addEventListener("canplay", onCanPlay);
    video.addEventListener("timeupdate", onTimeUpdate);
    video.addEventListener("progress", updateBuffered);
    video.addEventListener("durationchange", onDurationChange);
    video.addEventListener("volumechange", onVolumeChange);
    video.addEventListener("error", onErr);
    video.addEventListener("enterpictureinpicture", onPipEnter);
    video.addEventListener("leavepictureinpicture", onPipLeave);

    return () => {
      video.removeEventListener("play", onPlay);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("waiting", onWaiting);
      video.removeEventListener("loadedmetadata", onLoadedMetadata);
      video.removeEventListener("loadeddata", onLoadedData);
      video.removeEventListener("playing", onPlaying);
      video.removeEventListener("canplay", onCanPlay);
      video.removeEventListener("timeupdate", onTimeUpdate);
      video.removeEventListener("progress", updateBuffered);
      video.removeEventListener("durationchange", onDurationChange);
      video.removeEventListener("volumechange", onVolumeChange);
      video.removeEventListener("error", onErr);
      video.removeEventListener("enterpictureinpicture", onPipEnter);
      video.removeEventListener("leavepictureinpicture", onPipLeave);
    };
  }, [requestPlaybackFallback, resolved]);

  // -- Loading progress animation ----------------------------------------
  useEffect(() => {
    if (error || (ready && !buffering)) return;
    const cap = USE_MPV_ENGINE && loadingPhase === "buffering" ? 88 : loadingCap(loadingPhase);
    const interval = window.setInterval(() => {
      setLoadingProgress((current) => {
        if (current >= cap) return current;
        const next = current + Math.max(0.35, (cap - current) * 0.075);
        return Math.min(cap, next);
      });
    }, 300);
    return () => window.clearInterval(interval);
  }, [buffering, error, loadingPhase, ready]);

  // -- 5. Resume position (VOD/episode only) ------------------------------
  useEffect(() => {
    if (!ready || isLive) return;
    if (USE_MPV_ENGINE) {
      if (!nativeToken) return;
      let cancelled = false;
      void (async () => {
        try {
          const info = await invoke<ResumeInfo | null>("get_resume_position", {
            profileId,
            mediaKind: media.kind,
            mediaId: media.id,
          });
          if (cancelled || !info || info.completed || info.positionSec <= 5) return;
          await invoke("mpv_seek_absolute", {
            token: nativeToken,
            positionSec: info.positionSec,
          });
        } catch {
          /* resume is best-effort */
        }
      })();
      return () => {
        cancelled = true;
      };
    }

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
        if (info.positionSec > 5) {
          video.currentTime = info.positionSec;
        }
      } catch {
        /* resume is best-effort */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [ready, isLive, nativeToken, profileId, media.kind, media.id]);

  // -- 6. Watch-history writes (every 5s + on pause + on close) -----------
  useEffect(() => {
    if (USE_MPV_ENGINE) {
      if (!nativeToken || !ready) return;
      const tick = () => {
        const latest = latestPlaybackRef.current;
        if (!Number.isFinite(latest.currentTime)) return;
        const dur =
          Number.isFinite(latest.duration) && latest.duration > 0 ? latest.duration : null;
        const completed = dur !== null && latest.currentTime / dur >= COMPLETED_THRESHOLD;
        void invoke("record_watch_progress", {
          profileId,
          mediaKind: media.kind,
          mediaId: media.id,
          positionSec: latest.currentTime,
          durationSec: dur,
          completed,
          captionMode: captionModeRef.current,
        }).catch(() => {
          /* progress writes are best-effort */
        });
      };

      const interval = window.setInterval(() => {
        if (latestPlaybackRef.current.playing) tick();
      }, PROGRESS_INTERVAL_MS);
      return () => {
        window.clearInterval(interval);
        tick();
      };
    }

    const video = videoRef.current;
    if (!video || !ready) return;

    const tick = () => {
      if (!Number.isFinite(video.currentTime)) return;
      const dur = Number.isFinite(video.duration) ? video.duration : null;
      const completed = dur !== null && video.currentTime / dur >= COMPLETED_THRESHOLD;
      void invoke("record_watch_progress", {
        profileId,
        mediaKind: media.kind,
        mediaId: media.id,
        positionSec: video.currentTime,
        durationSec: dur,
        completed,
        captionMode: captionModeRef.current,
      }).catch(() => {
        /* progress writes are best-effort */
      });
    };

    const interval = window.setInterval(() => {
      if (!video.paused) tick();
    }, PROGRESS_INTERVAL_MS);
    video.addEventListener("pause", tick);
    return () => {
      window.clearInterval(interval);
      video.removeEventListener("pause", tick);
      tick();
    };
  }, [nativeToken, ready, profileId, media.kind, media.id]);

  // -- 7. Fullscreen sync -------------------------------------------------
  // True fullscreen resizes the OS window via Tauri (the DOM Fullscreen API
  // can't resize the native mpv layer, which tracks the WKWebView bounds).
  // Seed state from the window, and nudge a layer redraw on resize so a paused
  // frame follows the new size.
  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    void win.isFullscreen().then((fs) => setIsFullscreen(fs));
    void win
      .onResized(() => {
        if (USE_MPV_ENGINE) void invoke("mpv_request_redraw").catch(() => undefined);
      })
      .then((un) => {
        unlisten = un;
      });
    return () => unlisten?.();
  }, []);

  // -- Cinema mode: constrain the native video to the top region ----------
  // The mpv layer normally fills the window; in cinema mode we measure the
  // video region (CSS px, window-relative) and tell the native layer to shrink
  // into it, leaving the bottom panel visible. Re-pushed on layout/resize.
  useEffect(() => {
    if (!USE_MPV_ENGINE) return;
    const el = videoRegionRef.current;
    const pushRect = () => {
      if (!cinema || !el) {
        void invoke("mpv_set_video_rect", { rect: null }).catch(() => undefined);
        return;
      }
      const r = el.getBoundingClientRect();
      void invoke("mpv_set_video_rect", {
        rect: { x: r.x, y: r.y, width: r.width, height: r.height },
      }).catch(() => undefined);
    };

    pushRect();
    if (!cinema || !el) return;

    const ro = new ResizeObserver(pushRect);
    ro.observe(el);
    window.addEventListener("resize", pushRect);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", pushRect);
      // Restore full-window video when leaving cinema or unmounting.
      void invoke("mpv_set_video_rect", { rect: null }).catch(() => undefined);
    };
  }, [cinema, nativeEmbedded]);

  // -- Imperative actions -------------------------------------------------
  const togglePlay = useCallback(() => {
    if (USE_MPV_ENGINE) {
      if (nativeToken)
        void invoke("mpv_toggle_pause", { token: nativeToken }).catch(() => undefined);
      return;
    }

    const v = videoRef.current;
    if (!v) return;
    if (v.paused) void v.play().catch(() => undefined);
    else v.pause();
  }, [nativeToken]);

  const seekBy = useCallback(
    (delta: number) => {
      if (USE_MPV_ENGINE) {
        if (!nativeToken || isLive) return;
        void invoke("mpv_seek_relative", { token: nativeToken, deltaSec: delta }).catch(
          () => undefined
        );
        return;
      }

      const v = videoRef.current;
      if (!v || isLive) return;
      const next = Math.max(0, Math.min((v.duration || 0) - 0.1, v.currentTime + delta));
      v.currentTime = next;
    },
    [isLive, nativeToken]
  );

  const seekTo = useCallback(
    (seconds: number) => {
      if (USE_MPV_ENGINE) {
        if (!nativeToken || isLive) return;
        void invoke("mpv_seek_absolute", { token: nativeToken, positionSec: seconds }).catch(
          () => undefined
        );
        return;
      }

      const v = videoRef.current;
      if (!v || isLive || !Number.isFinite(v.duration)) return;
      v.currentTime = Math.max(0, Math.min(v.duration - 0.1, seconds));
    },
    [isLive, nativeToken]
  );

  const seekToLiveEdge = useCallback(() => {
    if (USE_MPV_ENGINE) {
      if (nativeToken)
        void invoke("mpv_seek_live_edge", { token: nativeToken }).catch(() => undefined);
      return;
    }

    const v = videoRef.current;
    if (!v) return;
    const hls = hlsRef.current;
    if (hls && hls.liveSyncPosition != null) {
      v.currentTime = hls.liveSyncPosition;
      return;
    }
    // Native HLS: seek to the end of the seekable range.
    if (v.seekable.length > 0) v.currentTime = v.seekable.end(v.seekable.length - 1);
  }, [nativeToken]);

  const setVol = useCallback(
    (next: number) => {
      const clamped = Math.max(0, Math.min(1, next));
      if (USE_MPV_ENGINE) {
        setVolume(clamped);
        if (nativeToken)
          void invoke("mpv_set_volume", { token: nativeToken, volume: clamped }).catch(
            () => undefined
          );
        if (clamped > 0 && muted) {
          setMuted(false);
          if (nativeToken)
            void invoke("mpv_set_mute", { token: nativeToken, muted: false }).catch(
              () => undefined
            );
        }
        return;
      }

      const v = videoRef.current;
      if (!v) return;
      v.volume = clamped;
      if (clamped > 0 && v.muted) v.muted = false;
    },
    [muted, nativeToken]
  );

  const toggleMute = useCallback(() => {
    if (USE_MPV_ENGINE) {
      const next = !muted;
      setMuted(next);
      if (nativeToken)
        void invoke("mpv_set_mute", { token: nativeToken, muted: next }).catch(() => undefined);
      return;
    }

    const v = videoRef.current;
    if (!v) return;
    v.muted = !v.muted;
  }, [muted, nativeToken]);

  const toggleFullscreen = useCallback(() => {
    const win = getCurrentWindow();
    void (async () => {
      try {
        const next = !(await win.isFullscreen());
        await win.setFullscreen(next);
        setIsFullscreen(next);
        // The window resize repositions the WKWebView; nudge libmpv to repaint
        // so the video follows even while paused.
        if (USE_MPV_ENGINE) void invoke("mpv_request_redraw").catch(() => undefined);
      } catch {
        /* window control is best-effort */
      }
    })();
  }, []);

  const exitFullscreenIfActive = useCallback(async (): Promise<boolean> => {
    const win = getCurrentWindow();
    try {
      if (await win.isFullscreen()) {
        await win.setFullscreen(false);
        setIsFullscreen(false);
        if (USE_MPV_ENGINE) void invoke("mpv_request_redraw").catch(() => undefined);
        return true;
      }
    } catch {
      /* best-effort */
    }
    return false;
  }, []);

  const togglePip = useCallback(() => {
    const video = videoRef.current as PiPVideoElement | null;
    const doc = document as PiPDocument;
    if (!video || !pipSupported) return;
    if (doc.pictureInPictureElement) {
      void doc.exitPictureInPicture?.().catch(() => undefined);
    } else {
      void video.requestPictureInPicture?.().catch(() => undefined);
    }
  }, [pipSupported]);

  const selectAudioTrack = useCallback(
    (id: number | "no") => {
      setAudioTrackId(id === "no" ? null : id);
      if (nativeToken)
        void invoke("mpv_set_audio_track", { token: nativeToken, id: String(id) }).catch(
          () => undefined
        );
    },
    [nativeToken]
  );

  const selectVideoTrack = useCallback(
    (id: number) => {
      setVideoTrackId(id);
      if (nativeToken)
        void invoke("mpv_set_video_track", { token: nativeToken, id: String(id) }).catch(
          () => undefined
        );
    },
    [nativeToken]
  );

  // Apply the profile's preferred audio language once per stream, when tracks
  // first arrive (the start screen sets this; exact switching stays manual).
  const preferredAudioAppliedRef = useRef(false);
  useEffect(() => {
    preferredAudioAppliedRef.current = false;
  }, [media.id]);
  useEffect(() => {
    if (preferredAudioAppliedRef.current) return;
    const pref = profileSettings.settings.preferredAudioLang;
    if (!pref || audioTracks.length < 2) return;
    const match = audioTracks.find((t) => t.lang?.toLowerCase().startsWith(pref.toLowerCase()));
    if (match && match.id !== audioTrackId) selectAudioTrack(match.id);
    preferredAudioAppliedRef.current = true;
  }, [audioTracks, audioTrackId, selectAudioTrack, profileSettings.settings.preferredAudioLang]);

  const changeSpeed = useCallback(
    (next: number) => {
      setSpeed(next);
      if (nativeToken)
        void invoke("mpv_set_speed", { token: nativeToken, speed: next }).catch(() => undefined);
    },
    [nativeToken]
  );

  const stepFrame = useCallback(
    (forward: boolean) => {
      if (nativeToken)
        void invoke("mpv_frame_step", { token: nativeToken, forward }).catch(() => undefined);
    },
    [nativeToken]
  );

  const flashSkip = useCallback((dir: -1 | 1) => {
    setSkipIndicator({ dir, key: Date.now() });
    // The indicator hides itself after the CSS animation completes; we
    // clear state shortly after so the same direction can re-trigger.
    window.setTimeout(() => setSkipIndicator(null), 600);
  }, []);

  // -- Controls visibility (auto-hide while playing) ----------------------
  const revealControls = useCallback(() => {
    setControlsVisible(true);
    if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
    const shouldAutoHide = USE_MPV_ENGINE ? playing : !videoRef.current?.paused;
    if (shouldAutoHide) {
      hideTimerRef.current = window.setTimeout(() => {
        setControlsVisible(false);
      }, CONTROLS_HIDE_DELAY_MS);
    }
  }, [playing]);

  useEffect(() => {
    if (!playing) {
      setControlsVisible(true);
      if (hideTimerRef.current) {
        window.clearTimeout(hideTimerRef.current);
        hideTimerRef.current = null;
      }
    } else {
      revealControls();
    }
  }, [playing, revealControls]);

  useEffect(() => {
    return () => {
      if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
    };
  }, []);

  // -- Keyboard shortcuts -------------------------------------------------
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Don't hijack typing in inputs (defensive - none exist in the modal
      // today but easy to add later).
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;

      switch (e.key) {
        case "Escape":
          if (cinema) {
            setCinema(false);
            return;
          }
          void (async () => {
            const exited = await exitFullscreenIfActive();
            if (!exited) onClose();
          })();
          return;
        case " ":
        case "k":
        case "K":
          e.preventDefault();
          togglePlay();
          revealControls();
          return;
        case "ArrowLeft":
          e.preventDefault();
          seekBy(-ARROW_SECONDS);
          flashSkip(-1);
          revealControls();
          return;
        case "ArrowRight":
          e.preventDefault();
          seekBy(ARROW_SECONDS);
          flashSkip(1);
          revealControls();
          return;
        case "j":
        case "J":
          seekBy(-SKIP_SECONDS);
          flashSkip(-1);
          revealControls();
          return;
        case "l":
        case "L":
          seekBy(SKIP_SECONDS);
          flashSkip(1);
          revealControls();
          return;
        case "ArrowUp":
          e.preventDefault();
          setVol((USE_MPV_ENGINE ? volume : (videoRef.current?.volume ?? 0)) + VOLUME_STEP);
          revealControls();
          return;
        case "ArrowDown":
          e.preventDefault();
          setVol((USE_MPV_ENGINE ? volume : (videoRef.current?.volume ?? 0)) - VOLUME_STEP);
          revealControls();
          return;
        case "m":
        case "M":
          toggleMute();
          revealControls();
          return;
        case "f":
        case "F":
          toggleFullscreen();
          return;
        case "t":
        case "T":
          setCinema((v) => !v);
          revealControls();
          return;
        case "p":
        case "P":
          togglePip();
          return;
        case ",":
          if (USE_MPV_ENGINE && !isLive && !playing) {
            e.preventDefault();
            stepFrame(false);
            revealControls();
          }
          return;
        case ".":
          if (USE_MPV_ENGINE && !isLive && !playing) {
            e.preventDefault();
            stepFrame(true);
            revealControls();
          }
          return;
        default:
          // 0-9 -> jump to that decile (VOD only).
          if (/^[0-9]$/.test(e.key) && !isLive) {
            const targetDuration = USE_MPV_ENGINE ? duration : videoRef.current?.duration;
            if (targetDuration && Number.isFinite(targetDuration)) {
              seekTo((parseInt(e.key, 10) / 10) * targetDuration);
              revealControls();
            }
          }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    onClose,
    togglePlay,
    seekBy,
    seekTo,
    setVol,
    toggleMute,
    toggleFullscreen,
    exitFullscreenIfActive,
    cinema,
    togglePip,
    stepFrame,
    flashSkip,
    revealControls,
    isLive,
    duration,
    volume,
    playing,
  ]);

  // -- Surface interactions: single tap = play/pause, double tap = skip --
  const onSurfacePointerDown = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      // Ignore clicks that originated on controls.
      if ((e.target as HTMLElement).closest("[data-controls]")) return;

      revealControls();
      const now = Date.now();
      const last = lastTapRef.current;
      const x = e.clientX;
      const w = e.currentTarget.clientWidth;

      if (last && now - last.t < DOUBLE_TAP_WINDOW_MS && Math.abs(last.x - x) < 60) {
        // Double tap - left third = -10, right third = +10, middle = fullscreen.
        lastTapRef.current = null;
        const ratio = x / w;
        if (ratio < 0.35 && !isLive) {
          seekBy(-SKIP_SECONDS);
          flashSkip(-1);
        } else if (ratio > 0.65 && !isLive) {
          seekBy(SKIP_SECONDS);
          flashSkip(1);
        } else {
          toggleFullscreen();
        }
        return;
      }
      lastTapRef.current = { t: now, x };
      // Single-tap play/pause fires after the double-tap window expires so
      // it doesn't race the skip gesture.
      window.setTimeout(() => {
        if (lastTapRef.current?.t === now) togglePlay();
      }, DOUBLE_TAP_WINDOW_MS);
    },
    [flashSkip, isLive, revealControls, seekBy, toggleFullscreen, togglePlay]
  );

  // -- Derived display values ---------------------------------------------
  const progressPct = useMemo(() => {
    if (isLive || !duration) return 0;
    return Math.min(100, (currentTime / duration) * 100);
  }, [currentTime, duration, isLive]);

  const bufferedPct = useMemo(() => {
    if (isLive || !duration) return 0;
    return Math.min(100, (bufferedEnd / duration) * 100);
  }, [bufferedEnd, duration, isLive]);

  const atLiveEdge = useMemo(() => {
    if (!isLive) return false;
    if (USE_MPV_ENGINE) return true;
    const hls = hlsRef.current;
    if (hls?.liveSyncPosition != null) {
      return Math.abs(currentTime - hls.liveSyncPosition) < 4;
    }
    const v = videoRef.current;
    if (v && v.seekable.length > 0) {
      const end = v.seekable.end(v.seekable.length - 1);
      return Math.abs(currentTime - end) < 4;
    }
    return true;
  }, [currentTime, isLive]);

  const loadingVisible =
    !error && (USE_MPV_ENGINE ? !ready || buffering : !resolved || !ready || buffering);
  const bufferHealthPct = Math.min(
    99,
    Math.round((bufferedAheadSec / BUFFER_TARGET_SECONDS) * 100)
  );
  const visibleLoadingPct = USE_MPV_ENGINE
    ? Math.min(99, Math.round(loadingProgress))
    : ready && buffering
      ? Math.max(5, bufferHealthPct)
      : Math.min(99, Math.round(loadingProgress));
  const loadingDetail = USE_MPV_ENGINE
    ? "Opening the provider stream"
    : resolved
      ? playbackDetail(resolved, bufferedAheadSec, ready && buffering)
      : activeRouteOverride
        ? `Retrying with local ${activeRouteOverride.replace("_hls", "").replace("_", " ")}`
        : "Backend is resolving the provider stream";

  // -- Render -------------------------------------------------------------
  return (
    <div
      ref={containerRef}
      className={cn(
        "nova-native-player-shell fixed inset-0 z-50 flex flex-col select-none",
        USE_MPV_ENGINE && nativeEmbedded ? "bg-transparent" : "bg-black",
        // When controls hide we also hide the cursor.
        !controlsVisible && playing && "cursor-none"
      )}
      onPointerMove={revealControls}
    >
      <div
        ref={videoRegionRef}
        className="relative flex-1 overflow-hidden"
        onPointerDown={onSurfacePointerDown}
      >
        {USE_MPV_ENGINE ? (
          <NativePlaybackBackdrop
            posterUrl={media.posterUrl}
            title={media.title}
            playing={playing}
            buffering={buffering}
            embedded={nativeEmbedded}
            cover={!hasPainted}
          />
        ) : (
          <video
            ref={videoRef}
            poster={media.posterUrl ?? undefined}
            autoPlay
            playsInline
            preload="auto"
            className="absolute inset-0 h-full w-full bg-black"
          />
        )}

        {/* AirPlay placeholder while streaming to an external device. */}
        {airplay.engaged && (
          <AirPlayOverlay
            title={media.title}
            isLive={isLive}
            status={airplay.status}
            onStop={airplay.stop}
          />
        )}

        {/* Caption overlay - Phase 6 source captions; Phase 7 adds translation. */}
        <CaptionOverlay
          mode={captionMode}
          sourceCues={sourceCues}
          translatedCues={translatedCues}
          currentTime={currentTime}
          sourceLang={sourceCaptions.language ?? resolved?.sourceLang}
          targetLang={targetLang}
          captionStyle={captionStyleState.style}
        />

        {/* Caption status banner: model-download prompt, preparing, or error.
            Hidden once cues are streaming (status "streaming" with cues). */}
        {captionMode === "source" &&
          sourceCaptions.status !== "idle" &&
          sourceCaptions.status !== "complete" &&
          !(sourceCaptions.status === "streaming" && sourceCues.length > 0) && (
            <div className="absolute inset-x-0 bottom-[22%] z-10 flex justify-center px-6">
              <div
                className={cn(
                  "flex items-center gap-3 rounded-full bg-black/75 px-4 py-1.5 text-sm text-white/90",
                  "border border-white/10 shadow-lg backdrop-blur-md"
                )}
              >
                {sourceCaptions.status === "needsModel" ? (
                  <>
                    <span>Captions need a one-time model download (~500MB)</span>
                    <button
                      type="button"
                      onClick={sourceCaptions.downloadModel}
                      className="rounded-full bg-[var(--color-accent)] px-3 py-0.5 text-xs font-semibold text-white hover:brightness-110"
                    >
                      Download
                    </button>
                  </>
                ) : sourceCaptions.status === "failed" ? (
                  <>
                    <span className="text-rose-300">
                      Captions failed{sourceCaptions.error ? `: ${sourceCaptions.error}` : ""}
                    </span>
                    <button
                      type="button"
                      onClick={sourceCaptions.downloadModel}
                      className="rounded-full bg-white/15 px-3 py-0.5 text-xs font-semibold hover:bg-white/25"
                    >
                      Retry
                    </button>
                  </>
                ) : (
                  <span>
                    Preparing captions — {sourceCaptionNote(sourceCaptions) ?? "working…"}
                  </span>
                )}
              </div>
            </div>
          )}

        {/* Translation model banner: the NLLB model needs a one-time download, or
            translation failed. Sits above the caption banner so both can show. */}
        {(captionMode === "translation" || captionMode === "dual") &&
          (translatedCaptions.status === "needsModel" ||
            translatedCaptions.status === "downloadingModel" ||
            translatedCaptions.status === "failed") && (
            <div className="absolute inset-x-0 bottom-[28%] z-10 flex justify-center px-6">
              <div
                className={cn(
                  "flex items-center gap-3 rounded-full bg-black/75 px-4 py-1.5 text-sm text-white/90",
                  "border border-white/10 shadow-lg backdrop-blur-md"
                )}
              >
                {translatedCaptions.status === "needsModel" ? (
                  <>
                    <span>Translation needs a one-time model download (~1GB)</span>
                    <button
                      type="button"
                      onClick={translatedCaptions.downloadModel}
                      className="rounded-full bg-[var(--color-accent)] px-3 py-0.5 text-xs font-semibold text-white hover:brightness-110"
                    >
                      Download
                    </button>
                  </>
                ) : translatedCaptions.status === "failed" ? (
                  <>
                    <span className="text-rose-300">
                      Translation failed
                      {translatedCaptions.error ? `: ${translatedCaptions.error}` : ""}
                    </span>
                    <button
                      type="button"
                      onClick={translatedCaptions.downloadModel}
                      className="rounded-full bg-white/15 px-3 py-0.5 text-xs font-semibold hover:bg-white/25"
                    >
                      Retry
                    </button>
                  </>
                ) : (
                  <span>{translatedCaptionNote(translatedCaptions) ?? "Loading translation…"}</span>
                )}
              </div>
            </div>
          )}

        {/* Skip indicator (+-10s) */}
        {skipIndicator && (
          <div
            key={skipIndicator.key}
            className={cn(
              "pointer-events-none absolute top-1/2 -translate-y-1/2 flex h-24 w-24 items-center justify-center rounded-full bg-white/10 backdrop-blur-md text-white",
              "animate-[skip-pop_500ms_ease-out_forwards]",
              skipIndicator.dir < 0 ? "left-[18%]" : "right-[18%]"
            )}
          >
            {skipIndicator.dir < 0 ? (
              <SkipBack className="h-9 w-9" />
            ) : (
              <SkipForward className="h-9 w-9" />
            )}
          </div>
        )}

        {/* Startup / buffering banner */}
        {loadingVisible && (
          <LoadingBanner
            percent={visibleLoadingPct}
            title={
              USE_MPV_ENGINE
                ? nativeLoadingTitle(loadingPhase)
                : loadingTitle(loadingPhase, resolved)
            }
            detail={loadingDetail}
          />
        )}

        {/* Center play button when paused (gives the empty state weight) */}
        {!playing && ready && !buffering && !error && (
          <button
            data-controls
            onClick={togglePlay}
            className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 flex h-20 w-20 items-center justify-center rounded-full bg-white/15 backdrop-blur-md text-white transition-all hover:bg-white/25 hover:scale-105 active:scale-95"
            aria-label="Play"
          >
            <Play className="h-9 w-9 fill-current ml-1" />
          </button>
        )}

        {/* Error overlay */}
        {error && (
          <div className="absolute inset-0 z-20 flex flex-col items-center justify-center gap-3 bg-black/85 text-white">
            <p className="text-sm text-white/80">{error}</p>
            <button
              onClick={onClose}
              className="rounded-lg bg-white/10 px-4 py-1.5 text-xs font-medium transition-colors hover:bg-white/20"
            >
              Close
            </button>
          </div>
        )}

        {/* Top chrome: gradient + title + close (also the window drag handle) */}
        <div
          data-controls
          data-tauri-drag-region
          className={cn(
            "absolute inset-x-0 top-0 z-10 flex items-start justify-between gap-4 px-6 pt-5 pb-12",
            "bg-gradient-to-b from-black/85 via-black/40 to-transparent",
            "transition-opacity duration-200",
            controlsVisible ? "opacity-100" : "opacity-0 pointer-events-none"
          )}
        >
          <div className="min-w-0 flex-1">
            <p className="truncate text-base font-medium text-white drop-shadow-md">
              {media.title}
            </p>
            {media.subtitle && (
              <p className="truncate text-xs text-white/65 mt-0.5 drop-shadow-md">
                {media.subtitle}
              </p>
            )}
          </div>
          <button
            onClick={onClose}
            className="shrink-0 rounded-full bg-white/10 p-2 text-white/85 backdrop-blur-sm transition-all hover:bg-white/20 hover:text-white hover:scale-105"
            aria-label="Close player"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Bottom chrome: scrubber + controls row */}
        <div
          data-controls
          className={cn(
            "absolute inset-x-0 bottom-0 z-10 px-6 pt-12 pb-4",
            "bg-gradient-to-t from-black/90 via-black/55 to-transparent",
            "transition-opacity duration-200",
            controlsVisible ? "opacity-100" : "opacity-0 pointer-events-none"
          )}
        >
          {isLive ? (
            <LiveBar
              atLiveEdge={atLiveEdge}
              onJumpToLive={seekToLiveEdge}
              bufferedPct={bufferedPct}
              progressPct={progressPct}
            />
          ) : (
            <Scrubber
              progressPct={progressPct}
              bufferedPct={bufferedPct}
              duration={duration}
              onSeek={seekTo}
            />
          )}

          <div className="mt-2 flex items-center gap-3 text-white">
            <IconButton onClick={togglePlay} label={playing ? "Pause" : "Play"}>
              {playing ? (
                <Pause className="h-5 w-5 fill-current" />
              ) : (
                <Play className="h-5 w-5 fill-current" />
              )}
            </IconButton>

            {!isLive && (
              <>
                <IconButton onClick={() => seekBy(-SKIP_SECONDS)} label="Back 10 seconds">
                  <SkipBack className="h-5 w-5" />
                </IconButton>
                <IconButton onClick={() => seekBy(SKIP_SECONDS)} label="Forward 10 seconds">
                  <SkipForward className="h-5 w-5" />
                </IconButton>
              </>
            )}

            <VolumeControl
              volume={volume}
              muted={muted}
              onToggleMute={toggleMute}
              onChange={setVol}
            />

            <div className="text-xs tabular-nums text-white/85">
              {isLive ? (
                <span className="flex items-center gap-1.5">
                  <Radio className="h-3.5 w-3.5 text-red-500" />
                  <span className="font-medium tracking-wide">LIVE</span>
                </span>
              ) : (
                <>
                  <span>{formatTime(currentTime)}</span>
                  <span className="text-white/40"> / </span>
                  <span className="text-white/65">{formatTime(duration)}</span>
                </>
              )}
            </div>

            <div className="flex-1" />

            {audioTracks.length >= 2 && (
              <ChromeMenu
                icon={<AudioLines className="h-5 w-5" />}
                label="Audio track"
                heading="Audio"
                active={audioMenuOpen}
                open={audioMenuOpen}
                onToggle={() => setAudioMenuOpen((v) => !v)}
                onClose={() => setAudioMenuOpen(false)}
                options={audioTracks.map((t) => ({
                  key: String(t.id),
                  label: trackLabel(t),
                  active: audioTrackId === t.id,
                }))}
                onSelect={(key) => selectAudioTrack(Number(key))}
              />
            )}

            {videoTracks.length >= 2 && (
              <ChromeMenu
                icon={<MonitorCog className="h-5 w-5" />}
                label="Quality"
                heading="Quality"
                active={qualityMenuOpen}
                open={qualityMenuOpen}
                onToggle={() => setQualityMenuOpen((v) => !v)}
                onClose={() => setQualityMenuOpen(false)}
                options={videoTracks.map((t) => ({
                  key: String(t.id),
                  label: trackLabel(t),
                  active: videoTrackId === t.id,
                }))}
                onSelect={(key) => selectVideoTrack(Number(key))}
              />
            )}

            {!isLive && (
              <ChromeMenu
                icon={<Gauge className="h-5 w-5" />}
                label="Playback speed"
                heading="Speed"
                active={speedMenuOpen || Math.abs(speed - 1) > 0.01}
                open={speedMenuOpen}
                onToggle={() => setSpeedMenuOpen((v) => !v)}
                onClose={() => setSpeedMenuOpen(false)}
                options={SPEED_OPTIONS.map((rate) => ({
                  key: String(rate),
                  label: rate === 1 ? "Normal" : `${rate}×`,
                  active: Math.abs(speed - rate) < 0.01,
                }))}
                onSelect={(key) => changeSpeed(Number(key))}
              />
            )}

            <CaptionButton
              mode={captionMode}
              onChange={setCaptionMode}
              open={captionMenuOpen}
              onToggle={() => setCaptionMenuOpen((v) => !v)}
              onClose={() => setCaptionMenuOpen(false)}
              sourceNote={sourceCaptionNote(sourceCaptions)}
              translatedNote={translatedCaptionNote(translatedCaptions)}
              targetLang={targetLang}
              onTargetChange={setTargetLang}
              languages={targetLanguages}
            />

            {/* Caption-style ("Aa") controls — only while captions are showing. */}
            {captionMode !== "off" && (
              <CaptionStyleButton
                style={captionStyleState.style}
                onChange={captionStyleState.setStyle}
                open={captionStyleOpen}
                onToggle={() => setCaptionStyleOpen((v) => !v)}
                onClose={() => setCaptionStyleOpen(false)}
              />
            )}

            {!isLive && (
              <IconButton
                onClick={() => setTranscriptOpen((v) => !v)}
                label={transcriptOpen ? "Hide transcript" : "Transcript"}
                active={transcriptOpen}
              >
                <ScrollText className="h-5 w-5" />
              </IconButton>
            )}

            <IconButton
              onClick={() => setCinema((v) => !v)}
              label={cinema ? "Exit cinema mode" : "Cinema mode"}
              active={cinema}
            >
              <Clapperboard className="h-5 w-5" />
            </IconButton>

            {pipSupported && (
              <IconButton onClick={togglePip} label="Picture in Picture" active={pipActive}>
                <PictureInPicture className="h-5 w-5" />
              </IconButton>
            )}
            {USE_MPV_ENGINE && <AirPlayButton slotRef={airplay.buttonRef} />}
            <IconButton
              onClick={toggleFullscreen}
              label={isFullscreen ? "Exit fullscreen" : "Fullscreen"}
            >
              {isFullscreen ? <Minimize className="h-5 w-5" /> : <Maximize className="h-5 w-5" />}
            </IconButton>
          </div>
        </div>
      </div>

      {cinema && (
        <CinemaPanel
          media={media}
          isLive={isLive}
          profileId={profileId}
          sourceId={sourceId ?? null}
          currentMediaId={media.id}
          onSelectMedia={(m) => onPlayMedia?.(m)}
        />
      )}

      {transcriptOpen && !isLive && (
        <TranscriptPanel
          profileId={profileId}
          media={{ kind: media.kind, id: media.id }}
          audioTrackIndex={selectedAudioTrack.index}
          targetLang={targetLang}
          currentTime={currentTime}
          sourceCues={sourceCues}
          translatedCues={translatedCues}
          onSeek={seekTo}
          onClose={() => setTranscriptOpen(false)}
        />
      )}

      <style>{`
        @keyframes skip-pop {
          0%   { opacity: 0; transform: translateY(-50%) scale(0.7); }
          25%  { opacity: 1; transform: translateY(-50%) scale(1.05); }
          100% { opacity: 0; transform: translateY(-50%) scale(1); }
        }
      `}</style>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function IconButton({
  children,
  onClick,
  label,
  active = false,
}: {
  children: React.ReactNode;
  onClick: () => void;
  label: string;
  active?: boolean;
}) {
  return (
    <TooltipProvider delayDuration={300} disableHoverableContent>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            onClick={onClick}
            aria-label={label}
            className={cn(
              "flex h-9 w-9 items-center justify-center rounded-full text-white/85",
              "transition-all hover:bg-white/15 hover:text-white hover:scale-105 active:scale-95",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              active && "bg-white/15 text-white"
            )}
          >
            {children}
          </button>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

// YouTube-style cinema panel below the (shrunk) video. Live shows a channel
// strip for in-player switching; VOD/episodes show an info card. The panel is
// opaque so the non-video area never shows the desktop through the transparent
// window.
function CinemaPanel({
  media,
  isLive,
  profileId,
  sourceId,
  currentMediaId,
  onSelectMedia,
}: {
  media: PlayMedia;
  isLive: boolean;
  profileId: string;
  sourceId: number | null;
  currentMediaId: number;
  onSelectMedia: (m: PlayMedia) => void;
}) {
  return (
    <div className="h-56 shrink-0 overflow-hidden border-t border-white/10 bg-card">
      {isLive ? (
        <CinemaChannelStrip
          profileId={profileId}
          sourceId={sourceId}
          currentChannelId={currentMediaId}
          group={media.channelGroup}
          category={media.category}
          region={media.region}
          onSelectChannel={onSelectMedia}
        />
      ) : media.kind === "episode" ? (
        <CinemaEpisodeBrowser
          profileId={profileId}
          currentEpisodeId={currentMediaId}
          fallbackPoster={media.posterUrl}
          onSelectMedia={onSelectMedia}
        />
      ) : (
        <div className="flex h-full items-center gap-5 px-8">
          <div className="aspect-[2/3] h-40 shrink-0 overflow-hidden rounded-lg bg-neutral-800">
            {media.posterUrl && (
              <img
                src={media.posterUrl}
                alt=""
                className="h-full w-full object-cover"
                loading="lazy"
              />
            )}
          </div>
          <div className="min-w-0">
            <p className="text-lg font-semibold text-white">{media.title}</p>
            {media.subtitle && <p className="mt-1 text-sm text-white/55">{media.subtitle}</p>}
          </div>
        </div>
      )}
    </div>
  );
}

/// One tile in a cinema row.
function ChannelTile({
  title,
  logoUrl,
  active,
  onClick,
}: {
  title: string;
  logoUrl: string | null;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex w-28 flex-shrink-0 flex-col items-center gap-1.5 rounded-lg p-2 text-center transition-colors",
        "focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        active ? "bg-white/15" : "hover:bg-white/10"
      )}
    >
      <div className="flex h-16 w-full items-center justify-center overflow-hidden rounded bg-neutral-800">
        {logoUrl ? (
          <img
            src={logoUrl}
            alt=""
            loading="lazy"
            className="max-h-full max-w-full object-contain"
          />
        ) : (
          <Radio className="h-5 w-5 text-white/40" />
        )}
      </div>
      <span className="line-clamp-2 text-[11px] font-medium text-white/85">{title}</span>
    </button>
  );
}

/// Two-layer cinema browser for live: layer 1 = the playing channel's brand
/// bundle; layer 2 = more channels of the same type (preferring the same region).
function CinemaChannelStrip({
  profileId,
  sourceId,
  currentChannelId,
  group,
  category,
  region,
  onSelectChannel,
}: {
  profileId: string;
  sourceId: number | null;
  currentChannelId: number;
  group?: string | null;
  category?: string | null;
  region?: string | null;
  onSelectChannel: (m: PlayMedia) => void;
}) {
  const [bundle, setBundle] = useState<BrowseItem[]>([]);
  const [suggestions, setSuggestions] = useState<ChannelGroup[]>([]);

  // Layer 1 — the brand bundle.
  useEffect(() => {
    if (sourceId == null || !group) {
      setBundle([]);
      return;
    }
    let cancelled = false;
    fetchChannelsInGroup(profileId, sourceId, group)
      .then((c) => !cancelled && setBundle(c))
      .catch(() => !cancelled && setBundle([]));
    return () => {
      cancelled = true;
    };
  }, [profileId, sourceId, group]);

  // Layer 2 — more of the same type, region preferred.
  useEffect(() => {
    if (sourceId == null || !category) {
      setSuggestions([]);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const groups = await invoke<ChannelGroup[]>("list_channel_groups", {
          profileId,
          sourceId,
          limit: 40,
          offset: 0,
          filters: { category, region: region ?? null },
        });
        if (!cancelled) setSuggestions(groups.filter((g) => g.group !== group));
      } catch {
        if (!cancelled) setSuggestions([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [profileId, sourceId, category, region, group]);

  const hasBundle = bundle.length > 1;
  return (
    <div className="flex h-full flex-col gap-3 overflow-y-auto px-6 py-3">
      {hasBundle && (
        <div className="flex flex-col gap-1.5">
          <p className="text-xs font-semibold uppercase tracking-wide text-white/45">
            More from this group
          </p>
          <div className="-mx-1 flex gap-3 overflow-x-auto px-1 [scrollbar-width:thin]">
            {bundle.map((c) => (
              <ChannelTile
                key={c.key}
                title={c.title}
                logoUrl={c.posterUrl}
                active={c.id === currentChannelId}
                onClick={() =>
                  onSelectChannel({
                    kind: "live",
                    id: c.id,
                    title: c.title,
                    subtitle: c.subtitle,
                    posterUrl: c.posterUrl,
                    channelGroup: c.channel?.channelGroup ?? group,
                    category: c.channel?.category ?? category,
                    region: c.channel?.region ?? region,
                  })
                }
              />
            ))}
          </div>
        </div>
      )}
      {suggestions.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <p className="text-xs font-semibold uppercase tracking-wide text-white/45">
            {category ? `More ${category}` : "More channels"}
          </p>
          <div className="-mx-1 flex gap-3 overflow-x-auto px-1 [scrollbar-width:thin]">
            {suggestions.map((g) => (
              <ChannelTile
                key={g.group}
                title={g.title}
                logoUrl={g.logoUrl}
                active={false}
                onClick={() =>
                  onSelectChannel({
                    kind: "live",
                    id: g.representativeId,
                    title: g.title,
                    subtitle: [g.region, g.category].filter(Boolean).join(" / ") || null,
                    posterUrl: g.logoUrl,
                    channelGroup: g.group,
                    category: g.category,
                    region: g.region,
                  })
                }
              />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function trackLabel(t: NativeTrack): string {
  const lang = t.lang ? t.lang.toUpperCase() : null;
  if (t.title && lang) return `${t.title} (${lang})`;
  if (t.title) return t.title;
  if (lang) return lang;
  return `Track ${t.id}`;
}

interface ChromeMenuOption {
  key: string;
  label: string;
  detail?: string;
  active: boolean;
}

/// Toolbar button + popover used by the audio-track, speed, and quality
/// menus. Mirrors the click-outside / radio-list pattern of `CaptionButton`.
function ChromeMenu({
  icon,
  label,
  heading,
  options,
  onSelect,
  open,
  onToggle,
  onClose,
  active = false,
}: {
  icon: React.ReactNode;
  label: string;
  heading: string;
  options: ChromeMenuOption[];
  onSelect: (key: string) => void;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  active?: boolean;
}) {
  return (
    <DropdownMenu open={open} onOpenChange={(o) => (o ? onToggle() : onClose())}>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={label}
          title={label}
          className={cn(
            "flex h-9 w-9 items-center justify-center rounded-full text-white/85",
            "transition-all hover:bg-white/15 hover:text-white hover:scale-105 active:scale-95",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            active && "bg-white/15 text-white"
          )}
        >
          {icon}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="top" align="end" className="max-h-72 w-56 overflow-auto">
        <DropdownMenuLabel>{heading}</DropdownMenuLabel>
        {options.map((opt) => (
          <DropdownMenuCheckItem
            key={opt.key}
            checked={opt.active}
            onSelect={() => onSelect(opt.key)}
          >
            {opt.label}
            {opt.detail ? (
              <span className="ml-2 text-[11px] text-muted-foreground">{opt.detail}</span>
            ) : null}
          </DropdownMenuCheckItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function NativePlaybackBackdrop({
  posterUrl,
  title,
  playing,
  buffering,
  embedded,
  cover,
}: {
  posterUrl?: string | null;
  title: string;
  playing: boolean;
  buffering: boolean;
  embedded: boolean;
  cover: boolean;
}) {
  if (embedded) {
    // Until the current load paints its first frame, cover the GL surface with
    // opaque black so the previous file's last frame can't bleed through on a
    // switch. Once painted, stay transparent — even across mid-stream
    // rebuffering — so the last frame remains visible behind the spinner.
    return (
      <div
        className={cn(
          "pointer-events-none absolute inset-0 transition-colors duration-150",
          cover ? "bg-black" : "bg-transparent"
        )}
      />
    );
  }

  return (
    <div className="absolute inset-0 overflow-hidden bg-black">
      {posterUrl && (
        <img
          src={posterUrl}
          alt=""
          className="absolute inset-0 h-full w-full object-cover opacity-25 blur-xl scale-110"
        />
      )}
      <div className="absolute inset-0 bg-black/70" />
      <div className="absolute inset-0 flex items-center justify-center px-6">
        <div className="max-w-md text-center">
          <p className="truncate text-lg font-medium text-white">{title}</p>
          <p className="mt-2 text-sm text-white/60">
            {buffering ? "Opening stream" : playing ? "Playing" : "Paused"}
          </p>
        </div>
      </div>
    </div>
  );
}

function LoadingBanner({
  percent,
  title,
  detail,
}: {
  percent: number;
  title: string;
  detail: string;
}) {
  const radius = 28;
  const circumference = 2 * Math.PI * radius;
  const clamped = Math.max(0, Math.min(100, percent));
  const strokeOffset = circumference - (clamped / 100) * circumference;

  return (
    <div className="pointer-events-none absolute inset-0 z-20 flex items-center justify-center px-4">
      <div
        className={cn(
          "flex min-h-24 w-full max-w-sm items-center gap-4 rounded-lg border border-white/10",
          "bg-black/[0.78] px-5 py-4 text-white shadow-2xl backdrop-blur-xl"
        )}
      >
        <div className="relative flex h-20 w-20 shrink-0 items-center justify-center">
          <svg className="h-20 w-20 -rotate-90" viewBox="0 0 72 72" aria-hidden="true">
            <circle
              cx="36"
              cy="36"
              r={radius}
              fill="none"
              stroke="rgba(255,255,255,0.16)"
              strokeWidth="5"
            />
            <circle
              cx="36"
              cy="36"
              r={radius}
              fill="none"
              stroke="var(--color-accent)"
              strokeLinecap="round"
              strokeWidth="5"
              strokeDasharray={circumference}
              strokeDashoffset={strokeOffset}
              className="transition-[stroke-dashoffset] duration-300 ease-out"
            />
          </svg>
          <span className="absolute text-sm font-semibold tabular-nums">
            {Math.round(clamped)}%
          </span>
        </div>
        <div className="min-w-0">
          <p className="truncate text-sm font-medium">{title}</p>
          <p className="mt-1 max-h-10 overflow-hidden text-xs leading-5 text-white/60">{detail}</p>
        </div>
      </div>
    </div>
  );
}

function Scrubber({
  progressPct,
  bufferedPct,
  duration,
  onSeek,
}: {
  progressPct: number;
  bufferedPct: number;
  duration: number;
  onSeek: (sec: number) => void;
}) {
  const [hoverPct, setHoverPct] = useState<number | null>(null);
  const previewTime = hoverPct != null && duration > 0 ? hoverPct * duration : null;

  return (
    <div
      className="group relative py-1.5"
      onPointerMove={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        setHoverPct(Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)));
      }}
      onPointerLeave={() => setHoverPct(null)}
    >
      {/* Buffered range, behind the slider's own track. */}
      <div className="pointer-events-none absolute inset-x-0 top-1/2 h-1 -translate-y-1/2 overflow-hidden rounded-full">
        <div className="h-full bg-white/25" style={{ width: `${bufferedPct}%` }} />
      </div>
      <Slider
        value={[Math.max(0, Math.min(100, progressPct))]}
        max={100}
        step={0.1}
        onValueChange={(v) => onSeek((v[0] / 100) * duration)}
        aria-label="Seek"
      />
      {previewTime != null && (
        <div
          className="pointer-events-none absolute -top-6 -translate-x-1/2 rounded bg-black/85 px-1.5 py-0.5 text-[11px] tabular-nums text-white shadow-md"
          style={{ left: `${(hoverPct ?? 0) * 100}%` }}
        >
          {formatTime(previewTime)}
        </div>
      )}
    </div>
  );
}

function LiveBar({
  atLiveEdge,
  onJumpToLive,
  bufferedPct,
  progressPct,
}: {
  atLiveEdge: boolean;
  onJumpToLive: () => void;
  bufferedPct: number;
  progressPct: number;
}) {
  return (
    <div className="flex items-center gap-3">
      <div className="relative h-1 flex-1 overflow-hidden rounded-full bg-white/15">
        <div
          className="absolute inset-y-0 left-0 bg-white/25"
          style={{ width: `${bufferedPct || 100}%` }}
        />
        <div
          className="absolute inset-y-0 left-0 bg-red-500"
          style={{ width: `${atLiveEdge ? 100 : progressPct}%` }}
        />
      </div>
      <button
        onClick={onJumpToLive}
        disabled={atLiveEdge}
        className={cn(
          "flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium uppercase tracking-wide transition-colors",
          atLiveEdge ? "bg-red-500/15 text-red-400" : "bg-white/10 text-white/85 hover:bg-white/20"
        )}
      >
        <span
          className={cn("h-1.5 w-1.5 rounded-full", atLiveEdge ? "bg-red-500" : "bg-white/50")}
        />
        {atLiveEdge ? "Live" : "Go live"}
      </button>
    </div>
  );
}

function VolumeControl({
  volume,
  muted,
  onToggleMute,
  onChange,
}: {
  volume: number;
  muted: boolean;
  onToggleMute: () => void;
  onChange: (v: number) => void;
}) {
  const displayed = muted ? 0 : volume;
  const Icon = muted || volume === 0 ? VolumeX : volume < 0.5 ? Volume1 : Volume2;

  return (
    <div className="group flex items-center">
      <IconButton onClick={onToggleMute} label={muted ? "Unmute" : "Mute"}>
        <Icon className="h-5 w-5" />
      </IconButton>
      <div className="ml-1 w-0 overflow-hidden transition-[width] duration-200 group-hover:w-24 group-focus-within:w-24">
        <Slider
          value={[displayed * 100]}
          max={100}
          step={1}
          onValueChange={(v) => onChange(v[0] / 100)}
          className="w-24"
          aria-label="Volume"
        />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function loadingCap(phase: LoadingPhase): number {
  switch (phase) {
    case "resolving":
      return 42;
    case "connecting":
      return 58;
    case "manifest":
      return 72;
    case "buffering":
      return 94;
    case "ready":
      return 100;
  }
}

function loadingTitle(phase: LoadingPhase, stream: ResolvedStream | null): string {
  if (!stream) return "Preparing stream";
  switch (phase) {
    case "resolving":
      return "Preparing stream";
    case "connecting":
      return "Opening local stream";
    case "manifest":
      return "Loading stream manifest";
    case "buffering":
      return "Buffering video";
    case "ready":
      return "Ready";
  }
}

function nativeLoadingTitle(phase: LoadingPhase): string {
  switch (phase) {
    case "resolving":
    case "connecting":
    case "manifest":
      return "Opening player";
    case "buffering":
      return "Buffering stream";
    case "ready":
      return "Ready";
  }
}

function playbackDetail(
  stream: ResolvedStream,
  bufferedAheadSec: number,
  isRebuffering: boolean
): string {
  if (isRebuffering) {
    return `${formatSeconds(bufferedAheadSec)} buffered ahead`;
  }
  const route = stream.route.replaceAll("_", " ");
  const codecParts = [stream.videoCodec, stream.audioCodec].filter(Boolean).join(" / ");
  if (stream.fallbackReason) return `Fallback ${route}: ${stream.fallbackReason}`;
  if (codecParts) return `${route} - ${codecParts}`;
  return route;
}

function nextFallbackRoute(route: PlaybackRoute): PlaybackRouteOverride | null {
  switch (route) {
    case "hls_passthrough":
    case "file_passthrough":
      return "remux_hls";
    case "remux_hls":
      return "transcode_hls";
    case "transcode_hls":
      return null;
  }
}

function formatSeconds(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0.0s";
  return `${seconds.toFixed(1)}s`;
}

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const s = Math.floor(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  const ss = String(sec).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

function formatHlsError(data: ErrorData): string {
  const parts: string[] = [];
  if (data.type) parts.push(String(data.type));
  if (data.details) parts.push(String(data.details));
  // For network errors include the HTTP status if we have one - the most
  // common silent failure today is the proxy returning 5xx and us showing
  // a generic "Stream error".
  const status = (data as { response?: { code?: number } }).response?.code;
  if (typeof status === "number") parts.push(`HTTP ${status}`);
  const reason = (data as { reason?: string }).reason;
  if (typeof reason === "string" && reason) parts.push(reason);
  return parts.join(" - ");
}

function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "object" && err !== null && "message" in err) {
    const m = (err as { message?: unknown }).message;
    if (typeof m === "string") return m;
  }
  return String(err);
}
