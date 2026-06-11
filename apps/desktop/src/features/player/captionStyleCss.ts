//! Caption-style → CSS mapping (Phase 6.2).
//!
//! Shared by `CaptionOverlay` (live captions) and `CaptionStylePanel` (the
//! preview) so the two can't drift. Kept in its own module so the component
//! files export only components (React Fast Refresh).

import type { CSSProperties } from "react";

import { cn } from "@/lib/utils";
import type { CaptionStyle } from "@/types";

/// Base caption font size in rem (the pre-6.2 `text-lg`); scaled by `fontScale`.
const BASE_CAPTION_REM = 1.125;

export const FONT_STACKS: Record<CaptionStyle["fontFamily"], string> = {
  sans: "ui-sans-serif, system-ui, sans-serif",
  serif: "ui-serif, Georgia, Cambria, 'Times New Roman', serif",
  mono: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
  // System "dyslexia-friendly" fallback: a humanist sans with open letterforms.
  dyslexic: "'Comic Sans MS', 'Trebuchet MS', Verdana, sans-serif",
};

export const TEXT_COLORS: Record<CaptionStyle["textColor"], string> = {
  white: "#ffffff",
  yellow: "#f5d90a",
  cyan: "#5fe3e0",
};

/// Tailwind classes that don't depend on dynamic values (the colour and the
/// backdrop variants live in the inline style so opacity can be continuous).
export function captionTextClass(style: CaptionStyle): string {
  return cn(style.background === "box" && "rounded-md px-3 py-1");
}

/// Inline style realising the font, colour, and backdrop choices.
export function captionTextStyle(style: CaptionStyle): CSSProperties {
  const css: CSSProperties = {
    fontSize: `${(BASE_CAPTION_REM * style.fontScale).toFixed(3)}rem`,
    fontFamily: FONT_STACKS[style.fontFamily],
    color: TEXT_COLORS[style.textColor],
  };
  if (style.background === "shadow") {
    css.textShadow = "0 2px 2px rgba(0,0,0,0.75)";
  } else if (style.background === "outline") {
    // Four-way hard shadow approximates a text outline cross-platform.
    css.textShadow =
      "-1px -1px 0 #000, 1px -1px 0 #000, -1px 1px 0 #000, 1px 1px 0 #000, 0 0 3px rgba(0,0,0,0.9)";
  } else {
    // Translucent box behind the text.
    const a = Math.max(0, Math.min(1, style.bgOpacity));
    css.backgroundColor = `rgba(0,0,0,${a.toFixed(2)})`;
  }
  return css;
}
