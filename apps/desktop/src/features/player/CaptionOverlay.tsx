//! Caption rendering overlay.
//!
//! Source and translated committed dialogue, rendered independently of playback.

import type { ReactNode } from "react";

import { cn } from "@/lib/utils";
import { DEFAULT_CAPTION_STYLE, type CaptionStyle } from "@/types";
import { captionTextClass, captionTextStyle } from "./captionStyleCss";
import type { CaptionCue } from "@/types/generated/CaptionCue";

export type CaptionMode = "off" | "source" | "translation" | "dual";

// Existing companion/fixture cues can be display-only. Translation requires a
// committed segmentId from Rust; the hook never submits display-only text.
export type Cue = Pick<CaptionCue, "start" | "end" | "text"> & { segmentId?: number };

interface CaptionOverlayProps {
  mode: CaptionMode;
  /// Source-language cues. Populated by `transcription:cue` events in Phase 6/9.
  sourceCues: Cue[];
  translatedCues?: Cue[];
  targetLang?: string;
  sourceLang?: string | null;
  currentTime: number;
  /// User caption appearance. Defaults to the medium-sans/white/drop-shadow look.
  captionStyle?: CaptionStyle;
}

export function CaptionOverlay({
  mode,
  sourceCues,
  translatedCues = [],
  targetLang = "en",
  sourceLang,
  currentTime,
  captionStyle = DEFAULT_CAPTION_STYLE,
}: CaptionOverlayProps) {
  if (mode === "off") return null;

  const sourceActive = activeCue(sourceCues, currentTime);
  if (!sourceActive) return null;
  const translated = activeCue(translatedCues, currentTime);
  // A target cue must be the same committed segment. Timing alone can pair a
  // stale translation with a replacement boundary cue.
  const matches =
    sourceActive.segmentId != null && translated && translated.segmentId === sourceActive.segmentId;
  if (mode === "source" || !matches) {
    return (
      <CaptionStack>
        <CaptionLine text={sourceActive.text} style={captionStyle} />
      </CaptionStack>
    );
  }
  const sourceRtl = sourceLang?.startsWith("ar") ?? false;
  return (
    <CaptionStack>
      {mode === "dual" && (
        <CaptionLine
          text={sourceActive.text}
          style={captionStyle}
          rtl={sourceRtl}
          scale={DUAL_SOURCE_SCALE}
          className="opacity-80"
        />
      )}
      <CaptionLine
        text={translated.text}
        style={captionStyle}
        lang={targetLang}
        rtl={targetLang === "ar"}
      />
    </CaptionStack>
  );
}

/// In dual mode the original line sits above the translation, slightly smaller.
const DUAL_SOURCE_SCALE = 0.85;
/// Arabic glyphs read smaller than Latin ones at the same size.
const ARABIC_SCALE = 1.1;

/// Every caption mode shares one bottom-anchored stack, so translated and dual
/// captions sit where source captions do and follow the same appearance settings.
function CaptionStack({ children }: { children: ReactNode }) {
  return (
    <div
      data-caption-stack
      className="pointer-events-none absolute inset-x-0 bottom-[12%] z-10 flex flex-col items-center gap-1.5 px-6"
    >
      {children}
    </div>
  );
}

function CaptionLine({
  text,
  style,
  lang,
  rtl = false,
  scale = 1,
  className,
}: {
  text: string;
  style: CaptionStyle;
  lang?: string;
  rtl?: boolean;
  scale?: number;
  className?: string;
}) {
  const css = captionTextStyle({
    ...style,
    fontScale: style.fontScale * scale * (rtl ? ARABIC_SCALE : 1),
  });
  return (
    <p
      lang={lang}
      dir={rtl ? "rtl" : "auto"}
      className={cn(
        "max-w-[70%] whitespace-pre-line text-center font-medium leading-snug",
        captionTextClass(style),
        className
      )}
      style={rtl ? { ...css, fontFamily: "var(--font-arabic)" } : css}
    >
      {text}
    </p>
  );
}

function activeCue(cues: Cue[], t: number): Cue | undefined {
  // Linear scan is fine - caption arrays are bounded by the 30s rolling
  // window for live, and full transcript reads are batched by Phase 6.
  // Cues are padded for reading time, so the previous line may still be up
  // when the next one starts: the line that started last wins.
  let active: Cue | undefined;
  for (const c of cues) {
    if (t >= c.start && t < c.end && (!active || c.start >= active.start)) active = c;
  }
  return active;
}
