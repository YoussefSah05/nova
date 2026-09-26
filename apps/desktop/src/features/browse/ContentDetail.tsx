//! Per-content "starting screen" for movies (and the header reused by series):
//! a backdrop hero with the TMDB overview/genres and the primary actions — Play,
//! preferred-audio language, Translate (coming later), favourite / watchlist, and
//! a version picker when the catalog had quality/backup duplicates.

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
import { AmbientGlow, useAmbientColor } from "@/lib/ambient";
import { motion, useReducedMotion, EASE_ENTER } from "@/lib/motion";
import { FavoriteButton } from "./FavoriteButton";
import { WatchlistButton } from "./WatchlistButton";
import { fetchChannelsInGroup, type BrowseItem } from "./queries";
import type { CanonicalMediaDetail } from "@/types";

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
        {item.posterUrl && (
          <img
            src={item.posterUrl}
            alt=""
            decoding="async"
            className="h-16 w-28 rounded-lg object-contain bg-neutral-900 p-2"
          />
        )}
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
            <button
              key={c.key}
              onClick={() => onPlay(c)}
              className="flex flex-col items-center gap-2 rounded-xl border border-white/10 bg-white/[0.03] p-3 text-center transition-colors hover:bg-white/10"
            >
              <div className="flex h-16 w-full items-center justify-center">
                {c.posterUrl ? (
                  <img
                    src={c.posterUrl}
                    alt=""
                    loading="lazy"
                    decoding="async"
                    className="max-h-16 max-w-full object-contain"
                  />
                ) : (
                  <Play className="h-6 w-6 text-white/40" />
                )}
              </div>
              <span className="line-clamp-2 text-xs font-medium text-white/85">{c.title}</span>
            </button>
          ))}
        </div>
      )}
    </div>
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
}: ContentDetailProps) {
  const { settings, setSettings } = useProfileSettings(profileId);
  const reduce = useReducedMotion();
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
  const ambient = useAmbientColor(movie?.backdropUrl ?? item.vod?.backdropUrl ?? item.posterUrl);

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

  const backdrop = movie?.backdropUrl ?? item.vod?.backdropUrl ?? item.posterUrl;
  const overview = movie?.overview ?? item.vod?.overview ?? null;
  const resume = detail?.resume && !detail.resume.completed ? detail.resume : null;
  const playLabel = resume ? `Resume · ${formatClock(resume.positionSec)}` : "Play";
  const versions = [item, ...(item.variants ?? [])];
  const audioLabel =
    AUDIO_LANGS.find((l) => l.value === settings.preferredAudioLang)?.label ?? "Original";

  return (
    <div className="relative min-h-screen bg-neutral-950 text-white">
      <DragRegion />
      {/* Backdrop */}
      <div className="absolute inset-x-0 top-0 h-[60vh] overflow-hidden">
        {backdrop ? (
          <motion.img
            src={backdrop}
            alt=""
            decoding="async"
            className="h-full w-full object-cover object-top"
            initial={reduce ? false : { scale: 1.06, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            transition={{ duration: 0.6, ease: EASE_ENTER }}
          />
        ) : (
          <div className="h-full w-full bg-gradient-to-br from-primary/20 via-neutral-900 to-neutral-950" />
        )}
        <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/60 to-transparent" />
        <div className="absolute inset-0 bg-gradient-to-r from-neutral-950/85 to-transparent" />
      </div>

      {/* Ambient tint from the artwork's own palette — a coloured aura over the
          darkened hero that bleeds down into the page (screen-blended so it adds
          light rather than covering content). */}
      <AmbientGlow color={ambient} className="h-[70vh] mix-blend-screen" />

      <motion.div
        className="relative z-10 flex flex-col gap-6 px-8 pb-12 pt-10"
        initial={reduce ? false : { y: 16, opacity: 0 }}
        animate={{ y: 0, opacity: 1 }}
        transition={{ duration: 0.4, ease: EASE_ENTER, delay: 0.08 }}
      >
        <Button
          variant="ghost"
          size="sm"
          onClick={onBack}
          className="w-fit gap-2 text-white/70 hover:text-white"
        >
          <ArrowLeft className="h-4 w-4" /> Back
        </Button>

        <div className="mt-[34vh] flex items-end gap-5">
          {item.posterUrl && (
            <img
              src={item.posterUrl}
              alt=""
              loading="lazy"
              decoding="async"
              className="hidden h-52 w-36 shrink-0 rounded-xl object-cover shadow-2xl sm:block"
            />
          )}
          <div className="min-w-0 max-w-2xl">
            <h1 className="font-display text-3xl font-semibold tracking-tight drop-shadow-lg md:text-4xl">
              {item.title}
            </h1>
            {item.subtitle && <p className="mt-2 text-sm text-white/70">{item.subtitle}</p>}
            {overview && (
              <p className="mt-3 line-clamp-4 max-w-xl text-sm leading-6 text-white/75">
                {overview}
              </p>
            )}
          </div>
        </div>

        {/* Actions */}
        <div className="flex flex-wrap items-center gap-3">
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
        </div>
      </motion.div>
    </div>
  );
}
