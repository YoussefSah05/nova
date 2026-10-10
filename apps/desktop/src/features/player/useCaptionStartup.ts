import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** Native `playing` can precede the coordinator's resume seek. */
export function useCaptionStartup(
  token: string | null,
  enabled: boolean,
  onPosition: (position: number) => void
) {
  const [settledToken, setSettledToken] = useState<string | null>(null);
  const onPositionRef = useRef(onPosition);
  onPositionRef.current = onPosition;
  useEffect(() => {
    if (!token || !enabled) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const check = async () => {
      try {
        const position = await invoke<number | null>("playback_caption_position", { token });
        if (disposed) return;
        if (position !== null) {
          onPositionRef.current(position);
          setSettledToken(token);
          return;
        }
      } catch {
        // A replaced native session cannot release this startup gate.
      }
      if (!disposed) timer = setTimeout(() => void check(), 250);
    };
    void check();
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [token, enabled]);
  return token !== null && settledToken === token;
}
