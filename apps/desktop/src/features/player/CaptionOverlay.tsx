//! Caption rendering overlay.
//!
//! Phase 5: inert - renders nothing when `mode === "off"` (the only mode the
//! player is wired for today). The component exists so Phase 6 (source
//! captions) and Phase 7 (translation card + dual mode) can drop in by
//! feeding it cue arrays without touching `PlayerModal`.
//!
//! Never use `<track>` tags - the translation card (`ARCHITECTURE.md section 10`)
//! requires a centred glassmorphic box that native rendering can't deliver.
//! All cues render through this component instead.

import { cn } from "@/lib/utils";
import { DEFAULT_CAPTION_STYLE, type CaptionStyle } from "@/types";
import { captionTextClass, captionTextStyle, FONT_STACKS, TEXT_COLORS } from "./captionStyleCss";

export type CaptionMode = "off" | "source" | "translation" | "dual";

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
  /// Translated cues. Populated by `translation:cue` events in Phase 7/10.
  translatedCues: Cue[];
  currentTime: number;
  sourceLang?: string | null;
  targetLang?: string | null;
  /// User caption appearance. Defaults to the medium-sans/white/drop-shadow look.
  captionStyle?: CaptionStyle;
}

export function CaptionOverlay({
  mode,
  sourceCues,
  translatedCues,
  currentTime,
  sourceLang,
  targetLang,
  captionStyle = DEFAULT_CAPTION_STYLE,
}: CaptionOverlayProps) {
  if (mode === "off") return null;

  const sourceActive = activeCue(sourceCues, currentTime);
  const translatedActive = activeCue(translatedCues, currentTime);

  if (mode === "source") {
    if (!sourceActive) return null;
    return <StandardCue text={sourceActive.text} style={captionStyle} />;
  }

  // Modes B/C (ARCHITECTURE.md §10): the signature glassmorphic "translation
  // card". Translation shows only the reading-language line; dual stacks a dim
  // source line above it with a divider.
  if (mode === "translation") {
    if (!translatedActive?.text) return null;
    return (
      <TranslationCard
        translated={translatedActive.text}
        targetLang={targetLang}
        style={captionStyle}
      />
    );
  }

  if (mode === "dual") {
    const source = sourceActive?.text;
    const translated = translatedActive?.text;
    if (!source && !translated) return null;
    return (
      <TranslationCard
        source={source}
        translated={translated}
        sourceLang={sourceLang}
        targetLang={targetLang}
        style={captionStyle}
      />
    );
  }

  return null;
}

/// Script hints for a language tag: a fallback font that can render its glyphs
/// (appended after the user's chosen font, so an Arabic line still renders even
/// when the user picks a Latin font), RTL direction, and a slightly taller
/// line-height for Arabic/CJK.
function scriptOf(lang?: string | null): { fallback: string; rtl: boolean; tall: boolean } {
  const base = (lang ?? "").toLowerCase().split(/[-_]/)[0];
  if (["ar", "fa", "ur", "he"].includes(base)) {
    return { fallback: "var(--font-arabic)", rtl: true, tall: true };
  }
  if (["zh", "ja", "ko"].includes(base)) {
    return { fallback: "var(--font-cjk)", rtl: false, tall: true };
  }
  return { fallback: "", rtl: false, tall: false };
}

/// Font family honouring the user's "Aa" choice, with a script-appropriate
/// fallback appended so non-Latin glyphs still render.
function familyFor(style: CaptionStyle, script: ReturnType<typeof scriptOf>): string {
  const base = FONT_STACKS[style.fontFamily];
  return script.fallback ? `${script.fallback}, ${base}` : base;
}

/// The translation card (ARCHITECTURE.md §10 chrome). Pinned near the bottom like
/// standard subtitles, auto-fit width (30–80% of frame), glassmorphic. Honours
/// the user's caption style (size via `fontScale`, font via `fontFamily`, colour
/// via `textColor`). `source` is present only in dual mode.
function TranslationCard({
  source,
  translated,
  sourceLang,
  targetLang,
  style,
}: {
  source?: string;
  translated?: string;
  sourceLang?: string | null;
  targetLang?: string | null;
  style: CaptionStyle;
}) {
  const tgt = scriptOf(targetLang);
  const src = scriptOf(sourceLang);
  const scale = style.fontScale;
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-[12%] z-10 flex justify-center px-6">
      <div
        className="nova-caption-card max-w-[80%] rounded-xl border border-white/[0.08] px-6 py-3"
        style={{
          background: "rgba(10, 10, 12, 0.78)",
          backdropFilter: "blur(20px) saturate(160%)",
          WebkitBackdropFilter: "blur(20px) saturate(160%)",
          boxShadow: "0 8px 32px rgba(0, 0, 0, 0.4)",
        }}
      >
        {source != null && (
          <p
            dir={src.rtl ? "rtl" : "ltr"}
            className="text-center"
            style={{
              fontFamily: familyFor(style, src),
              fontSize: 16 * scale,
              lineHeight: src.tall ? 1.6 : 1.5,
              fontWeight: 400,
              color: "rgba(255, 255, 255, 0.55)",
            }}
          >
            {source}
          </p>
        )}
        {source != null && translated != null && (
          <div className="my-2 h-px bg-white/[0.06]" />
        )}
        {translated != null && (
          <p
            dir={tgt.rtl ? "rtl" : "ltr"}
            className="text-center"
            style={{
              fontFamily: familyFor(style, tgt),
              fontSize: (tgt.tall ? 24 : 22) * scale,
              lineHeight: tgt.tall ? 1.6 : 1.5,
              letterSpacing: tgt.rtl || tgt.tall ? undefined : "-0.01em",
              fontWeight: 400,
              color: TEXT_COLORS[style.textColor],
            }}
          >
            {translated}
          </p>
        )}
      </div>
    </div>
  );
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
