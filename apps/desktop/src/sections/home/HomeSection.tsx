//! Home, "Tonight": an editorial front page of mixed modules rather than a
//! stack of identical rails — what to resume, what is on air, what to watch
//! next, what is new, what is trending, and more like what you just watched.
//! Every module reads the browse caches, which the session warms at start.

import { RECENTLY_ADDED_RAIL, useContinueWatching, useRail } from "@/features/browse/queries";
import { useSession } from "@/app/session";
import { NoPlaylist } from "@/sections/shared/NoPlaylist";
import { BecauseYouWatched, TrendingRanked } from "./Discovery";
import { homeFeed } from "./homeFeed";
import { OnAirNow } from "./OnAirNow";
import { ResumeHero } from "./ResumeHero";
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
  // The spotlight (when nothing is in progress) is the newest film with art.
  const recent = useRail("movies", profile.id, sourceId, RECENTLY_ADDED_RAIL, true).data ?? [];
  const spotlight = recent.find((m) => m.movie?.backdropUrl) ?? recent[0] ?? null;

  return (
    <div className="flex flex-col gap-14 pb-20">
      <ResumeHero item={feed.hero} alsoInProgress={feed.alsoInProgress} spotlight={spotlight} />
      <OnAirNow recent={feed.recentChannels} />
      <UpNextAndNew upNext={feed.upNext} />
      <TrendingRanked />
      <BecauseYouWatched seed={feed.seedMovie} />
    </div>
  );
}
