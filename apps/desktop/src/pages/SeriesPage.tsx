//! A series' page (seasons, episodes, more like this), pushed on the current
//! section's stack.

import type { SeriesSummary } from "@/types";
import { SeriesDetail } from "@/features/browse/SeriesDetail";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { useNavActions } from "@/app/navigation/useNav";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";

export function SeriesPage({ series }: { series: SeriesSummary }) {
  const { profile } = useSession();
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
