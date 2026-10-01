import { useDeferredValue, useMemo, useState } from "react";
import { BookmarkIcon, HeartIcon, HistoryIcon } from "lucide-react";

import { useSession } from "@/app/session";
import { useActivate } from "@/app/useActivate";
import { useLibraryActions } from "@/app/libraryActions";
import { useScreenScroll } from "@/app/screenContext";
import { useNavActions } from "@/app/navigation/useNav";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SimpleSelect } from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { BrowseBoundary } from "@/features/browse/BrowseBoundary";
import { BrowseGrid } from "@/features/browse/BrowseGrid";
import type { BrowseFilters, BrowseTab } from "@/features/browse/queries";
import { NoPlaylist } from "@/sections/shared/NoPlaylist";
import { SectionHeader } from "@/ui/SectionHeader";
import { LibraryHistory } from "./LibraryHistory";

export function LibrarySection() {
  const { profile, activeSourceId, sources, sourcesLoaded } = useSession();
  if (sourcesLoaded && sources.length === 0) return <NoPlaylist />;
  return (
    <div className="flex flex-col gap-8 px-[88px] pb-16 pt-[108px]">
      <SectionHeader
        title="Library"
        summary="Saved for later. Loved for longer. Watched along the way."
      />
      {activeSourceId !== null && (
        <LibraryViews
          key={`${profile.id}:${activeSourceId}`}
          profileId={profile.id}
          sourceId={activeSourceId}
        />
      )}
    </div>
  );
}

function LibraryViews({ profileId, sourceId }: { profileId: string; sourceId: number }) {
  return (
    <Tabs defaultValue="watchlist" className="flex flex-col gap-8">
      <TabsList aria-label="Library views" className="self-start">
        <TabsTrigger value="watchlist">
          <BookmarkIcon size={16} strokeWidth={1.5} aria-hidden />
          Watchlist
        </TabsTrigger>
        <TabsTrigger value="favourites">
          <HeartIcon size={16} strokeWidth={1.5} aria-hidden />
          Favourites
        </TabsTrigger>
        <TabsTrigger value="history">
          <HistoryIcon size={16} strokeWidth={1.5} aria-hidden />
          History
        </TabsTrigger>
      </TabsList>
      {(["watchlist", "favourites"] as const).map((view) => (
        <TabsContent key={view} value={view}>
          <BrowseBoundary>
            <SavedView view={view} profileId={profileId} sourceId={sourceId} />
          </BrowseBoundary>
        </TabsContent>
      ))}
      <TabsContent value="history">
        <BrowseBoundary>
          <LibraryHistory profileId={profileId} sourceId={sourceId} />
        </BrowseBoundary>
      </TabsContent>
    </Tabs>
  );
}

function SavedView({
  view,
  profileId,
  sourceId,
}: {
  view: "watchlist" | "favourites";
  profileId: string;
  sourceId: number;
}) {
  const [tab, setTab] = useState<BrowseTab>("movies");
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<"title" | "recentlyAdded">("title");
  const query = useDeferredValue(search.trim());
  const scrollRef = useScreenScroll();
  const activate = useActivate();
  const { switchTo } = useNavActions();
  const { toggleFavorite, toggleWatchlist } = useLibraryActions();
  const filters = useMemo<BrowseFilters>(
    () => ({
      query: query || null,
      sort,
      watchlistOnly: view === "watchlist",
      favoritesOnly: view === "favourites",
    }),
    [query, sort, view]
  );
  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-3">
        <SimpleSelect
          value={tab}
          onChange={setTab}
          ariaLabel="Saved media type"
          options={[
            { value: "movies", label: "Films" },
            { value: "series", label: "Shows" },
            { value: "live", label: "Channels" },
          ]}
        />
        <Input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search saved titles"
          placeholder="Search saved titles"
          className="max-w-sm"
        />
        {tab === "live" ? (
          <p className="text-xs text-muted-foreground">Grouped by channel family</p>
        ) : (
          <SimpleSelect
            value={sort}
            onChange={setSort}
            ariaLabel="Saved title order"
            options={[
              { value: "title", label: "A–Z" },
              { value: "recentlyAdded", label: "Recently added to playlist" },
            ]}
          />
        )}
      </div>
      <BrowseGrid
        tab={tab}
        profileId={profileId}
        sourceId={sourceId}
        filters={filters}
        scrollRef={scrollRef}
        layout={tab === "series" ? "landscape" : "poster"}
        onActivate={activate}
        onToggleFavorite={toggleFavorite}
        onToggleWatchlist={toggleWatchlist}
        emptyState={
          <div className="flex flex-col items-center gap-4 rounded-xl border border-border py-16 text-center">
            <p className="text-lg font-medium">
              {query
                ? "No saved titles match"
                : view === "watchlist"
                  ? "Something to look forward to"
                  : "Keep your favourites close"}
            </p>
            <p className="max-w-sm text-sm text-muted-foreground">
              {query
                ? "Try another title or clear your search."
                : view === "watchlist"
                  ? "Add titles with the plus button. They’ll be waiting here in this playlist."
                  : "Use the heart on any title to save it here in this playlist."}
            </p>
            <Button variant="outline" onClick={() => (query ? setSearch("") : switchTo(tab))}>
              {query
                ? "Clear search"
                : `Browse ${tab === "movies" ? "films" : tab === "series" ? "shows" : "channels"}`}
            </Button>
          </div>
        }
      />
    </div>
  );
}
