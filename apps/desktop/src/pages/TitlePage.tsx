//! A pushed title: a canonical movie's page, or (for a channel brand bundle
//! or a legacy row) the existing detail screen.

import { ContentDetail } from "@/features/browse/ContentDetail";
import type { BrowseItem } from "@/features/browse/queries";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { useNavActions } from "@/app/navigation/useNav";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";
import { MoviePage } from "./MoviePage";

export function TitlePage({ item }: { item: BrowseItem }) {
  if (item.kind === "movie" && item.entityId) return <MoviePage item={item} />;
  return <LegacyTitlePage item={item} />;
}

function LegacyTitlePage({ item }: { item: BrowseItem }) {
  const { profile, activeSourceId } = useSession();
  const { back } = useNavActions();
  const { playItem } = usePlayback();
  const activate = useActivate();
  const { toggleFavorite, toggleWatchlist } = useLibraryActions();

  return (
    <ContentDetail
      profileId={profile.id}
      sourceId={activeSourceId}
      item={item}
      onBack={back}
      onPlay={playItem}
      onToggleFavorite={toggleFavorite}
      onToggleWatchlist={toggleWatchlist}
      onOpenTitle={activate}
    />
  );
}
