//! Translated captions for the player (Phase 7 — streaming).
//!
//! When the user picks "Translated" or "Dual", this hook drives a backend
//! *translation session* that consumes the source captions and emits
//! `translation:cues` in the chosen reading language. It mirrors
//! `useSourceCaptions`: ensure the NLLB model is present (one-time download),
//! start the session at the playhead, accumulate cues, push the playhead.
//!
//! Source captions must also be running (PlayerModal runs the source session for
//! translation/dual too), since translation reads the source segments.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Cue } from "./CaptionOverlay";
import type { AudioTrackSelection } from "./useSourceCaptions";

export type TranslatedCaptionStatus =
  | "idle"
  | "needsModel"
  | "downloadingModel"
  | "preparing"
  | "streaming"
  | "failed";

export interface TranslatedCaptionsState {
  cues: Cue[];
  status: TranslatedCaptionStatus;
  /// Model-download progress (0..100) when known, else null.
  modelPct: number | null;
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
  state: "preparing" | "streaming" | "failed";
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

/// Drive streaming translated captions for `media` into `targetLang` while
/// `active`. Changing `targetLang` or `audioTrack` tears down + restarts (the
/// new target's cached translations, if any, replay instantly).
export function useTranslatedCaptions(
  profileId: string,
  media: { kind: string; id: number },
  active: boolean,
  getPlayhead: () => number,
  getDuration: () => number,
  audioTrack: AudioTrackSelection | null,
  targetLang: string
): TranslatedCaptionsState {
  const [cues, setCues] = useState<Cue[]>([]);
  const [status, setStatus] = useState<TranslatedCaptionStatus>("idle");
  const [modelPct, setModelPct] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const trackIndex = audioTrack?.index ?? 0;
  // Re-key on (media, track, target): switching any tears down + restarts.
  const sessionKey = `${media.kind}:${media.id}:${trackIndex}:${targetLang}`;
  const eligible = active && media.kind !== "live" && targetLang.length > 0;

  const playheadRef = useRef(getPlayhead);
  const durationRef = useRef(getDuration);
  playheadRef.current = getPlayhead;
  durationRef.current = getDuration;

  const startSession = useCallback(async () => {
    setStatus("preparing");
    try {
      const dur = durationRef.current();
      await invoke("start_translation_session", {
        profileId,
        mediaKind: media.kind,
        mediaId: media.id,
        fromSec: playheadRef.current(),
        durationSec: dur > 0 ? dur : null,
        audioTrackIndex: trackIndex,
        targetLang,
      });
    } catch (e) {
      setStatus("failed");
      setError(String(e));
    }
  }, [profileId, media.kind, media.id, trackIndex, targetLang]);

  const downloadModel = useCallback(() => {
    void (async () => {
      setStatus("downloadingModel");
      setModelPct(null);
      try {
        await invoke("download_nllb_model");
        await startSession();
      } catch (e) {
        setStatus("failed");
        setError(String(e));
      }
    })();
  }, [startSession]);

  // Reset when media / track / target changes.
  useEffect(() => {
    setCues([]);
    setStatus("idle");
    setModelPct(null);
    setError(null);
  }, [sessionKey]);

  useEffect(() => {
    if (!eligible) return;
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    void Promise.all([
      listen<CuesEvent>("translation:cues", (ev) => {
        const p = ev.payload;
        if (p.mediaKind !== media.kind || p.mediaId !== media.id) return;
        setCues((prev) => mergeCues(prev, p.cues));
      }),
      listen<StatusEvent>("translation:status", (ev) => {
        const p = ev.payload;
        if (p.mediaKind !== media.kind || p.mediaId !== media.id) return;
        if (p.state === "failed") {
          setStatus("failed");
          setError(p.error ?? "translation failed");
        } else {
          setStatus(p.state);
        }
      }),
      listen<ModelProgress>("nllb:download:progress", (ev) => {
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

    void (async () => {
      try {
        const model = await invoke<ModelStatusResult>("get_nllb_model_status");
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

    const interval = window.setInterval(() => {
      void invoke("set_translation_playhead", { sec: playheadRef.current() }).catch(
        () => undefined
      );
    }, PLAYHEAD_PUSH_MS);

    return () => {
      cancelled = true;
      window.clearInterval(interval);
      unlisteners.forEach((u) => u());
      void invoke("stop_translation_session").catch(() => undefined);
    };
  }, [eligible, profileId, sessionKey, startSession, media.kind, media.id]);

  return { cues, status, modelPct, error, downloadModel };
}

/// Short status label for the caption menu, or null when nothing needs surfacing.
export function translatedCaptionNote(s: TranslatedCaptionsState): string | null {
  switch (s.status) {
    case "needsModel":
      return "Download model (~1GB)";
    case "downloadingModel":
      return s.modelPct != null ? `Downloading model… ${s.modelPct}%` : "Downloading model…";
    case "preparing":
      return "Preparing…";
    case "streaming":
      return s.cues.length === 0 ? "Translating…" : null;
    case "failed":
      return "Failed — try again";
    default:
      return null;
  }
}
