//! A pushed title: a canonical movie's page, a channel bundle's guide, or
//! (for a legacy row) the existing detail screen.

import { ContentDetail } from "@/features/browse/ContentDetail";
import type { BrowseItem } from "@/features/browse/queries";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { useNavActions } from "@/app/navigation/useNav";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";
import { ChannelBundlePage } from "./ChannelBundlePage";
import { MoviePage } from "./MoviePage";

export function TitlePage({ item }: { item: BrowseItem }) {
  if (item.kind === "movie" && item.entityId) return <MoviePage item={item} />;
  if (item.kind === "live" && item.groupKey && (item.memberCount ?? 1) > 1)
    return <ChannelBundlePage item={{ ...item, groupKey: item.groupKey }} />;
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
