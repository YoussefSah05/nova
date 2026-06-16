//! Source-language captions for the player (Phase 6.1 — streaming).
//!
//! When the user picks "Source language", this hook drives a backend caption
//! *session*: it ensures the model is present (prompting an explicit one-time
//! download), starts the session at the current playhead, then accumulates the
//! `transcription:cues` that stream in window-by-window. The first caption
//! appears within seconds; captions roll forward as playback advances. It also
//! pushes the playhead so the backend prioritises/throttles correctly.
//!
//! Cues render through the React `CaptionOverlay`, never mpv's native renderer.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Cue } from "./CaptionOverlay";

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
  status: SourceCaptionStatus;
  /// Model-download progress (0..100) when known, else null.
  modelPct: number | null;
  language: string | null;
  error: string | null;
  /// Called by the UI to confirm the one-time model download.
  downloadModel: () => void;
}

interface CuesEvent {
  mediaKind: string;
  mediaId: number;
  language: string;
  cues: Cue[];
}
interface StatusEvent {
  mediaKind: string;
  mediaId: number;
  state: "preparing" | "streaming" | "complete" | "failed";
  coveredSec?: number | null;
  error?: string | null;
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
  const [status, setStatus] = useState<SourceCaptionStatus>("idle");
  const [modelPct, setModelPct] = useState<number | null>(null);
  const [language, setLanguage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const trackIndex = audioTrack?.index ?? 0;
  const trackLang = audioTrack?.lang ?? null;
  // Re-key the session on (media, track): switching either tears down + restarts.
  const sessionKey = `${media.kind}:${media.id}:${trackIndex}`;
  const eligible = active && media.kind !== "live";

  // Keep the latest clock getters in refs so the lifecycle effect need not
  // depend on them (they change identity every render).
  const playheadRef = useRef(getPlayhead);
  const durationRef = useRef(getDuration);
  playheadRef.current = getPlayhead;
  durationRef.current = getDuration;

  const startSession = useCallback(async () => {
    setStatus("preparing");
    try {
      const dur = durationRef.current();
      await invoke("start_caption_session", {
        profileId,
        mediaKind: media.kind,
        mediaId: media.id,
        fromSec: playheadRef.current(),
        durationSec: dur > 0 ? dur : null,
        audioTrackIndex: trackIndex,
        trackLang,
      });
    } catch (e) {
      setStatus("failed");
      setError(String(e));
    }
  }, [profileId, media.kind, media.id, trackIndex, trackLang]);

  const downloadModel = useCallback(() => {
    void (async () => {
      setStatus("downloadingModel");
      setModelPct(null);
      try {
        await invoke("download_whisper_model");
        await startSession();
      } catch (e) {
        setStatus("failed");
        setError(String(e));
      }
    })();
  }, [startSession]);

  // Reset everything when the media or selected audio track changes.
  useEffect(() => {
    setCues([]);
    setStatus("idle");
    setModelPct(null);
    setLanguage(null);
    setError(null);
  }, [sessionKey]);

  useEffect(() => {
    if (!eligible) return;
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    void Promise.all([
      listen<CuesEvent>("transcription:cues", (ev) => {
        const p = ev.payload;
        if (p.mediaKind !== media.kind || p.mediaId !== media.id) return;
        setLanguage(p.language);
        setCues((prev) => mergeCues(prev, p.cues));
      }),
      listen<StatusEvent>("transcription:status", (ev) => {
        const p = ev.payload;
        if (p.mediaKind !== media.kind || p.mediaId !== media.id) return;
        if (p.state === "failed") {
          setStatus("failed");
          setError(p.error ?? "transcription failed");
        } else {
          setStatus(p.state);
        }
      }),
      listen<ModelProgress>("model:download:progress", (ev) => {
        const p = ev.payload;
        setModelPct(
          p.totalBytes && p.totalBytes > 0
            ? Math.round((p.downloadedBytes / p.totalBytes) * 100)
            : null
        );
      }),
    ]).then((uns) => {
      if (cancelled) {
        uns.forEach((u) => u());
        return;
      }
      unlisteners.push(...uns);
    });

    // Decide whether to prompt for the model or start immediately.
    void (async () => {
      try {
        const model = await invoke<ModelStatusResult>("get_model_status");
        if (cancelled) return;
        if (model.installed) {
          await startSession();
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
      window.clearInterval(interval);
      unlisteners.forEach((u) => u());
      void invoke("stop_caption_session").catch(() => undefined);
    };
  }, [eligible, profileId, sessionKey, startSession, media.kind, media.id]);

  return { cues, status, modelPct, language, error, downloadModel };
}

/// Short status label for the caption menu / banner, or null when nothing needs
/// surfacing (idle / streaming-with-cues / complete).
export function sourceCaptionNote(s: SourceCaptionsState): string | null {
  switch (s.status) {
    case "needsModel":
      return "Download model (~500MB)";
    case "downloadingModel":
      return s.modelPct != null ? `Downloading model… ${s.modelPct}%` : "Downloading model…";
    case "preparing":
      return "Preparing…";
    case "streaming":
      return s.cues.length === 0 ? "Preparing…" : null;
    case "failed":
      return "Failed — try again";
    default:
      return null;
  }
}
