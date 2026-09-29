//! A series' page (seasons, episodes, more like this), pushed on the current
//! section's stack.

import { useMemo } from "react";

import type { SeriesSummary } from "@/types";
import { useSeriesCards } from "@/features/browse/queries";
import { SeriesDetail } from "@/features/browse/SeriesDetail";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { useNavActions } from "@/app/navigation/useNav";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";

export function SeriesPage({ series: seed }: { series: SeriesSummary }) {
  const { profile } = useSession();
  // Fresh facts and progress; the seed's flags stay (the navigation stack
  // patches them optimistically).
  const { data: fresh } = useSeriesCards(profile.id, seed.sourceId, [seed.seriesTitle]);
  const series = useMemo(() => {
    const card = fresh?.find((c) => c.seriesTitle === seed.seriesTitle);
    return card
      ? { ...card, isFavorite: seed.isFavorite, isInWatchlist: seed.isInWatchlist }
      : seed;
  }, [fresh, seed]);
  const { back } = useNavActions();
  const { playMedia } = usePlayback();
  const activate = useActivate();
  const actions = useLibraryActions();

  return (
    <SeriesDetail
      profileId={profile.id}
      series={series}
      onBack={back}
      onOpenTitle={activate}
      onToggleItemFavorite={actions.toggleFavorite}
      onToggleItemWatchlist={actions.toggleWatchlist}
      onToggleFavorite={(next) => actions.toggleSeriesFavorite(series, next)}
      onToggleWatchlist={(next) => actions.toggleSeriesWatchlist(series, next)}
      onPlayEpisode={(episode) =>
        playMedia({
          kind: "episode",
          id: episode.id,
          title: series.displayTitle,
          subtitle: episode.label,
          posterUrl: episode.stillUrl ?? series.posterUrl,
        })
      }
    />
  );
}
