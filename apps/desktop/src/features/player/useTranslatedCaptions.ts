//! Bounded committed translation; one request at a time per target generation.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { CaptionCue } from "@/types/generated/CaptionCue";
import type { SourceCaptionsState } from "./useSourceCaptions";

interface TranslationState {
  key: string;
  cues: CaptionCue[];
  note: string | null;
  failed: boolean;
}

function errorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }
  return "Translation unavailable — source captions remain available";
}

export function useTranslatedCaptions(
  profileId: string,
  source: SourceCaptionsState,
  active: boolean,
  targetLang: string,
  getPlayhead: () => number
) {
  const [retryCount, setRetryCount] = useState(0);
  const [state, setState] = useState<TranslationState>({
    key: "",
    cues: [],
    note: null,
    failed: false,
  });
  const sourceRef = useRef(source);
  const playheadRef = useRef(getPlayhead);
  sourceRef.current = source;
  playheadRef.current = getPlayhead;
  const sessionId = source.sessionId;
  const transcriptId = source.transcriptId;
  const key = `${profileId}:${sessionId}:${transcriptId}:${targetLang}:${active}:${retryCount}`;
  const retry = useCallback(() => setRetryCount((n) => n + 1), []);

  useEffect(() => {
    if (!active || !sessionId || !transcriptId) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const translated = new Map<number, CaptionCue>();
    setState({ key, cues: [], note: "Preparing translation…", failed: false });
    const tick = async () => {
      if (cancelled) return;
      const playhead = playheadRef.current();
      const candidates = sourceRef.current.cues
        .filter(
          (c) =>
            c.segmentId != null &&
            !translated.has(c.segmentId) &&
            c.end > playhead - 10 &&
            c.start < playhead + 90
        )
        .slice(0, 8);
      if (candidates.length) {
        try {
          const cues = await invoke<CaptionCue[]>("translate_caption_segments", {
            profileId,
            sessionId,
            transcriptId,
            targetLang,
            segmentIds: candidates.map((c) => c.segmentId),
          });
          if (cancelled) return;
          cues.forEach((c) => translated.set(c.segmentId, c));
          // Keep only a bounded playhead horizon in React. Cached translations
          // behind it can be requested again after a backwards seek.
          for (const [id, c] of translated) {
            if (c.end < playhead - 30 || c.start > playhead + 120) translated.delete(id);
          }
          setState({
            key,
            cues: [...translated.values()].sort((a, b) => a.start - b.start),
            note: null,
            failed: false,
          });
        } catch (error) {
          if (cancelled) return;
          // Rapid retargeting can briefly leave the preceding batch in the
          // session's single slot. Wait for it; its result never enters this UI.
          if (
            !(
              typeof error === "object" &&
              error !== null &&
              "kind" in error &&
              error.kind === "Conflict"
            )
          ) {
            setState({
              key,
              cues: [...translated.values()],
              note: errorMessage(error),
              failed: true,
            });
            return;
          }
        }
      }
      timer = setTimeout(() => void tick(), 300);
    };
    void tick();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [active, profileId, sessionId, transcriptId, targetLang, key]);

  // Clear old target/media text on the render that changes the key, before
  // effects have run, so stale translations can never flash over new media.
  const current = state.key === key;
  return {
    cues: current ? state.cues : [],
    note: active && current ? state.note : null,
    failed: current && state.failed,
    retry,
  };
}
