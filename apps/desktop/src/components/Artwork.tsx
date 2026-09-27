//! Artwork image with a broken-image fallback.
//!
//! Provider logos and posters are often dead links (expired CDNs, hotlink
//! blocks). An `<img>` that fails to load renders the `fallback` instead of the
//! browser's broken-image glyph, and the URL is remembered for the session so
//! virtualized rows that remount don't retry (and flash) the same dead link.

import type { ImgHTMLAttributes, ReactNode } from "react";

import { useArtworkSrc } from "@/lib/artwork";

type ArtworkProps = Omit<ImgHTMLAttributes<HTMLImageElement>, "src" | "onError"> & {
  src: string | null | undefined;
  /** Rendered when there is no URL or it fails to load. */
  fallback?: ReactNode;
};

export function Artwork({ src, fallback = null, alt = "", ...rest }: ArtworkProps) {
  const [usable, onError] = useArtworkSrc(src);
  if (!usable) return <>{fallback}</>;
  return <img src={usable} alt={alt} onError={onError} {...rest} />;
}
