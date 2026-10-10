import { useEffect, useState } from "react";
import { useAudioLanguages, type AudioTarget } from "./useAudioLanguages";

export function useAudioTrackChoice(profileId: string, target: AudioTarget | null) {
  const query = useAudioLanguages(profileId, target);
  const key = `${profileId}:${target?.mediaKind}:${target?.mediaId}`;
  const [selection, setSelection] = useState<{ key: string; index: number } | null>(null);
  useEffect(() => setSelection(null), [key]);
  const selected =
    selection?.key === key
      ? query.data?.tracks?.find((track) => track.audioIndex === selection.index)
      : undefined;
  return {
    query,
    selected,
    choose: (index: number | null) => setSelection(index === null ? null : { key, index }),
    launchTrack:
      target && selected
        ? {
            ...target,
            index: selected.audioIndex,
            language: selected.language,
          }
        : undefined,
  };
}
