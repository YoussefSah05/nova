import type { CaptionMode } from "./CaptionOverlay";

/** Per-request choices. They never update a profile or choose a provider version. */
export interface PlaybackLaunchOptions {
  captionMode?: CaptionMode;
  translationTarget?: "en" | "fr" | "ar";
  audioTrack?: {
    mediaKind: "vod" | "episode" | "live";
    mediaId: number;
    index: number;
    language: string | null;
  };
}

export function canonicalTrackLanguage(tag: string | null | undefined): string | null {
  if (!tag || ["und", "mul", "zxx", "mis", "unk"].includes(tag.toLowerCase())) return null;
  try {
    return new Intl.Locale(tag.trim().replaceAll("_", "-")).language.toLowerCase();
  } catch {
    return null;
  }
}

export function initialCaptionMode(
  options: PlaybackLaunchOptions | undefined,
  profileMode: string
): CaptionMode {
  return options?.captionMode ?? (profileMode === "off" ? "off" : "source");
}
