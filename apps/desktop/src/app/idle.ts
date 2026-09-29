//! Run work when the app is idle. WebKit has no `requestIdleCallback`, so it
//! falls back to a short timeout.

export function onIdle(cb: () => void): () => void {
  if (typeof requestIdleCallback === "function") {
    const id = requestIdleCallback(cb);
    return () => cancelIdleCallback(id);
  }
  const id = setTimeout(cb, 200);
  return () => clearTimeout(id);
}
