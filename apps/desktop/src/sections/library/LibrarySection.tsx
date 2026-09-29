//! Library: everything you saved, across films, shows and channels. Reuses
//! the sections' own watchlist and favourite rails (same cache entries), so
//! it costs nothing extra; the history timeline arrives with UI-6.

import {
  FAVORITES_RAIL,
  WATCHLIST_RAIL,
  type BrowseTab,
  type RailSpec,
} from "@/features/browse/queries";
import { Rail } from "@/features/browse/Rail";
import { SectionHeader } from "@/ui/SectionHeader";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { useSession } from "@/app/session";
import { NoPlaylist } from "@/sections/shared/NoPlaylist";

const SHELVES: { tab: BrowseTab; spec: RailSpec }[] = [
  { tab: "movies", spec: { ...WATCHLIST_RAIL, title: "Films to watch" } },
  { tab: "series", spec: { ...WATCHLIST_RAIL, title: "Shows to watch" } },
  { tab: "movies", spec: { ...FAVORITES_RAIL, title: "Favourite films" } },
  { tab: "series", spec: { ...FAVORITES_RAIL, title: "Favourite shows" } },
  { tab: "live", spec: { ...FAVORITES_RAIL, title: "Favourite channels" } },
];

export function LibrarySection() {
  const { profile, activeSourceId, sources, sourcesLoaded } = useSession();
  const activate = useActivate();
  const { toggleFavorite, toggleWatchlist } = useLibraryActions();

  if (sourcesLoaded && sources.length === 0) return <NoPlaylist />;

  return (
    <div className="flex flex-col gap-8 px-[88px] pb-16 pt-[108px]">
      <SectionHeader
        title="Library"
        summary="Your watchlist and favourites. Add to them with + and ♡ on any title."
      />
      {activeSourceId !== null &&
        SHELVES.map(({ tab, spec }) => (
          <Rail
            key={`${tab}-${spec.id}`}
            tab={tab}
            profileId={profile.id}
            sourceId={activeSourceId}
            spec={spec}
            eager
            onActivate={activate}
            onToggleFavorite={toggleFavorite}
            onToggleWatchlist={toggleWatchlist}
          />
        ))}
    </div>
  );
}
