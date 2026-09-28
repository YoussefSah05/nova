//! Favourite / watchlist writes for any browse item. Canonical movies and
//! series are addressed by their entity; live and legacy VOD rows by row id.

import { invoke } from "@tauri-apps/api/core";

import type { BrowseItem } from "./queries";

/// A series is addressed by its canonical entity. One without an entity
/// hasn't been through the catalog compiler yet (only possible mid-import).
function unboundSeriesGuard(item: BrowseItem): string {
  const seriesId = item.series?.entityId;
  if (!seriesId) {
    throw new Error("This series is still being prepared. Try again once the refresh finishes.");
  }
  return seriesId;
}

export async function invokeFavorite(
  item: BrowseItem,
  profileId: string,
  next: boolean
): Promise<void> {
  if (item.kind === "live")
    return invoke("set_channel_favorite", { profileId, channelId: item.id, isFavorite: next });
  if (item.kind === "movie")
    return invoke("set_movie_favorite", { profileId, entityId: item.entityId, isFavorite: next });
  if (item.kind === "vod")
    return invoke("set_vod_favorite", { profileId, vodId: item.id, isFavorite: next });
  const seriesId = unboundSeriesGuard(item);
  return invoke("set_series_favorite", { profileId, seriesId, isFavorite: next });
}

export async function invokeWatchlist(
  item: BrowseItem,
  profileId: string,
  next: boolean
): Promise<void> {
  if (item.kind === "live")
    return invoke("set_channel_in_watchlist", { profileId, channelId: item.id, inWatchlist: next });
  if (item.kind === "movie")
    return invoke("set_movie_in_watchlist", {
      profileId,
      entityId: item.entityId,
      inWatchlist: next,
    });
  if (item.kind === "vod")
    return invoke("set_vod_in_watchlist", { profileId, vodId: item.id, inWatchlist: next });
  const seriesId = unboundSeriesGuard(item);
  return invoke("set_series_in_watchlist", { profileId, seriesId, inWatchlist: next });
}
