//! Per-content "starting screen" for movies (the frame is shared with series,
//! `detail/TitleHero`): backdrop, title logo, facts (rating, runtime, age
//! rating, cast), and the actions: Play/Resume, versions, preferred audio,
//! trailer, Translate (coming later), watchlist and favourite. Live brand
//! bundles get a channel grid instead.

import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, ChevronDown, Headphones, Languages, Play } from "lucide-react";

import { Button } from "@/components/ui/button";
import { DragRegion } from "@/components/DragRegion";
import {
  DropdownMenu,
  DropdownMenuCheckItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useProfileSettings } from "@/features/settings/useProfileSettings";
import { FavoriteButton } from "./FavoriteButton";
import { WatchlistButton } from "./WatchlistButton";
import { fetchChannelsInGroup, movieToItem, type BrowseItem } from "./queries";
import { ItemRail } from "./ItemRail";
import type { CanonicalMediaDetail } from "@/types";
import { titleArtSubject, useTitleArt } from "./titleArt";
import { ChannelTile } from "./ChannelTile";
import { useChannelGuide } from "./guide";
import { GuideLine } from "./GuideViews";
import { CastStrip } from "./detail/CastStrip";
import { TitleHero } from "./detail/TitleHero";
import { TrailerButton } from "./detail/TrailerButton";
import { useRelatedTitles, useTitleFacts } from "./detail/data";
import { metaLine } from "./detail/meta";

/// Preferred audio language choices. `null` = keep the stream's original track.
const AUDIO_LANGS: { value: string | null; label: string }[] = [
  { value: null, label: "Original" },
  { value: "en", label: "English" },
  { value: "fr", label: "French" },
  { value: "ar", label: "Arabic" },
  { value: "es", label: "Spanish" },
  { value: "de", label: "German" },
  { value: "it", label: "Italian" },
  { value: "pt", label: "Portuguese" },
];

interface ContentDetailProps {
  profileId: string;
  sourceId: number | null;
  item: BrowseItem;
  onBack: () => void;
  onPlay: (item: BrowseItem) => void;
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
  onPlay: (item: BrowseItem) => void;
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
    <div className="min-h-screen bg-neutral-950 px-8 pb-6 pt-10 text-white">
      <DragRegion />
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
  const { settings, setSettings } = useProfileSettings(profileId);
  // Canonical movie detail (entity-owned resume state); the card's summary
  // renders immediately and the detail refines it when it arrives.
  const { data: detail } = useQuery({
    queryKey: ["canonical-movie", profileId, item.entityId],
    queryFn: () =>
      invoke<CanonicalMediaDetail>("get_movie", { profileId, entityId: item.entityId }),
    enabled: item.kind === "movie" && !!item.entityId,
    staleTime: 0,
  });
  const movie = detail ?? item.movie;
  const art = useTitleArt(profileId, item);
  const movieSubject = item.kind === "movie" ? titleArtSubject(item) : null;
  const { data: facts } = useTitleFacts(profileId, movieSubject);
  const { data: related } = useRelatedTitles(profileId, sourceId, movieSubject);

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
  const versions = [item, ...(item.variants ?? [])];
  const audioLabel =
    AUDIO_LANGS.find((l) => l.value === settings.preferredAudioLang)?.label ?? "Original";

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
      overview={overview ?? facts?.plot}
      onBack={onBack}
      actions={
        <>
          {/* Version picker (quality/backup variants) sits next to Play. */}
          {versions.length > 1 ? (
            <div className="flex items-center overflow-hidden rounded-md">
              <Button onClick={() => onPlay(item)} className="gap-2 rounded-r-none">
                <Play className="h-4 w-4 fill-current" /> {playLabel}
              </Button>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button className="rounded-l-none border-l border-primary-foreground/20 px-2">
                    <ChevronDown className="h-4 w-4" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="max-h-72 w-64 overflow-auto">
                  <DropdownMenuLabel>Versions</DropdownMenuLabel>
                  {versions.map((v, i) => (
                    <DropdownMenuCheckItem key={v.key} onSelect={() => onPlay(v)}>
                      {variantLabel(v, i)}
                    </DropdownMenuCheckItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          ) : (
            <Button onClick={() => onPlay(item)} className="gap-2">
              <Play className="h-4 w-4 fill-current" /> {playLabel}
            </Button>
          )}

          {/* Preferred audio language (applied when playback starts). */}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="secondary" className="gap-2">
                <Headphones className="h-4 w-4" /> Audio: {audioLabel}
                <ChevronDown className="h-4 w-4 opacity-70" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-48">
              <DropdownMenuLabel>Preferred audio</DropdownMenuLabel>
              {AUDIO_LANGS.map((l) => (
                <DropdownMenuCheckItem
                  key={l.label}
                  checked={settings.preferredAudioLang === l.value}
                  onSelect={() => setSettings({ preferredAudioLang: l.value })}
                >
                  {l.label}
                </DropdownMenuCheckItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>

          <TrailerButton url={facts?.trailerUrl} />

          {/* Translation arrives in a later phase. */}
          <Button variant="secondary" disabled className="gap-2 opacity-60">
            <Languages className="h-4 w-4" /> Translate
            <span className="rounded bg-white/15 px-1 text-[10px] uppercase">Soon</span>
          </Button>

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
