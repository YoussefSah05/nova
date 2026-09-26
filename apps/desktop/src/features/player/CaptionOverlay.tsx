//! Caption rendering overlay.
//!
//! Renders source-language cues (Phase 6 streaming captions). Never use
//! `<track>` tags: captions render through this component so appearance follows
//! the user's caption style. Translated/dual modes were removed with the NLLB
//! engine and return with translation in a later milestone.

import { cn } from "@/lib/utils";
import { DEFAULT_CAPTION_STYLE, type CaptionStyle } from "@/types";
import { captionTextClass, captionTextStyle } from "./captionStyleCss";

export type CaptionMode = "off" | "source";

export interface Cue {
  /// Seconds from playback start.
  start: number;
  end: number;
  text: string;
}

interface CaptionOverlayProps {
  mode: CaptionMode;
  /// Source-language cues. Populated by `transcription:cue` events in Phase 6/9.
  sourceCues: Cue[];
  currentTime: number;
  /// User caption appearance. Defaults to the medium-sans/white/drop-shadow look.
  captionStyle?: CaptionStyle;
}

export function CaptionOverlay({
  mode,
  sourceCues,
  currentTime,
  captionStyle = DEFAULT_CAPTION_STYLE,
}: CaptionOverlayProps) {
  if (mode === "off") return null;

  const sourceActive = activeCue(sourceCues, currentTime);
  if (!sourceActive) return null;
  return <StandardCue text={sourceActive.text} style={captionStyle} />;
}

function activeCue(cues: Cue[], t: number): Cue | undefined {
  // Linear scan is fine - caption arrays are bounded by the 30s rolling
  // window for live, and full transcript reads are batched by Phase 6.
  for (const c of cues) {
    if (t >= c.start && t <= c.end) return c;
  }
  return undefined;
}

function StandardCue({ text, style }: { text: string; style: CaptionStyle }) {
  return (
    <div
      className={cn(
        "pointer-events-none absolute inset-x-0 bottom-[12%] z-10 flex justify-center px-6"
      )}
    >
      <p
        className={cn(
          "max-w-[70%] whitespace-pre-line text-center font-medium leading-snug",
          captionTextClass(style)
        )}
        style={captionTextStyle(style)}
      >
        {text}
      </p>
    </div>
  );
}
