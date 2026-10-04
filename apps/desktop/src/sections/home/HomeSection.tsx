//! Home, "Tonight": an editorial front page of mixed modules rather than a
//! stack of identical rails — what to resume, what is on air, what to watch
//! next, what is new, what is trending, and more like what you just watched.
//! Every module reads the browse caches, which the session warms at start.

import { useMemo } from "react";
import { RECENTLY_ADDED_RAIL, useContinueWatching, useRail } from "@/features/browse/queries";
import { useSession } from "@/app/session";
import { NoPlaylist } from "@/sections/shared/NoPlaylist";
import { BecauseYouWatched, TrendingRanked } from "./Discovery";
import { homeFeed } from "./homeFeed";
import { OnAirNow } from "./OnAirNow";
import { HomeShowcase } from "./HomeShowcase";
import { showcaseSlides } from "./showcase";
import { UpNextAndNew } from "./UpNextAndNew";

export function HomeSection() {
  const { sources, sourcesLoaded, activeSourceId } = useSession();
  if (sourcesLoaded && sources.length === 0) return <NoPlaylist />;
  if (activeSourceId === null) return null;
  return <HomeFeedView sourceId={activeSourceId} />;
}

function HomeFeedView({ sourceId }: { sourceId: number }) {
  const { profile } = useSession();
  const movies = useContinueWatching(profile.id, sourceId, "movies").data;
  const series = useContinueWatching(profile.id, sourceId, "series").data;
  const live = useContinueWatching(profile.id, sourceId, "live").data;
  const feed = homeFeed(movies, series, live);
  const recent = useRail("movies", profile.id, sourceId, RECENTLY_ADDED_RAIL, true).data;
  const recentSeries = useRail("series", profile.id, sourceId, RECENTLY_ADDED_RAIL, true).data;
  const slides = useMemo(
    () => showcaseSlides(feed.hero, recent ?? [], recentSeries ?? []),
    [feed.hero, recent, recentSeries]
  );

  return (
    <div className="flex flex-col gap-14 pb-20">
      <HomeShowcase
        key={`${profile.id}:${sourceId}`}
        slides={slides}
        alsoInProgress={feed.alsoInProgress}
      />
      <OnAirNow recent={feed.recentChannels} />
      <UpNextAndNew upNext={feed.upNext} />
      <TrendingRanked />
      <BecauseYouWatched seed={feed.seedMovie} />
    </div>
  );
}
