import { invoke } from "@tauri-apps/api/core";

// Each backend owns one active session. Keep starts/stops ordered even when
// cleanup runs while a preceding command is still resolving.
const pending = new Map<string, Promise<unknown>>();
export function invokeSession(
  kind: "caption" | "airplay",
  command: string,
  args?: Record<string, unknown>
) {
  const next = Promise.resolve(pending.get(kind)).then(() => invoke(command, args));
  pending.set(
    kind,
    next.catch(() => undefined)
  );
  return next;
}
