//! Artwork loading: through Nova's validated disk cache first, the original URL
//! second, the placeholder last (see `components/Artwork.tsx`).
//!
//! `artworkSrc` maps an http(s) image URL onto the `nova-art` protocol, which
//! serves it from the on-device cache (downloading and validating it once).
//! When that route fails (the image is not cached and Nova could not fetch or
//! validate it) the webview tries the original URL directly, since some CDNs
//! only answer a browser. A URL that fails both ways is remembered for the
//! session so remounting rows don't retry (and flash) a dead link.

import { convertFileSrc } from "@tauri-apps/api/core";
import { useState } from "react";

const SCHEME = "nova-art";
const MAX_REMEMBERED = 5000;
/** URLs the cache route failed for: go straight to the original. */
const cacheFailed = new Set<string>();
/** URLs that failed both ways: show the placeholder. */
const broken = new Set<string>();

function remember(set: Set<string>, src: string) {
  if (set.size >= MAX_REMEMBERED) set.clear();
  set.add(src);
}

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function isArtworkRoute(url: string): boolean {
  return url.startsWith(`${SCHEME}:`) || url.startsWith(`http://${SCHEME}.localhost/`);
}

/** The cached-artwork URL for an http(s) image; anything else unchanged. */
export function artworkSrc(url: string): string {
  if (!inTauri() || isArtworkRoute(url) || !/^https?:\/\//i.test(url)) return url;
  try {
    return convertFileSrc(url, SCHEME);
  } catch {
    return url;
  }
}

/** Test hook: forget every remembered failure. */
export function resetBrokenArtwork() {
  cacheFailed.clear();
  broken.clear();
}

/**
 * The URL to load (`null` when missing or known broken) and the error handler
 * to attach to the element that loads it.
 */
export function useArtworkSrc(src: string | null | undefined): [string | null, () => void] {
  // Failures seen by this component, per original URL: 1 = cache route, 2 = both.
  const [failed, setFailed] = useState<{ src: string; level: 1 | 2 } | null>(null);
  if (!src || broken.has(src)) return [null, () => {}];
  const level = failed?.src === src ? failed.level : 0;
  if (level === 2) return [null, () => {}];
  const cached = artworkSrc(src);
  const direct = level === 1 || cacheFailed.has(src) || cached === src;
  const onError = () => {
    if (direct) {
      remember(broken, src);
      setFailed({ src, level: 2 });
    } else {
      remember(cacheFailed, src);
      setFailed({ src, level: 1 });
    }
  };
  return [direct ? src : cached, onError];
}
