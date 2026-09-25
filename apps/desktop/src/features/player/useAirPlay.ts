//! AirPlay handoff hook.
//!
//! The local engine is libmpv (renders into a `CAOpenGLLayer`, which cannot
//! AirPlay). When the user picks an AirPlay device through the native
//! `AVRoutePickerView` overlay, the backend `AVPlayer` engine takes over and
//! streams the proxy HLS URL to the device. This hook owns the frontend side
//! of that handoff:
//!
//! - Listens for `airplay:status` (engine state) and `airplay:picker-closed`
//!   (user finished with the system picker). On picker close it calls
//!   `airplay_engage`; once `status.engaged` confirms the route actually
//!   activated it pauses mpv (via `onEngage`) and unmutes the external player.
//!   If the route never activates it aborts and mpv keeps playing.
//! - On disconnect (engaged → not engaged) or an explicit Stop it resumes mpv
//!   at the AirPlay position (`onDisengage`).
//! - Positions the native picker overlay over the control-bar button slot.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { invokeSession } from "./sessionCommands";

export interface AirPlayStatus {
  available: boolean;
  engaged: boolean;
  active: boolean;
  positionSec: number;
  durationSec: number | null;
  paused: boolean;
  volume: number;
  muted: boolean;
  error: string | null;
}

interface UseAirPlayArgs {
  enabled: boolean;
  profileId: string;
  mediaKind: "live" | "vod" | "episode";
  mediaId: number;
  /// Re-key when layout changes (fullscreen / cinema) so the picker overlay
  /// re-measures its position over the button.
  layoutKey: string;
  controlsVisible: boolean;
  /// Read the current mpv position (seconds) at engage time.
  getPosition: () => number;
  /// Read whether mpv is currently playing at engage time.
  getPlaying: () => boolean;
  /// Pause the local mpv session — called once AirPlay actually engages.
  onEngage: () => void;
  /// Resume the local mpv session at `positionSec` — called on disconnect/stop.
  onDisengage: (positionSec: number, wasPlaying: boolean) => void;
}

type EngageState = "idle" | "attempting" | "engaged";

/// Time we wait for `status.engaged` after the picker closes before assuming
/// the user cancelled (or selected a non-AirPlay route) and reverting to mpv.
const ENGAGE_TIMEOUT_MS = 6_000;

export function useAirPlay({
  enabled,
  profileId,
  mediaKind,
  mediaId,
  layoutKey,
  controlsVisible,
  getPosition,
  getPlaying,
  onEngage,
  onDisengage,
}: UseAirPlayArgs) {
  const [available, setAvailable] = useState(false);
  const [engaged, setEngaged] = useState(false);
  const [status, setStatus] = useState<AirPlayStatus | null>(null);
  const buttonRef = useRef<HTMLDivElement | null>(null);

  // Mutable handoff state kept in refs so the event subscription stays stable.
  const engageRef = useRef<EngageState>("idle");
  const generationRef = useRef(0);
  const proxyTokenRef = useRef<string | null>(null);
  const prevEngagedRef = useRef(false);
  const resumeRef = useRef<{ position: number; wasPlaying: boolean } | null>(null);
  const attemptTimerRef = useRef<number | null>(null);

  // Latest prop callbacks, so handlers don't need them in their dep arrays.
  const cbRef = useRef({ getPosition, getPlaying, onEngage, onDisengage });
  cbRef.current = { getPosition, getPlaying, onEngage, onDisengage };
  const mediaRef = useRef({ profileId, mediaKind, mediaId });
  mediaRef.current = { profileId, mediaKind, mediaId };

  const clearAttemptTimer = useCallback(() => {
    if (attemptTimerRef.current !== null) {
      window.clearTimeout(attemptTimerRef.current);
      attemptTimerRef.current = null;
    }
  }, []);

  // Release the proxy session + AVPlayer item and reset to idle.
  const teardown = useCallback(async () => {
    clearAttemptTimer();
    generationRef.current += 1;
    engageRef.current = "idle";
    prevEngagedRef.current = false;
    resumeRef.current = null;
    const token = proxyTokenRef.current;
    proxyTokenRef.current = null;
    setEngaged(false);
    await invokeSession("airplay", "airplay_disengage", { proxyToken: token }).catch(
      () => undefined
    );
  }, [clearAttemptTimer]);

  // Disconnect/stop: hand back to mpv at the external position, then tear down.
  const handBackToMpv = useCallback(
    async (positionSec: number) => {
      const resume = resumeRef.current;
      resumeRef.current = null;
      const pos = positionSec > 0 ? positionSec : (resume?.position ?? 0);
      cbRef.current.onDisengage(pos, resume?.wasPlaying ?? true);
      await teardown();
    },
    [teardown]
  );

  const handleStatus = useCallback(
    (s: AirPlayStatus) => {
      setStatus(s);
      setAvailable(s.available);

      const wasEngaged = prevEngagedRef.current;
      prevEngagedRef.current = s.engaged;

      if (engageRef.current === "attempting" && s.engaged) {
        // External route activated — complete the handoff.
        engageRef.current = "engaged";
        clearAttemptTimer();
        cbRef.current.onEngage();
        void invoke("airplay_set_mute", { muted: false }).catch(() => undefined);
        setEngaged(true);
        return;
      }

      if (engageRef.current === "engaged" && wasEngaged && !s.engaged) {
        // Device disconnected while playing — resume locally.
        void handBackToMpv(s.positionSec);
      }
    },
    [clearAttemptTimer, handBackToMpv]
  );

  const handlePickerClosed = useCallback(async () => {
    if (!enabled || engageRef.current !== "idle") return;
    const generation = generationRef.current;
    const position = cbRef.current.getPosition();
    const wasPlaying = cbRef.current.getPlaying();
    resumeRef.current = { position, wasPlaying };
    engageRef.current = "attempting";

    try {
      const { profileId, mediaKind, mediaId } = mediaRef.current;
      const res = (await invokeSession("airplay", "airplay_engage", {
        profileId,
        mediaKind,
        mediaId,
        positionSec: position,
      })) as { proxyToken: string };
      if (generation !== generationRef.current) {
        // Teardown is queued behind this engage; release its newly-known token.
        await invoke("stop_playback", { token: res.proxyToken }).catch(() => undefined);
        return;
      }
      proxyTokenRef.current = res.proxyToken;
      // If the route never activates (user cancelled / picked the Mac), revert.
      clearAttemptTimer();
      attemptTimerRef.current = window.setTimeout(() => {
        if (engageRef.current === "attempting") void teardown();
      }, ENGAGE_TIMEOUT_MS);
    } catch {
      if (generation === generationRef.current) await teardown();
    }
  }, [enabled, clearAttemptTimer, teardown]);

  // Subscribe once; handlers read mutable state from refs.
  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    const statusUnlisten = listen<AirPlayStatus>("airplay:status", (e) => {
      if (!cancelled) handleStatus(e.payload);
    });
    const pickerUnlisten = listen("airplay:picker-closed", () => {
      if (!cancelled) void handlePickerClosed();
    });
    return () => {
      cancelled = true;
      void statusUnlisten.then((f) => f()).catch(() => undefined);
      void pickerUnlisten.then((f) => f()).catch(() => undefined);
    };
  }, [enabled, handleStatus, handlePickerClosed]);

  // Drop any AirPlay session when the media changes; the new content plays
  // locally and the user can re-pick AirPlay for it.
  useEffect(() => {
    if (engageRef.current !== "idle") void teardown();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profileId, mediaKind, mediaId, enabled]);

  // Position the native route-picker overlay over the button slot. Hidden while
  // unavailable, controls are hidden, or already engaged (the overlay has its
  // own Stop control then).
  useEffect(() => {
    if (!enabled) return;
    // Show/position the native picker whenever the controls are visible (the
    // system picker itself handles the "no devices" case), not only when a
    // route is already detected — otherwise the button never appears until a
    // device happens to be reachable.
    const visible = controlsVisible && !engaged;
    const el = buttonRef.current;
    if (!visible || !el) {
      void invoke("airplay_set_picker_rect", { rect: null }).catch(() => undefined);
      return;
    }
    const measure = () => {
      const r = el.getBoundingClientRect();
      void invoke("airplay_set_picker_rect", {
        rect: { x: r.x, y: r.y, width: r.width, height: r.height },
      }).catch(() => undefined);
    };
    measure();
    // Re-measure shortly after layout settles (the control bar animates in).
    const t = window.setTimeout(measure, 250);
    window.addEventListener("resize", measure);
    return () => {
      window.clearTimeout(t);
      window.removeEventListener("resize", measure);
      void invoke("airplay_set_picker_rect", { rect: null }).catch(() => undefined);
    };
  }, [enabled, controlsVisible, engaged, layoutKey]);

  // Tear down on unmount so a closed modal doesn't leave AVPlayer running.
  useEffect(() => {
    return () => {
      if (engageRef.current !== "idle") void teardown();
    };
  }, [teardown]);

  const stop = useCallback(() => {
    void handBackToMpv(status?.positionSec ?? 0);
  }, [handBackToMpv, status]);

  return { available, engaged, status, buttonRef, stop };
}
