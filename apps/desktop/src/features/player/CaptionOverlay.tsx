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
}

export function CaptionOverlay({
  mode,
  sourceCues,
  translatedCues,
  currentTime,
}: CaptionOverlayProps) {
  if (mode === "off") return null;

  const sourceActive = activeCue(sourceCues, currentTime);
  const translatedActive = activeCue(translatedCues, currentTime);

  if (mode === "source") {
    if (!sourceActive) return null;
    return <StandardCue text={sourceActive.text} />;
  }

  // Modes B/C are spec'd in ARCHITECTURE.md section 10 but ship in Phase 7. The
  // component renders the standard cue in the meantime so a stray Phase 6
  // build doesn't crash with an unknown mode.
  if (mode === "translation") {
    const text = translatedActive?.text;
    if (!text) return null;
    return <StandardCue text={text} />;
  }

  if (mode === "dual") {
    const top = sourceActive?.text;
    const bottom = translatedActive?.text;
    if (!top && !bottom) return null;
    return <StandardCue text={[top, bottom].filter(Boolean).join("\n")} />;
  }

  return null;
}

function activeCue(cues: Cue[], t: number): Cue | undefined {
  // Linear scan is fine - caption arrays are bounded by the 30s rolling
  // window for live, and full transcript reads are batched by Phase 6.
  for (const c of cues) {
    if (t >= c.start && t <= c.end) return c;
  }
  return undefined;
}

function StandardCue({ text }: { text: string }) {
  return (
    <div
      className={cn(
        "pointer-events-none absolute inset-x-0 bottom-[12%] z-10 flex justify-center px-6"
      )}
    >
      <p
        className={cn(
          "max-w-[70%] whitespace-pre-line text-center text-lg font-medium leading-snug text-white",
          "drop-shadow-[0_2px_2px_rgba(0,0,0,0.75)]"
        )}
      >
        {text}
      </p>
    </div>
  );
}
