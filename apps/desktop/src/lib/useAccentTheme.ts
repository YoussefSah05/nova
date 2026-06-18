//! Accent theming. The brand colour is driven entirely by CSS custom properties
//! on :root (set in globals.css, default amber). `useAccentTheme` overrides them
//! at runtime from the profile's chosen theme so the whole app — buttons, focus
//! rings, active states, glow — recolours live. Adding a theme = one entry here.

import { useEffect } from "react";

export interface AccentTheme {
  id: string;
  label: string;
  /// Solid brand colour (--color-primary / --color-ring / --color-brand).
  primary: string;
  /// Pill-button gradient endpoints.
  hi: string;
  lo: string;
  /// rgb triplet (no alpha) used to compose the hover glow shadow.
  glowRgb: string;
}

export const ACCENT_THEMES: AccentTheme[] = [
  {
    id: "amber",
    label: "Amber",
    primary: "#e0a33e",
    hi: "#e9b65f",
    lo: "#dd9f37",
    glowRgb: "224, 163, 62",
  },
  {
    id: "teal",
    label: "Teal",
    primary: "#2dd4bf",
    hi: "#54e0cf",
    lo: "#21c3ae",
    glowRgb: "45, 212, 191",
  },
  {
    id: "rose",
    label: "Rose",
    primary: "#fb7185",
    hi: "#fd8b9b",
    lo: "#f9637a",
    glowRgb: "251, 113, 133",
  },
  {
    id: "violet",
    label: "Violet",
    primary: "#a78bfa",
    hi: "#b9a4fb",
    lo: "#9b7df9",
    glowRgb: "167, 139, 250",
  },
  {
    id: "blue",
    label: "Blue",
    primary: "#60a5fa",
    hi: "#7db6fb",
    lo: "#4f9bf9",
    glowRgb: "96, 165, 250",
  },
];

export function accentTheme(id: string): AccentTheme {
  return ACCENT_THEMES.find((t) => t.id === id) ?? ACCENT_THEMES[0];
}

/// Apply a theme's brand vars to :root. Exported so it can be called eagerly
/// (e.g. live preview while the user hovers a swatch) outside the effect.
export function applyAccentTheme(id: string): void {
  const t = accentTheme(id);
  const root = document.documentElement.style;
  root.setProperty("--color-primary", t.primary);
  root.setProperty("--color-ring", t.primary);
  root.setProperty("--color-brand", t.primary);
  root.setProperty("--color-brand-hi", t.hi);
  root.setProperty("--color-brand-lo", t.lo);
  root.setProperty(
    "--shadow-brand-glow",
    `0 0 0 1px rgba(${t.glowRgb}, 0.35), 0 6px 24px rgba(${t.glowRgb}, 0.22)`
  );
}

/// Keep :root brand vars in sync with the selected accent for the app's lifetime.
export function useAccentTheme(id: string): void {
  useEffect(() => {
    applyAccentTheme(id);
  }, [id]);
}
