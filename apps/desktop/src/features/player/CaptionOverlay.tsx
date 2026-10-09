//! Caption rendering overlay.
//!
//! Source and translated committed dialogue, rendered independently of playback.

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
    return <StandardCue text={sourceActive.text} style={captionStyle} />;
  }
  const rtl = targetLang === "ar";
  return (
    <div className="pointer-events-none absolute inset-x-0 top-[55%] z-10 flex justify-center px-6">
      <div className="nova-glass nova-panel-enter min-w-[30%] max-w-[60%] rounded-xl border border-white/10 px-6 py-4 shadow-lg">
        {mode === "dual" && (
          <p
            dir={sourceLang?.startsWith("ar") ? "rtl" : "auto"}
            className={cn(
              "mb-2 border-b border-white/10 pb-2 opacity-65",
              captionTextClass(captionStyle)
            )}
            style={{
              ...captionTextStyle(captionStyle),
              fontSize: `${captionStyle.fontScale}rem`,
              ...(sourceLang?.startsWith("ar") ? { fontFamily: "var(--font-arabic)" } : {}),
            }}
          >
            {sourceActive.text}
          </p>
        )}
        <p
          lang={targetLang}
          dir={rtl ? "rtl" : "auto"}
          className={cn(
            "whitespace-pre-line leading-relaxed",
            captionTextClass(captionStyle),
            rtl && "text-right"
          )}
          style={{
            ...captionTextStyle(captionStyle),
            fontSize: `${(rtl ? 1.5 : 1.375) * captionStyle.fontScale}rem`,
            ...(rtl ? { fontFamily: "var(--font-arabic)" } : {}),
          }}
        >
          {translated.text}
        </p>
      </div>
    </div>
  );
}

function activeCue(cues: Cue[], t: number): Cue | undefined {
  // Linear scan is fine - caption arrays are bounded by the 30s rolling
  // window for live, and full transcript reads are batched by Phase 6.
  for (const c of cues) {
    if (t >= c.start && t < c.end) return c;
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
