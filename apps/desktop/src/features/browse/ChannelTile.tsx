//! A live channel's tile: the logo on its own brand colour (see
//! `lib/logoTone.ts`) instead of a grey box, as TV apps present channels. A
//! missing or broken logo falls back to the initials tile.

import { memo } from "react";

import { useArtworkSrc } from "@/lib/artwork";
import { NEUTRAL_TONE, useLogoTone } from "@/lib/logoTone";
import { cn } from "@/lib/utils";
import { PosterFallback } from "./PosterFallback";

export const ChannelTile = memo(function ChannelTile({
  logoUrl,
  title,
  className,
  logoClassName,
}: {
  logoUrl: string | null | undefined;
  title: string;
  className?: string;
  /** Overrides the logo's size inside the tile. */
  logoClassName?: string;
}) {
  const [src, onError] = useArtworkSrc(logoUrl);
  const tone = useLogoTone(src ? logoUrl : null);
  if (!src) return <PosterFallback title={title} live className={className} />;
  const t = tone ?? NEUTRAL_TONE;
  return (
    <div
      className={cn(
        "relative flex h-full w-full items-center justify-center overflow-hidden transition-[background] duration-500",
        className
      )}
      style={{ background: t.background }}
      data-logo-mode={tone?.mode ?? "pending"}
    >
      <img
        src={src}
        alt=""
        decoding="async"
        onError={onError}
        className={cn(
          "transition-opacity duration-300",
          tone ? "opacity-100" : "opacity-0",
          t.mode === "cover"
            ? "h-full w-full object-cover"
            : cn("max-h-[58%] max-w-[70%] object-contain", logoClassName)
        )}
      />
      {/* A faint top sheen gives the flat colour some depth. */}
      <div className="pointer-events-none absolute inset-0 bg-gradient-to-b from-white/[0.07] via-transparent to-black/[0.12]" />
    </div>
  );
});
