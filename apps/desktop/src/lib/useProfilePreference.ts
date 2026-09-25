import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// A reopened panel must read after the previous panel's final write.
const writes = new Map<string, Promise<unknown>>();

export function useProfilePreference<T extends object>(
  profileId: string,
  defaults: T,
  readCommand: string,
  writeCommand: string,
  field: string
) {
  const [value, setValue] = useState(defaults);
  const [loaded, setLoaded] = useState(false);
  const updateRef = useRef<(patch: Partial<T>) => void>(() => {});

  useEffect(() => {
    let cancelled = false;
    let current = defaults;
    let edits: Partial<T> = {};
    let loadingEdits: Partial<T> = {};
    let initialized = false;
    let dirty = false;
    let timer: number | undefined;
    const key = `${writeCommand}:${profileId}`;
    setValue(defaults);
    setLoaded(false);

    const ready = Promise.resolve(writes.get(key))
      .then(() => invoke<T>(readCommand, { profileId }))
      .catch(() => defaults)
      .then((saved) => {
        current = { ...saved, ...loadingEdits };
        initialized = true;
        if (!cancelled) {
          setValue(current);
          setLoaded(true);
        }
        return saved;
      });

    const flush = () => {
      window.clearTimeout(timer);
      if (!dirty) return;
      dirty = false;
      // Capture the edits and profile now, never from a later render.
      const patch = { ...edits };
      edits = {};
      const previous = writes.get(key);
      const saving = Promise.all([ready, previous])
        // Other mounted panels may have saved since our initial read. Only
        // replace fields edited in this flush, preserving their other changes.
        .then(() => invoke<T>(readCommand, { profileId }))
        .then((saved) =>
          invoke(writeCommand, {
            profileId,
            [field]: { ...saved, ...patch },
          })
        )
        .catch(() => undefined);
      writes.set(key, saving);
      void saving.then(() => {
        if (writes.get(key) === saving) writes.delete(key);
      });
    };

    updateRef.current = (patch) => {
      edits = { ...edits, ...patch };
      if (!initialized) loadingEdits = { ...loadingEdits, ...patch };
      current = { ...current, ...patch };
      dirty = true;
      setValue(current);
      window.clearTimeout(timer);
      timer = window.setTimeout(flush, 400);
    };
    return () => {
      cancelled = true;
      updateRef.current = () => {};
      flush();
    };
  }, [profileId, defaults, readCommand, writeCommand, field]);

  const update = useCallback((patch: Partial<T>) => updateRef.current(patch), []);
  return { value, update, loaded };
}
