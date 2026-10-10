//! Compact verified stream tags. Unknown tracks stay unknown; provider labels
//! and subtitle languages never become claims about the available audio.

import { Headphones } from "lucide-react";

import { Button } from "@/components/ui/button";
import type { CanonicalMediaDetail } from "@/types";
import { useAudioLanguages, useMovieAudioTarget, type AudioTarget } from "./useAudioLanguages";

const names = new Intl.DisplayNames(["en"], { type: "language" });

function languageName(code: string): string {
  try {
    return names.of(code) ?? code.toUpperCase();
  } catch {
    return code.toUpperCase();
  }
}

export function AudioLanguageBanner({
  profileId,
  target,
  label = "Audio",
  sourceNote,
  resolving = false,
  sourceError = false,
  onRetrySource,
}: {
  profileId: string;
  target: AudioTarget | null;
  label?: string;
  sourceNote?: string;
  resolving?: boolean;
  sourceError?: boolean;
  onRetrySource?: () => void;
}) {
  const query = useAudioLanguages(profileId, target);
  const failed = sourceError || query.isError;
  const loading = resolving || (target !== null && query.isPending && !failed);
  const audio = query.data;
  const retry = () => {
    if (sourceError) onRetrySource?.();
    else void query.refetch();
  };

  return (
    <div
      aria-label={label}
      className="flex flex-wrap items-center gap-2 rounded-lg border border-white/10 bg-white/[0.04] px-3 py-2 text-xs text-white/65"
    >
      <Headphones className="h-4 w-4 shrink-0" strokeWidth={1.5} aria-hidden />
      <span className="font-medium text-white/80">{label}</span>
      {loading ? (
        <span role="status">Checking audio…</span>
      ) : failed ? (
        <>
          <span>Audio information unavailable</span>
          <Button
            variant="ghost"
            size="sm"
            onClick={retry}
            className="h-6 px-2 text-xs"
            aria-label={`Retry ${label.toLowerCase()} information`}
          >
            Retry
          </Button>
        </>
      ) : audio ? (
        <>
          {audio.languages.length > 0 && (
            <ul aria-label="Audio languages" className="flex flex-wrap gap-1.5">
              {audio.languages.map((code) => (
                <li key={code}>
                  <span
                    title={languageName(code)}
                    aria-label={languageName(code)}
                    className="inline-flex rounded border border-primary/25 bg-primary/10 px-2 py-0.5 font-medium tracking-wide text-primary"
                  >
                    {code.toUpperCase()}
                  </span>
                </li>
              ))}
            </ul>
          )}
          {audio.trackCount === 0 ? (
            <span>No audio tracks reported</span>
          ) : audio.languages.length === 0 ? (
            <span>Languages not provided</span>
          ) : audio.unknownTracks > 0 ? (
            <span>
              +{audio.unknownTracks}{" "}
              {audio.unknownTracks === 1 ? "untagged track" : "untagged tracks"}
            </span>
          ) : null}
          {sourceNote && <span className="text-white/45">· {sourceNote}</span>}
        </>
      ) : (
        <span>No playable source available</span>
      )}
    </div>
  );
}

export function MovieAudioLanguageBanner({
  profileId,
  entityId,
  detail,
  loadingDetail,
  detailError,
  onRetryDetail,
}: {
  profileId: string;
  entityId: string | null | undefined;
  detail: CanonicalMediaDetail | undefined;
  loadingDetail: boolean;
  detailError: boolean;
  onRetryDetail: () => void;
}) {
  const picked = detail?.variants.find((variant) => variant.available && variant.picked);
  const source = useMovieAudioTarget(profileId, entityId, picked?.variantId, !!detail);
  return (
    <AudioLanguageBanner
      profileId={profileId}
      target={source.data ?? null}
      sourceNote="Selected version"
      resolving={!!entityId && (loadingDetail || (!!detail && source.isPending))}
      sourceError={detailError || source.isError}
      onRetrySource={() => (detailError ? onRetryDetail() : void source.refetch())}
    />
  );
}
