//! How a film's sources are presented: available first with Nova's pick on
//! top, and a plain reason for each — why Nova plays it, why it comes later,
//! or why it can't play — from the same evidence the variant selector uses.

import type { VariantSummary } from "@/types";

const QUALITY = ["4K", "FHD", "HD", "SD"];
const AUDIO_TAG: Record<string, string> = {
  VF: "fr",
  MULTI: "multi",
  VOSTFR: "fr-subs",
  VO: "original",
};

const LANGUAGE_NAMES: Record<string, string> = {
  en: "English",
  fr: "French",
  ar: "Arabic",
  es: "Spanish",
  de: "German",
  it: "Italian",
  pt: "Portuguese",
};

function quality(v: VariantSummary): string | null {
  return v.badges.find((b) => QUALITY.includes(b)) ?? null;
}

/// Order: Nova's pick, then playable sources, then recently failed, then gone.
export function orderVersions(variants: VariantSummary[]): VariantSummary[] {
  const rank = (v: VariantSummary) => (v.picked ? 0 : !v.available ? 3 : v.recentlyFailed ? 2 : 1);
  return [...variants].sort((a, b) => rank(a) - rank(b));
}

/// One line explaining a source's place.
export function versionReason(v: VariantSummary, preferredLang: string | null): string {
  if (!v.available) return "No longer listed by the provider.";
  if (v.recentlyFailed) return "Failed recently — tried last.";
  const q = quality(v);
  if (v.picked) {
    const lang = preferredLang ? (LANGUAGE_NAMES[preferredLang] ?? preferredLang) : null;
    const audio = v.badges.some((b) => AUDIO_TAG[b] === preferredLang || b === "MULTI");
    if (lang && audio) return `Nova plays this one: ${q ? `${q} with ` : ""}${lang} audio.`;
    if (q === "FHD") return "Nova plays this one: full HD, the steadiest picture over IPTV.";
    if (q) return `Nova plays this one: the best ${q} source that plays reliably.`;
    return "Nova plays this one by default.";
  }
  if (q === "4K") return "4K streams stall more often over IPTV.";
  if (q === "SD") return "Lower picture quality.";
  return "Another source of the same film.";
}
