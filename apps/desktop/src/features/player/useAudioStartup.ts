import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { canonicalTrackLanguage, type PlaybackLaunchOptions } from "./playbackLaunch";

interface AudioTrack {
  id: number;
  lang: string | null;
}

/** Confirm the initial audible track before captions consume its source ordinal. */
export function useAudioStartup({
  mediaKind,
  mediaId,
  token,
  ready,
  tracks,
  selectedId,
  requested,
  profileLoaded,
  preferredLanguage,
}: {
  mediaKind: string;
  mediaId: number;
  token: string | null;
  ready: boolean;
  tracks: AudioTrack[];
  selectedId: number | null;
  requested?: PlaybackLaunchOptions["audioTrack"];
  profileLoaded: boolean;
  preferredLanguage: string | null;
}) {
  const key = `${mediaKind}:${mediaId}:${token}`;
  const started = useRef<string | null>(null);
  const acknowledgementTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const currentSelected = useRef(selectedId);
  currentSelected.current = selectedId;
  const [result, setResult] = useState<{
    key: string;
    expectedId?: number;
    note: string | null;
  } | null>(null);
  useEffect(() => {
    if (
      result?.key === key &&
      result.expectedId !== undefined &&
      result.expectedId === selectedId
    ) {
      clearTimeout(acknowledgementTimer.current);
      setResult({ key, note: result.note });
    }
  }, [key, result, selectedId]);
  useEffect(() => {
    started.current = null;
    return () => {
      started.current = null;
      clearTimeout(acknowledgementTimer.current);
    };
  }, [key]);
  useEffect(() => {
    if (!token || !ready || !profileLoaded || started.current === key) return;
    started.current = key;
    let note: string | null = null;
    let desired: AudioTrack | undefined;
    if (requested) {
      const candidate = tracks[requested.index];
      if (
        requested.mediaKind === mediaKind &&
        requested.mediaId === mediaId &&
        candidate &&
        canonicalTrackLanguage(requested.language) === canonicalTrackLanguage(candidate.lang)
      ) {
        desired = candidate;
      } else {
        note = "Selected audio track is no longer available. Using automatic audio.";
      }
    }
    desired ??= tracks.find(
      (track) =>
        preferredLanguage &&
        canonicalTrackLanguage(track.lang) === canonicalTrackLanguage(preferredLanguage)
    );
    if (!tracks.length) note = "No audio track available for transcription.";
    if (!desired || desired.id === selectedId) {
      setResult({ key, note });
      return;
    }
    const expectedId = desired.id;
    void invoke("mpv_set_audio_track", { token, id: String(expectedId) })
      .then(() => {
        if (started.current !== key) return;
        setResult({ key, expectedId, note });
        acknowledgementTimer.current = setTimeout(() => {
          if (started.current === key && currentSelected.current !== expectedId)
            setResult({
              key,
              note: "Audio selection was not confirmed. Using the current audio track.",
            });
        }, 3000);
      })
      .catch(() => {
        if (started.current === key)
          setResult({
            key,
            note: "Audio track could not be selected. Using the current audio track.",
          });
      });
  }, [
    key,
    token,
    ready,
    profileLoaded,
    requested,
    preferredLanguage,
    tracks,
    selectedId,
    mediaKind,
    mediaId,
  ]);

  return {
    settled:
      result?.key === key && (result.expectedId === undefined || result.expectedId === selectedId),
    hasAudio: tracks.some((track) => track.id === selectedId),
    note:
      result?.key === key
        ? (result.note ??
          (tracks.length > 0 && selectedId === null
            ? "No audio track is selected. Choose audio in the player to start transcription."
            : null))
        : null,
  };
}
