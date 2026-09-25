import { useProfilePreference } from "@/lib/useProfilePreference";
import { DEFAULT_CAPTION_STYLE, type CaptionStyle } from "@/types";

export interface CaptionStyleState {
  style: CaptionStyle;
  setStyle: (patch: Partial<CaptionStyle>) => void;
}

export function useCaptionStyle(profileId: string): CaptionStyleState {
  const { value, update } = useProfilePreference(
    profileId,
    DEFAULT_CAPTION_STYLE,
    "get_caption_style",
    "set_caption_style",
    "style"
  );
  return { style: value, setStyle: update };
}
