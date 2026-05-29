import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SeriesEpisode, SeriesSummary } from "@/types";
import { FavoriteButton } from "@/features/browse/FavoriteButton";
import { WatchlistButton } from "@/features/browse/WatchlistButton";

interface SeriesDetailProps {
  profileId: string;
  series: SeriesSummary;
  onBack: () => void;
  onToggleFavorite: (next: boolean) => Promise<void> | void;
  onToggleWatchlist: (next: boolean) => Promise<void> | void;
  onPlayEpisode: (episode: SeriesEpisode) => void;
}

export function SeriesDetail({
  profileId,
  series,
  onBack,
  onToggleFavorite,
  onToggleWatchlist,
  onPlayEpisode,
}: SeriesDetailProps) {
  const [episodes, setEpisodes] = useState<SeriesEpisode[]>([]);
  const [openSeasons, setOpenSeasons] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Bumped by the retry button to re-run the fetch effect.
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    // The first open of an Xtream series fetches its episodes from the
    // provider (get_series_info), so this can take a moment.
    invoke<SeriesEpisode[]>("list_series_episodes", {
      profileId,
      sourceId: series.sourceId,
      seriesTitle: series.seriesTitle,
    })
      .then((eps) => {
        if (cancelled) return;
        setEpisodes(eps);
        // Open the lowest season by default so episodes are visible immediately.
        const first = eps.reduce<number | null>(
          (min, e) => (min === null || e.season < min ? e.season : min),
          null
        );
        setOpenSeasons(first === null ? new Set() : new Set([first]));
      })
      .catch((e) => {
        if (!cancelled) setError(typeof e === "string" ? e : "Couldn't load episodes.");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [profileId, series, reloadKey]);

  const bySeasonMap = new Map<number, SeriesEpisode[]>();
  for (const ep of episodes) {
    const list = bySeasonMap.get(ep.season) ?? [];
    list.push(ep);
    bySeasonMap.set(ep.season, list);
  }
  const seasons = Array.from(bySeasonMap.keys()).sort((a, b) => a - b);
  // Derive counts from the fetched episodes — the catalog summary reports 0 for
  // not-yet-fetched Xtream series.
  const seasonCount = seasons.length || series.seasonCount;
  const episodeCount = episodes.length || series.episodeCount;

  return (
    <div className="flex flex-col gap-4">
      <button
        onClick={onBack}
        className="flex items-center gap-2 text-sm text-white/50 hover:text-white transition-colors w-fit"
      >
        ← Back
      </button>

      <div className="flex items-start gap-4">
        {series.posterUrl && (
          <img
            src={series.posterUrl}
            alt={series.seriesTitle}
            className="w-20 h-28 object-cover rounded-lg flex-shrink-0"
          />
        )}
        <div className="flex flex-1 items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="text-xl font-semibold text-white">{series.seriesTitle}</h2>
            <p className="text-sm text-white/50 mt-1">
              {seasonCount} season{seasonCount !== 1 ? "s" : ""} ·{" "}
              {episodeCount} episode{episodeCount !== 1 ? "s" : ""}
              {series.category ? ` · ${series.category}` : ""}
            </p>
          </div>
          <div className="flex flex-shrink-0 items-center gap-2">
            <WatchlistButton
              inWatchlist={series.isInWatchlist}
              onToggle={onToggleWatchlist}
              label={series.seriesTitle}
            />
            <FavoriteButton
              isFavorite={series.isFavorite}
              onToggle={onToggleFavorite}
              label={series.seriesTitle}
            />
          </div>
        </div>
      </div>

      {loading && (
        <div className="flex items-center gap-3 px-1 py-8 text-sm text-white/50">
          <span className="h-4 w-4 animate-spin rounded-full border-2 border-white/20 border-t-white/70" />
          Loading episodes…
        </div>
      )}

      {!loading && error && (
        <div className="flex flex-col items-start gap-3 rounded-xl bg-neutral-900 px-4 py-5">
          <p className="text-sm text-white/70">{error}</p>
          <button
            onClick={() => setReloadKey((k) => k + 1)}
            className="rounded-lg bg-white/10 px-4 py-1.5 text-xs font-medium text-white transition-colors hover:bg-white/20"
          >
            Retry
          </button>
        </div>
      )}

      {!loading && !error && episodes.length === 0 && (
        <p className="px-1 py-8 text-sm text-white/50">No episodes available for this series.</p>
      )}

      <div className="flex flex-col gap-2">
        {seasons.map((season) => {
          const eps = bySeasonMap.get(season) ?? [];
          const isOpen = openSeasons.has(season);
          return (
            <div key={season} className="rounded-xl bg-neutral-900 overflow-hidden">
              <button
                className="w-full flex items-center justify-between px-4 py-3 text-sm font-medium text-white hover:bg-neutral-800 transition-colors"
                onClick={() =>
                  setOpenSeasons((prev) => {
                    const next = new Set(prev);
                    if (next.has(season)) next.delete(season);
                    else next.add(season);
                    return next;
                  })
                }
              >
                <span>Season {season}</span>
                <span className="text-white/40">{isOpen ? "▲" : "▼"}</span>
              </button>
              {isOpen && (
                <div className="divide-y divide-white/5">
                  {eps.map((ep) => (
                    <button
                      key={ep.id}
                      onClick={() => onPlayEpisode(ep)}
                      className="flex w-full items-center gap-3 px-4 py-2.5 text-left transition-colors hover:bg-white/[0.04] focus:outline-none focus:bg-white/[0.04]"
                      aria-label={`Play ${series.seriesTitle} S${season} E${ep.episode}`}
                    >
                      {ep.logoUrl ? (
                        <img
                          src={ep.logoUrl}
                          alt=""
                          className="w-16 h-9 object-cover rounded flex-shrink-0 bg-neutral-800"
                        />
                      ) : (
                        <div className="w-16 h-9 rounded bg-neutral-800 flex-shrink-0" />
                      )}
                      <span className="text-sm text-white/80">
                        E{String(ep.episode).padStart(2, "0")}
                        {ep.episodeTitle ? ` — ${ep.episodeTitle}` : ""}
                      </span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
