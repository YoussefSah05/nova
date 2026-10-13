//! Caption cold start on a title page (ADR-0014).
//!
//! Captions used to start only when the player did: the speech model loaded,
//! then the first audio window was downloaded and recognized while the film
//! already played. A title page now prepares that work as soon as the user
//! shows they will watch with captions: the profile starts playback with
//! captions on, or the playback menu was opened. The backend transcribes the
//! opening from the resume point at interactive priority; when Play starts a
//! caption session for the same media and audio track, it adopts this one,
//! so the first captions are often ready before the video is.
//!
//! The intent latches for the page: closing the menu by choosing an item must
//! not cancel the work the player is about to adopt. Leaving the page, or
//! changing the version, track or resume point, cancels an unadopted prepare.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface CaptionPrepareTarget {
  profileId: string;
  mediaKind: "vod" | "episode";
  mediaId: number;
  /** Where playback will start: the resume point, or 0. */
  fromSec: number;
}

export interface CaptionPrepareTrack {
  index: number;
  language: string | null;
}

export function useCaptionPrepare(
  target: CaptionPrepareTarget | null,
  track: CaptionPrepareTrack | undefined,
  intent: boolean
) {
  const fromSec = target ? Math.max(0, Math.floor(target.fromSec)) : 0;
  const key = target
    ? `${target.profileId}:${target.mediaKind}:${target.mediaId}:${track?.index ?? "auto"}:${fromSec}`
    : null;
  const [intentKey, setIntentKey] = useState<string | null>(null);
  useEffect(() => {
    if (intent && key) setIntentKey(key);
  }, [intent, key]);
  const active = key !== null && intentKey === key;

  useEffect(() => {
    if (!active || !target) return;
    const sessionId = crypto.randomUUID();
    void invoke("prepare_captions", {
      sessionId,
      profileId: target.profileId,
      mediaKind: target.mediaKind,
      mediaId: target.mediaId,
      fromSec,
      durationSec: null,
      audioTrackIndex: track?.index ?? null,
      trackLang: track?.language ?? null,
    }).catch(() => undefined);
    return () => {
      void invoke("cancel_caption_prepare", { sessionId }).catch(() => undefined);
    };
    // `key` covers every field the prepare depends on.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, key]);
}
