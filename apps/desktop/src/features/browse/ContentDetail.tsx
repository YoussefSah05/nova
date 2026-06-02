//! Per-content "starting screen" for movies (and the header reused by series):
//! a backdrop hero with the TMDB overview/genres and the primary actions — Play,
//! preferred-audio language, Translate (coming later), favourite / watchlist, and
//! a version picker when the catalog had quality/backup duplicates.

import { ArrowLeft, ChevronDown, Headphones, Languages, Play } from "lucide-react";

import { Button } from "@/components/ui/button";
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
import type { BrowseItem } from "./queries";

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
  item: BrowseItem;
  onBack: () => void;
  onPlay: (item: BrowseItem) => void;
  onToggleFavorite: (item: BrowseItem, next: boolean) => void;
  onToggleWatchlist: (item: BrowseItem, next: boolean) => void;
}

/// Raw provider name for a variant (shows quality, e.g. "… FHD"), for the picker.
function variantLabel(it: BrowseItem, index: number): string {
  const raw = it.vod?.name ?? it.channel?.name ?? it.title;
  return raw || `Version ${index + 1}`;
}

export function ContentDetail({
  profileId,
  item,
  onBack,
  onPlay,
  onToggleFavorite,
  onToggleWatchlist,
}: ContentDetailProps) {
  const { settings, setSettings } = useProfileSettings(profileId);

  const backdrop = item.vod?.backdropUrl ?? item.posterUrl;
  const overview = item.vod?.overview ?? null;
  const versions = [item, ...(item.variants ?? [])];
  const audioLabel =
    AUDIO_LANGS.find((l) => l.value === settings.preferredAudioLang)?.label ?? "Original";

  return (
    <div className="relative min-h-screen bg-neutral-950 text-white">
      {/* Backdrop */}
      <div className="absolute inset-x-0 top-0 h-[60vh] overflow-hidden">
        {backdrop ? (
          <img src={backdrop} alt="" className="h-full w-full object-cover object-top" />
        ) : (
          <div className="h-full w-full bg-gradient-to-br from-primary/20 via-neutral-900 to-neutral-950" />
        )}
        <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/60 to-transparent" />
        <div className="absolute inset-0 bg-gradient-to-r from-neutral-950/85 to-transparent" />
      </div>

      <div className="relative z-10 flex flex-col gap-6 px-8 pb-12 pt-6">
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
              className="hidden h-52 w-36 shrink-0 rounded-xl object-cover shadow-2xl sm:block"
            />
          )}
          <div className="min-w-0 max-w-2xl">
            <h1 className="text-3xl font-semibold tracking-tight drop-shadow-lg md:text-4xl">
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
                <Play className="h-4 w-4 fill-current" /> Play
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
              <Play className="h-4 w-4 fill-current" /> Play
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
      </div>
    </div>
  );
}
