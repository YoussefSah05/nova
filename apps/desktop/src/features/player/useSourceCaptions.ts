//! Source-language captions for the player (Phase 6).
//!
//! When the user picks "Source language" captions, this hook makes the
//! transcript appear: it checks whether one exists, enqueues a `transcribe_vod`
//! job if not, follows the backend's progress/complete events (including the
//! one-time model download), and fetches the cues when ready. Cues render
//! through the React `CaptionOverlay` — never mpv's native subtitle renderer —
//! so the Phase 7 translation card can reuse the same pipeline.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Cue } from "./CaptionOverlay";

export type SourceCaptionStatus =
  | "idle"
  | "checking"
  | "downloadingModel"
  | "generating"
  | "ready"
  | "failed";

export interface SourceCaptionsState {
  cues: Cue[];
  status: SourceCaptionStatus;
  /// Transcription progress, 0..100.
  pct: number;
  /// Model-download progress when known, else null.
  modelPct: number | null;
  language: string | null;
  error: string | null;
}

const INITIAL: SourceCaptionsState = {
  cues: [],
  status: "idle",
  pct: 0,
  modelPct: null,
  language: null,
  error: null,
};

interface ProgressEvent {
  mediaKind: string;
  mediaId: number;
  stage: string;
  pct: number;
  error?: string | null;
}
interface CompleteEvent {
  mediaKind: string;
  mediaId: number;
  language: string;
  vttPath: string;
}
interface ModelProgress {
  artifact: string;
  downloadedBytes: number;
  totalBytes?: number | null;
}
interface CuesResult {
  language: string;
  cues: Cue[];
}
interface StatusResult {
  state: string;
  vttPath?: string | null;
  language?: string | null;
}

/// Drive source-caption generation/loading for `media` while `active` is true.
/// `active` is typically `captionMode === "source"`. Live channels are not
/// eligible (live transcription arrives in Phase 9).
export function useSourceCaptions(
  profileId: string,
  media: { kind: string; id: number },
  active: boolean
): SourceCaptionsState {
  const [state, setState] = useState<SourceCaptionsState>(INITIAL);
  const mediaKey = `${media.kind}:${media.id}`;

  // Forget everything when the media changes.
  useEffect(() => {
    setState(INITIAL);
  }, [mediaKey]);

  const eligible = active && media.kind !== "live";

  useEffect(() => {
    if (!eligible) return;
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    const fetchCues = async () => {
      try {
        const res = await invoke<CuesResult>("get_transcript_cues", {
          profileId,
          mediaKind: media.kind,
          mediaId: media.id,
        });
        if (cancelled) return;
        setState((s) => ({
          ...s,
          cues: res.cues,
          language: res.language,
          status: "ready",
          pct: 100,
          error: null,
        }));
      } catch (e) {
        if (!cancelled) setState((s) => ({ ...s, status: "failed", error: String(e) }));
      }
    };

    const begin = async () => {
      setState((s) => ({ ...s, status: "checking", error: null }));
      let status: StatusResult;
      try {
        status = await invoke<StatusResult>("get_transcript_status", {
          profileId,
          mediaKind: media.kind,
          mediaId: media.id,
        });
      } catch (e) {
        if (!cancelled) setState((s) => ({ ...s, status: "failed", error: String(e) }));
        return;
      }
      if (cancelled) return;

      if (status.state === "done") {
        await fetchCues();
        return;
      }
      // none | queued | running | failed → (re)enqueue and wait for events.
      setState((s) => ({ ...s, status: "generating", pct: 0 }));
      try {
        await invoke("enqueue_transcription", {
          profileId,
          mediaKind: media.kind,
          mediaId: media.id,
        });
      } catch (e) {
        if (!cancelled) setState((s) => ({ ...s, status: "failed", error: String(e) }));
      }
    };

    // Register listeners before kicking off work so no fast event is missed.
    void Promise.all([
      listen<ProgressEvent>("transcription:progress", (ev) => {
        const p = ev.payload;
        if (p.mediaKind !== media.kind || p.mediaId !== media.id) return;
        if (p.stage === "failed") {
          setState((s) => ({ ...s, status: "failed", error: p.error ?? "transcription failed" }));
        } else if (p.stage !== "complete") {
          setState((s) => ({ ...s, status: "generating", pct: p.pct }));
        }
      }),
      listen<CompleteEvent>("transcription:complete", (ev) => {
        const p = ev.payload;
        if (p.mediaKind !== media.kind || p.mediaId !== media.id) return;
        void fetchCues();
      }),
      listen<ModelProgress>("model:download:progress", (ev) => {
        const p = ev.payload;
        const pct =
          p.totalBytes && p.totalBytes > 0
            ? Math.round((p.downloadedBytes / p.totalBytes) * 100)
            : null;
        setState((s) => ({ ...s, status: "downloadingModel", modelPct: pct }));
      }),
    ]).then((uns) => {
      if (cancelled) uns.forEach((u) => u());
      else {
        unlisteners.push(...uns);
        void begin();
      }
    });

    return () => {
      cancelled = true;
      unlisteners.forEach((u) => u());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eligible, profileId, mediaKey]);

  return state;
}

/// A short status label for the caption menu / progress banner, or null when
/// nothing needs surfacing (idle or ready).
export function sourceCaptionNote(s: SourceCaptionsState): string | null {
  switch (s.status) {
    case "checking":
      return "Checking…";
    case "downloadingModel":
      return s.modelPct != null ? `Downloading model… ${s.modelPct}%` : "Downloading model…";
    case "generating":
      return `Generating… ${s.pct}%`;
    case "failed":
      return "Failed — try again";
    default:
      return null;
  }
}
