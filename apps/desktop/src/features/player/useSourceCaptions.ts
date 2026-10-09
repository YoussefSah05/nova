//! Source-language captions for the player (Phase 6.1 — streaming).
//!
//! When the user picks "Source language", this hook drives a backend caption
//! *session*: it ensures the model is present (prompting an explicit one-time
//! download), starts the session at the current playhead, then accumulates the
//! `transcription:cues` that stream in window-by-window. The first caption
//! appears after an audio window is extracted and transcribed. It also
//! pushes the playhead so the backend prioritises/throttles correctly.
//!
//! Cues render through the React `CaptionOverlay`, never mpv's native renderer.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { invokeSession } from "./sessionCommands";

import type { Cue } from "./CaptionOverlay";
import type { CaptionStatusEvent } from "@/types/generated/CaptionStatusEvent";
import type { CuesEvent } from "@/types/generated/CuesEvent";

export type SourceCaptionStatus =
  | "idle"
  | "needsModel"
  | "downloadingModel"
  | "preparing"
  | "streaming"
  | "complete"
  | "failed";

export interface SourceCaptionsState {
  cues: Cue[];
  sessionId: string | null;
  transcriptId: number | null;
  status: SourceCaptionStatus;
  /// Model-download progress (0..100) when known, else null.
  modelPct: number | null;
  phase?: string | null;
  stageSec?: number;
  firstCuesSec?: number | null;
  language: string | null;
  error: string | null;
  /// Called by the UI to confirm the one-time model download.
  downloadModel: () => void;
}

interface ModelProgress {
  artifact: string;
  downloadedBytes: number;
  totalBytes?: number | null;
}
interface ModelStatusResult {
  installed: boolean;
  modelId: string;
}

const PLAYHEAD_PUSH_MS = 2000;
const CUE_DEDUPE_EPS = 0.05;

/// Merge new cues into the sorted accumulator, replacing any with the same
/// start time (windows can re-emit a boundary cue).
function mergeCues(existing: Cue[], incoming: Cue[]): Cue[] {
  if (incoming.length === 0) return existing;
  const out = existing.slice();
  for (const c of incoming) {
    const i = out.findIndex((e) => Math.abs(e.start - c.start) < CUE_DEDUPE_EPS);
    if (i >= 0) out[i] = c;
    else out.push(c);
  }
  out.sort((a, b) => a.start - b.start);
  return out;
}

/// The selected audio track to transcribe. `index` is the 0-based position among
/// audio tracks (maps to ffmpeg `0:a:<index>`); `lang` is its language tag.
export interface AudioTrackSelection {
  index: number;
  lang: string | null;
}

/// Drive streaming source captions for `media` while `active` is true.
/// `getPlayhead`/`getDuration` read the live player clock without re-subscribing.
/// Changing `audioTrack` re-targets transcription to that track (its cached cues,
/// if any, replay instantly).
export function useSourceCaptions(
  profileId: string,
  media: { kind: string; id: number },
  active: boolean,
  getPlayhead: () => number,
  getDuration: () => number,
  audioTrack: AudioTrackSelection | null
): SourceCaptionsState {
  const [cues, setCues] = useState<Cue[]>([]);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [transcriptId, setTranscriptId] = useState<number | null>(null);
  const [stateKey, setStateKey] = useState<string | null>(null);
  const [status, setStatus] = useState<SourceCaptionStatus>("idle");
  const [modelPct, setModelPct] = useState<number | null>(null);
  const [language, setLanguage] = useState<string | null>(null);
  const [phase, setPhase] = useState<string | null>(null);
  const [stageSec, setStageSec] = useState(0);
  const [firstCuesSec, setFirstCuesSec] = useState<number | null>(null);
  const startedAt = useRef(0);
  const phaseStartedAt = useRef(0);
  const [error, setError] = useState<string | null>(null);

  const trackIndex = audioTrack?.index ?? 0;
  const trackLang = audioTrack?.lang ?? null;
  // Re-key the session on (media, track): switching either tears down + restarts.
  const sessionKey = `${profileId}:${media.kind}:${media.id}:${trackIndex}:${trackLang}`;
  const eligible = active && media.kind !== "live";

  // Keep the latest clock getters in refs so the lifecycle effect need not
  // depend on them (they change identity every render).
  const playheadRef = useRef(getPlayhead);
  const durationRef = useRef(getDuration);
  playheadRef.current = getPlayhead;
  durationRef.current = getDuration;

  const sessionRef = useRef<string | null>(null);

  const startSession = useCallback(
    async (sessionId: string) => {
      if (sessionRef.current !== sessionId) return;
      setStatus("preparing");
      setPhase("Preparing model");
      startedAt.current = performance.now();
      phaseStartedAt.current = startedAt.current;
      setFirstCuesSec(null);
      setStageSec(0);
      try {
        const dur = durationRef.current();
        await invokeSession("caption", "start_caption_session", {
          sessionId,
          profileId,
          mediaKind: media.kind,
          mediaId: media.id,
          fromSec: playheadRef.current(),
          durationSec: dur > 0 ? dur : null,
          audioTrackIndex: trackIndex,
          trackLang,
        });
      } catch (e) {
        if (sessionRef.current !== sessionId) return;
        setStatus("failed");
        setError(String(e));
      }
    },
    [profileId, media.kind, media.id, trackIndex, trackLang]
  );

  const downloadModel = useCallback(() => {
    const sessionId = sessionRef.current;
    if (!sessionId) return;
    void (async () => {
      setStatus("downloadingModel");
      setModelPct(null);
      try {
        await invoke("download_whisper_model");
        await startSession(sessionId);
      } catch (e) {
        if (sessionRef.current !== sessionId) return;
        setStatus("failed");
        setError(String(e));
      }
    })();
  }, [startSession]);

  // Reset everything when the media or selected audio track changes.
  useEffect(() => {
    setCues([]);
    setPhase(null);
    setStageSec(0);
    setFirstCuesSec(null);
    setTranscriptId(null);
    setStateKey(null);
    setStatus("idle");
    setModelPct(null);
    setLanguage(null);
    setError(null);
  }, [sessionKey]);

  useEffect(() => {
    if (!eligible) return;
    let cancelled = false;
    const sessionId = crypto.randomUUID();
    sessionRef.current = sessionId;
    setStateKey(sessionKey);
    setSessionId(sessionId);
    setTranscriptId(null);
    setCues([]);
    const unlisteners: Array<() => void> = [];

    const subscriptions = Promise.all(
      [
        listen<CuesEvent>("transcription:cues", (ev) => {
          if (cancelled) return;
          const p = ev.payload;
          if (p.sessionId !== sessionId || p.mediaKind !== media.kind || p.mediaId !== media.id)
            return;
          if (p.cues.length > 0)
            setFirstCuesSec(
              (previous) => previous ?? (performance.now() - startedAt.current) / 1000
            );
          setLanguage(p.language);
          setTranscriptId(p.transcriptId);
          setCues((prev) => mergeCues(prev, p.cues));
        }),
        listen<CaptionStatusEvent>("transcription:status", (ev) => {
          if (cancelled) return;
          const p = ev.payload;
          if (p.sessionId !== sessionId || p.mediaKind !== media.kind || p.mediaId !== media.id)
            return;
          phaseStartedAt.current = performance.now();
          setStageSec(0);
          setPhase(p.phase ?? null);
          if (p.state === "failed") {
            setStatus("failed");
            setError(p.error ?? "transcription failed");
          } else if (p.state === "preparing" || p.state === "streaming" || p.state === "complete") {
            setStatus(p.state);
          }
        }),
        listen<ModelProgress>("model:download:progress", (ev) => {
          if (cancelled) return;
          const p = ev.payload;
          setModelPct(
            p.totalBytes && p.totalBytes > 0
              ? Math.round((p.downloadedBytes / p.totalBytes) * 100)
              : null
          );
        }),
      ].map((subscription) =>
        subscription.then((unlisten) => {
          if (cancelled) unlisten();
          else unlisteners.push(unlisten);
        })
      )
    );

    // Decide whether to prompt for the model or start immediately.
    void (async () => {
      try {
        await subscriptions;
        if (cancelled) return;
        const model = await invoke<ModelStatusResult>("get_model_status");
        if (cancelled) return;
        if (model.installed) {
          await startSession(sessionId);
        } else {
          setStatus("needsModel");
        }
      } catch (e) {
        if (!cancelled) {
          setStatus("failed");
          setError(String(e));
        }
      }
    })();

    // Push the live playhead so the backend anchors/throttles to playback.
    const interval = window.setInterval(() => {
      void invoke("set_caption_playhead", { sec: playheadRef.current() }).catch(() => undefined);
    }, PLAYHEAD_PUSH_MS);

    return () => {
      cancelled = true;
      if (sessionRef.current === sessionId) sessionRef.current = null;
      window.clearInterval(interval);
      unlisteners.forEach((u) => u());
      void invokeSession("caption", "stop_caption_session").catch(() => undefined);
    };
  }, [eligible, profileId, sessionKey, startSession, media.kind, media.id]);

  useEffect(() => {
    if (!eligible || !phase || status === "failed") return;
    const tick = () => setStageSec(Math.floor((performance.now() - phaseStartedAt.current) / 1000));
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [eligible, phase, status]);

  // Media/track changes must hide preceding dialogue during the very first
  // render, before the lifecycle effects reset their stored state.
  const current = eligible && stateKey === sessionKey;
  return {
    cues: current ? cues : [],
    sessionId: current ? sessionId : null,
    transcriptId: current ? transcriptId : null,
    status: current ? status : "idle",
    modelPct: current ? modelPct : null,
    phase: current ? phase : null,
    stageSec: current ? stageSec : 0,
    firstCuesSec: current ? firstCuesSec : null,
    language: current ? language : null,
    error: current ? error : null,
    downloadModel,
  };
}

/// Short status label for the caption menu / banner, or null when nothing needs
/// surfacing (idle / complete). First-batch timing describes data readiness,
/// not the first visible caption at the playhead.
export function sourceCaptionNote(s: SourceCaptionsState): string | null {
  if (s.phase && s.status !== "failed") return `${s.phase} · ${s.stageSec ?? 0}s`;
  switch (s.status) {
    case "needsModel":
      return "Download model (~500MB)";
    case "downloadingModel":
      return s.modelPct != null ? `Downloading model… ${s.modelPct}%` : "Downloading model…";
    case "preparing":
      return "Preparing…";
    case "complete":
    case "streaming":
      return s.cues.length === 0
        ? "Waiting for caption data…"
        : s.firstCuesSec != null
          ? `Caption data ready · first batch ${s.firstCuesSec.toFixed(1)}s`
          : null;
    case "failed":
      return "Failed — try again";
    default:
      return null;
  }
}
