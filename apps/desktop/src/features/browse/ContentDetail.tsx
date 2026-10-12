import { useAudioTrackChoice } from "./detail/useAudioTrackChoice";
//! Per-content "starting screen" for movies (the frame is shared with series,
//! `detail/TitleHero`): backdrop, title logo, facts (rating, runtime, age
//! rating, cast), and the actions: Play/Resume, versions, preferred audio,
//! trailer, Translate (coming later), watchlist and favourite. Live brand
//! bundles get a channel grid instead.

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ChevronDown } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { FavoriteButton } from "./FavoriteButton";
import { WatchlistButton } from "./WatchlistButton";
import { movieToItem, type BrowseItem } from "./queries";
import { ItemRail } from "./ItemRail";
import { titleArtSubject, useTitleArt } from "./titleArt";
import { LiveBundleDetail } from "./LiveBundleDetail";
import { CastStrip } from "./detail/CastStrip";
import { TitleHero } from "./detail/TitleHero";
import { TrailerButton } from "./detail/TrailerButton";
import { canonicalMovieQuery, useRelatedTitles, useTitleFacts } from "./detail/data";
import { metaLine } from "./detail/meta";
import { AudioLanguageBanner } from "./detail/AudioLanguageBanner";
import type { PlaybackLaunchOptions } from "@/features/player/playbackLaunch";
import { AudioTrackPicker, PlayWithCaptions } from "./detail/PlaybackChoices";
import { useMovieAudioTarget } from "./detail/useAudioLanguages";
import { useScreenActive } from "@/lib/screenActive";

interface ContentDetailProps {
  profileId: string;
  sourceId: number | null;
  item: BrowseItem;
  onBack: () => void;
  onPlay: (item: BrowseItem, options?: PlaybackLaunchOptions) => void;
  onToggleFavorite: (item: BrowseItem, next: boolean) => void;
  onToggleWatchlist: (item: BrowseItem, next: boolean) => void;
  /// Open another title (from "More like this" or the collection).
  onOpenTitle: (item: BrowseItem) => void;
}

/// `m:ss` / `h:mm:ss` for a resume hint.
function formatClock(totalSec: number): string {
  const s = Math.max(0, Math.floor(totalSec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

/// Raw provider name for a variant (shows quality, e.g. "… FHD"), for the picker.
function variantLabel(it: BrowseItem, index: number): string {
  const raw = it.movie?.sourceTitle ?? it.vod?.name ?? it.channel?.name ?? it.title;
  return raw || `Version ${index + 1}`;
}

export function ContentDetail({
  profileId,
  sourceId,
  item,
  onBack,
  onPlay,
  onToggleFavorite,
  onToggleWatchlist,
  onOpenTitle,
}: ContentDetailProps) {
  const [version, setVersion] = useState<{ owner: string; key: string } | null>(null);
  // Canonical movie detail (entity-owned resume state); the card's summary
  // renders immediately and the detail refines it when it arrives.
  const detailQuery = useQuery({
    subscribed: useScreenActive(),
    ...canonicalMovieQuery(profileId, item.entityId),
    enabled: item.kind === "movie" && !!item.entityId,
    staleTime: 0,
  });
  const { data: detail } = detailQuery;
  const movie = detail ?? item.movie;
  const art = useTitleArt(profileId, item);
  const movieSubject = item.kind === "movie" ? titleArtSubject(item) : null;
  const { data: facts } = useTitleFacts(profileId, movieSubject);
  const { data: related } = useRelatedTitles(profileId, sourceId, movieSubject);

  const versions = [item, ...(item.variants ?? [])];
  const selectedVersion =
    version?.owner === item.key ? (versions.find((v) => v.key === version.key) ?? item) : item;
  const picked = detail?.variants.find((variant) => variant.available && variant.picked);
  const source = useMovieAudioTarget(
    profileId,
    item.entityId,
    picked?.variantId,
    item.kind === "movie" && !!detail
  );
  const audioTarget =
    selectedVersion.kind === "vod"
      ? { mediaKind: "vod" as const, mediaId: selectedVersion.id }
      : (source.data ?? null);
  const audioChoice = useAudioTrackChoice(profileId, audioTarget);
  const resolvingAudio =
    item.kind === "movie" && (detailQuery.isPending || (!!detail && source.isPending));
  const audioError = item.kind === "movie" && (detailQuery.isError || source.isError);
  const retryAudio = () =>
    detailQuery.isError ? void detailQuery.refetch() : void source.refetch();

  // Live brand bundle → channel grid; movies → the overview hero below.
  if (item.kind === "live") {
    return (
      <LiveBundleDetail
        key={`${profileId}:${sourceId}:${item.groupKey}`}
        profileId={profileId}
        sourceId={sourceId}
        item={item}
        onBack={onBack}
        onPlay={onPlay}
      />
    );
  }

  const overview = movie?.overview ?? item.vod?.overview ?? null;
  const resume = detail?.resume && !detail.resume.completed ? detail.resume : null;
  const playLabel = resume ? `Resume · ${formatClock(resume.positionSec)}` : "Play";

  return (
    <TitleHero
      title={item.title}
      posterUrl={item.posterUrl}
      backdropUrl={
        art?.backdropUrl ?? movie?.backdropUrl ?? item.vod?.backdropUrl ?? item.posterUrl
      }
      logoUrl={art?.logoUrl}
      meta={metaLine(facts, { kind: "movie", year: movie?.year, genre: movie?.category })}
      tagline={facts?.tagline}
      audioBanner={
        <AudioLanguageBanner
          profileId={profileId}
          target={audioTarget}
          sourceNote="Selected version"
          resolving={resolvingAudio}
          sourceError={audioError}
          onRetrySource={retryAudio}
        />
      }
      overview={overview ?? facts?.plot}
      onBack={onBack}
      actions={
        <>
          <PlayWithCaptions
            label={playLabel}
            audioTrack={audioChoice.launchTrack}
            onPlay={(options) => onPlay(selectedVersion, options)}
          />
          {versions.length > 1 && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="secondary" className="gap-2">
                  Version: {variantLabel(selectedVersion, versions.indexOf(selectedVersion))}
                  <ChevronDown className="h-4 w-4" aria-hidden />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="max-h-72 w-64 overflow-auto">
                <DropdownMenuLabel>Versions</DropdownMenuLabel>
                {versions.map((v, i) => (
                  <DropdownMenuCheckItem
                    key={v.key}
                    checked={v.key === selectedVersion.key}
                    onSelect={() => setVersion({ owner: item.key, key: v.key })}
                  >
                    {variantLabel(v, i)}
                  </DropdownMenuCheckItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          <AudioTrackPicker
            choice={audioChoice}
            resolving={resolvingAudio}
            sourceError={audioError}
            onRetrySource={retryAudio}
          />
          <TrailerButton url={facts?.trailerUrl} />

          <WatchlistButton
            inWatchlist={item.isInWatchlist}
            onToggle={(next) => onToggleWatchlist(item, next)}
            label={item.title}
          />
          <FavoriteButton
            isFavorite={item.isFavorite}
            onToggle={(next) => onToggleFavorite(item, next)}
            label={item.title}
          />
        </>
      }
    >
      <CastStrip
        cast={facts?.cast ?? []}
        people={facts?.directors ?? []}
        peopleLabel="Directed by"
      />
      <ItemRail
        title={related?.collectionName ?? "Collection"}
        items={(related?.collection ?? []).map(movieToItem)}
        onActivate={onOpenTitle}
        onToggleFavorite={onToggleFavorite}
        onToggleWatchlist={onToggleWatchlist}
      />
      <ItemRail
        title="More like this"
        items={(related?.movies ?? []).map(movieToItem)}
        onActivate={onOpenTitle}
        onToggleFavorite={onToggleFavorite}
        onToggleWatchlist={onToggleWatchlist}
      />
    </TitleHero>
  );
}
