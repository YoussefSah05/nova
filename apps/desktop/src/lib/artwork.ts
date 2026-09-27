//! Session memory of artwork URLs that failed to load, so a dead link is
//! tried once rather than on every remount (see `components/Artwork.tsx`).

import { useState } from "react";

const MAX_REMEMBERED = 5000;
const broken = new Set<string>();

function rememberBroken(src: string) {
  if (broken.size >= MAX_REMEMBERED) broken.clear();
  broken.add(src);
}

/** Test hook: forget every remembered broken URL. */
export function resetBrokenArtwork() {
  broken.clear();
}

/**
 * The usable image URL (`null` when missing or known broken) and the error
 * handler to attach to the element that loads it.
 */
export function useArtworkSrc(src: string | null | undefined): [string | null, () => void] {
  const [failed, setFailed] = useState<string | null>(null);
  const usable = src && failed !== src && !broken.has(src) ? src : null;
  const onError = () => {
    if (!src) return;
    rememberBroken(src);
    setFailed(src);
  };
  return [usable, onError];
}
