//! A movie's page, or a channel brand bundle's channel list, pushed on the
//! current section's stack.

import { ContentDetail } from "@/features/browse/ContentDetail";
import type { BrowseItem } from "@/features/browse/queries";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { useNavActions } from "@/app/navigation/useNav";
import { usePlayback } from "@/app/playback";
import { useSession } from "@/app/session";

export function TitlePage({ item }: { item: BrowseItem }) {
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
