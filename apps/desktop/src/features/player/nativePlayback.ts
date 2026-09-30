import { invoke } from "@tauri-apps/api/core";

// libmpv is one shared engine. Resolve/open/close operations must keep UI order,
// including a close followed immediately by a new player mount.
let pending: Promise<unknown> = Promise.resolve();
function serial<T>(operation: () => Promise<T>): Promise<T> {
  const next = pending.then(operation);
  pending = next.catch(() => undefined);
  return next;
}

export interface NativeOpenResult {
  token: string;
  embedded: boolean;
}

export function openNativePlayback(
  options: {
    profileId: string;
    mediaKind: string;
    mediaId: number;
    title: string;
    startSec?: number | null;
  },
  cancelled: () => boolean
): Promise<NativeOpenResult | null> {
  return serial(async () => {
    if (cancelled()) return null;
    const result = await invoke<NativeOpenResult>("mpv_open_stream", options);
    if (cancelled()) {
      await invoke("mpv_close", { token: result.token });
      return null;
    }
    return result;
  });
}

export function closeNativePlayback(token: string): Promise<void> {
  return serial(() => invoke<void>("mpv_close", { token }));
}
