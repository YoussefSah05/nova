import { useAudioTrackChoice } from "./detail/useAudioTrackChoice";
//! A series' page, on the same frame as a movie's: backdrop and title logo,
//! a meta line (years · rating · seasons · score), tagline and overview, the
//! main action (Resume / Next episode / Play S01E01), trailer, watchlist and
//! favourite, how far through the show the profile is, the cast, then season
//! tabs (posters when TMDB has them) with rich episode rows.

import { useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import type { EpisodeView, SeriesSummary } from "@/types";
import { FavoriteButton } from "./FavoriteButton";
import { WatchlistButton } from "./WatchlistButton";
import { useTitleArtFor, type TitleArtSubject } from "./titleArt";
import { CastStrip } from "./detail/CastStrip";
import { EpisodeList } from "./detail/EpisodeList";
import { SeasonTabs } from "./detail/SeasonTabs";
import { ShowProgress } from "./detail/ShowProgress";
import { TitleHero } from "./detail/TitleHero";
import { TrailerButton } from "./detail/TrailerButton";
import type { PlaybackLaunchOptions } from "@/features/player/playbackLaunch";
import { AudioTrackPicker, PlayWithCaptions } from "./detail/PlaybackChoices";
import { AudioLanguageBanner } from "./detail/AudioLanguageBanner";
import {
  useRelatedTitles,
  useSeasonEpisodes,
  useSeriesEpisodes,
  useSeriesResume,
  useTitleFacts,
} from "./detail/data";
import { episodeViewLabel, metaLine, seriesPrimaryAction } from "./detail/meta";
import { ItemRail } from "./ItemRail";
import { seriesToItem, type BrowseItem } from "./queries";

export interface PlayableEpisode {
  id: number;
  /// `S02E03 — Title`.
  label: string;
  stillUrl: string | null;
}

interface SeriesDetailProps {
  profileId: string;
  series: SeriesSummary;
  onBack: () => void;
  onToggleFavorite: (next: boolean) => Promise<void> | void;
  onToggleWatchlist: (next: boolean) => Promise<void> | void;
  onPlayEpisode: (episode: PlayableEpisode, options?: PlaybackLaunchOptions) => void;
  /// Open another title and toggle its flags (from "More like this").
  onOpenTitle: (item: BrowseItem) => void;
  onToggleItemFavorite: (item: BrowseItem, next: boolean) => void;
  onToggleItemWatchlist: (item: BrowseItem, next: boolean) => void;
}

function formatDate(date: string): string {
  const d = new Date(`${date}T00:00:00`);
  return Number.isNaN(d.getTime())
    ? date
    : d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

export function SeriesDetail({
  profileId,
  series,
  onBack,
  onToggleFavorite,
  onToggleWatchlist,
  onPlayEpisode,
  onOpenTitle,
  onToggleItemFavorite,
  onToggleItemWatchlist,
}: SeriesDetailProps) {
  const subject: TitleArtSubject | null = useMemo(
    () => (series.entityId ? { kind: "series", seriesId: series.entityId } : null),
    [series.entityId]
  );
  const art = useTitleArtFor(profileId, subject);
  const { data: facts } = useTitleFacts(profileId, subject);
  const all = useSeriesEpisodes(profileId, series);
  const { data: resume } = useSeriesResume(profileId, series);
  const { data: related } = useRelatedTitles(profileId, series.sourceId, subject);

  // Seasons the provider actually has, regular seasons first, specials last.
  const seasons = useMemo(() => {
    const numbers = [...new Set((all.data ?? []).map((e) => e.season))];
    return numbers.sort((a, b) => (a === 0 ? 1 : b === 0 ? -1 : a - b));
  }, [all.data]);
  const [season, setSeason] = useState<number | null>(null);
  // Open on the season being watched, else the first one.
  useEffect(() => {
    if (season !== null && seasons.includes(season)) return;
    const preferred = resume?.season ?? null;
    setSeason(preferred !== null && seasons.includes(preferred) ? preferred : (seasons[0] ?? null));
  }, [seasons, resume?.season, season]);
  const episodes = useSeasonEpisodes(profileId, series, season);

  const first = useMemo(
    () => (all.data ?? []).find((e) => e.season === seasons[0]),
    [all.data, seasons]
  );
  const action = seriesPrimaryAction(resume, first);
  const seasonName = (n: number) =>
    facts?.seasons.find((s) => s.number === n)?.name ?? (n === 0 ? "Specials" : `Season ${n}`);

  const audioTarget = action ? { mediaKind: "episode" as const, mediaId: action.episodeId } : null;
  const audioChoice = useAudioTrackChoice(profileId, audioTarget);
  const playEpisode = (ep: EpisodeView, options?: PlaybackLaunchOptions) =>
    onPlayEpisode({ id: ep.id, label: episodeViewLabel(ep, true), stillUrl: ep.stillUrl }, options);
  const playAction = (options: PlaybackLaunchOptions) => {
    if (!action) return;
    const listed = episodes.data?.find((e) => e.id === action.episodeId);
    if (listed) return playEpisode(listed, options);
    const label =
      resume?.episodeTitle && resume.mediaId === action.episodeId
        ? `${action.episodeCode} — ${resume.episodeTitle}`
        : action.episodeCode;
    onPlayEpisode({ id: action.episodeId, label, stillUrl: resume?.posterUrl ?? null }, options);
  };

  const next = facts?.nextEpisode;
  const note = next
    ? `New episode S${String(next.season).padStart(2, "0")}E${String(next.episode).padStart(2, "0")} · ${formatDate(next.airDate)}`
    : null;

  return (
    <TitleHero
      title={series.displayTitle}
      posterUrl={series.posterUrl}
      backdropUrl={art?.backdropUrl ?? series.backdropUrl}
      logoUrl={art?.logoUrl}
      meta={metaLine(facts, {
        kind: "series",
        seasonCount: seasons.filter((s) => s > 0).length || series.seasonCount,
        genre: series.category,
      })}
      tagline={facts?.tagline}
      audioBanner={
        <AudioLanguageBanner
          profileId={profileId}
          target={action ? { mediaKind: "episode", mediaId: action.episodeId } : null}
          label={action?.label.startsWith("Resume") ? "Resume episode audio" : "Next episode audio"}
          resolving={series.entityId !== null && all.isPending}
        />
      }
      overview={series.overview ?? facts?.plot}
      note={note}
      onBack={onBack}
      actions={
        <>
          <PlayWithCaptions
            onPlay={playAction}
            disabled={!action}
            label={action?.label ?? "Play"}
            audioTrack={audioChoice.launchTrack}
            prepare={
              action
                ? {
                    profileId,
                    mediaKind: "episode",
                    mediaId: action.episodeId,
                    fromSec:
                      resume && !resume.upNext && resume.mediaId === action.episodeId
                        ? resume.positionSec
                        : 0,
                  }
                : null
            }
          />
          <AudioTrackPicker choice={audioChoice} resolving={all.isPending} />
          <TrailerButton url={facts?.trailerUrl} />
          <WatchlistButton
            inWatchlist={series.isInWatchlist}
            onToggle={onToggleWatchlist}
            label={series.displayTitle}
          />
          <FavoriteButton
            isFavorite={series.isFavorite}
            onToggle={onToggleFavorite}
            label={series.displayTitle}
          />
        </>
      }
    >
      <ShowProgress series={series} />

      <CastStrip
        cast={facts?.cast ?? []}
        people={facts?.directors ?? []}
        peopleLabel="Created by"
      />

      <section className="flex flex-col gap-3">
        {all.isError ? (
          <div className="flex flex-col items-start gap-3 rounded-xl bg-neutral-900 px-4 py-5">
            <p className="text-sm text-white/70">Couldn't load episodes.</p>
            <Button variant="secondary" size="sm" onClick={() => void all.refetch()}>
              Retry
            </Button>
          </div>
        ) : (
          <>
            {seasons.length > 1 && (
              <SeasonTabs
                seasons={seasons}
                selected={season}
                details={facts?.seasons}
                nameOf={seasonName}
                onSelect={setSeason}
              />
            )}
            {seasons.length === 1 && season !== null && (
              <h2 className="text-sm font-medium text-white/85">{seasonName(season)}</h2>
            )}
            <EpisodeList
              episodes={episodes.data}
              loading={all.isLoading || episodes.isLoading}
              currentId={action?.episodeId}
              onPlay={(episode) => playEpisode(episode)}
            />
          </>
        )}
      </section>

      <ItemRail
        title="More like this"
        items={(related?.series ?? []).map(seriesToItem)}
        onActivate={onOpenTitle}
        onToggleFavorite={onToggleItemFavorite}
        onToggleWatchlist={onToggleItemWatchlist}
      />
    </TitleHero>
  );
}
