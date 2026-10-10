import { useAudioTrackChoice } from "./detail/useAudioTrackChoice";
//! Per-content "starting screen" for movies (the frame is shared with series,
//! `detail/TitleHero`): backdrop, title logo, facts (rating, runtime, age
//! rating, cast), and the actions: Play/Resume, versions, preferred audio,
//! trailer, Translate (coming later), watchlist and favourite. Live brand
//! bundles get a channel grid instead.

import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, ChevronDown } from "lucide-react";

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
import { fetchChannelsInGroup, movieToItem, type BrowseItem } from "./queries";
import { ItemRail } from "./ItemRail";
import { titleArtSubject, useTitleArt } from "./titleArt";
import { ChannelTile } from "./ChannelTile";
import { useChannelGuide } from "./guide";
import { GuideLine } from "./GuideViews";
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

/// Live brand bundle: a backdrop-less header + a grid of the bundle's channels.
function LiveBundleDetail({
  profileId,
  sourceId,
  item,
  onBack,
  onPlay,
}: {
  profileId: string;
  sourceId: number | null;
  item: BrowseItem;
  onBack: () => void;
  onPlay: (item: BrowseItem, options?: PlaybackLaunchOptions) => void;
}) {
  const [channels, setChannels] = useState<BrowseItem[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (sourceId == null || !item.groupKey) return;
    let cancelled = false;
    setLoading(true);
    fetchChannelsInGroup(profileId, sourceId, item.groupKey)
      .then((c) => !cancelled && setChannels(c))
      .catch(() => !cancelled && setChannels([]))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [profileId, sourceId, item.groupKey]);

  return (
    <div className="min-h-screen bg-neutral-950 px-[88px] pb-6 pt-[96px] text-white">
      <Button
        variant="ghost"
        size="sm"
        onClick={onBack}
        className="gap-2 text-white/70 hover:text-white"
      >
        <ArrowLeft className="h-4 w-4" /> Back
      </Button>
      <div className="mt-4 flex items-center gap-4">
        <div className="h-16 w-28 shrink-0 overflow-hidden rounded-xl shadow-lg shadow-black/40">
          <ChannelTile logoUrl={item.posterUrl} title={item.title} />
        </div>
        <div>
          <h1 className="font-display text-2xl font-semibold">{item.title}</h1>
          <p className="mt-1 text-sm text-white/55">
            {channels.length || item.memberCount} channels
            {item.subtitle ? ` · ${item.subtitle}` : ""}
          </p>
        </div>
      </div>

      {loading ? (
        <p className="mt-8 text-sm text-white/50">Loading channels…</p>
      ) : (
        <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5">
          {channels.map((c) => (
            <BundleChannel key={c.key} item={c} onPlay={onPlay} />
          ))}
        </div>
      )}
    </div>
  );
}

/// One channel of a brand bundle, with what it's showing now.
function BundleChannel({ item, onPlay }: { item: BrowseItem; onPlay: (item: BrowseItem) => void }) {
  const guide = useChannelGuide(item.id);
  return (
    <button
      onClick={() => onPlay(item)}
      className="group flex flex-col gap-2 text-left focus:outline-none"
    >
      <div className="aspect-video w-full overflow-hidden rounded-xl shadow-lg shadow-black/30 transition-transform duration-200 group-hover:-translate-y-1 group-hover:ring-1 group-hover:ring-white/25 group-focus-visible:ring-2 group-focus-visible:ring-ring">
        <ChannelTile logoUrl={item.posterUrl} title={item.title} />
      </div>
      <span className="line-clamp-2 text-xs font-medium text-white/85">{item.title}</span>
      <GuideLine guide={guide} />
    </button>
  );
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
