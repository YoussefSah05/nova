-- Numeric media IDs can be reused after refresh/source deletion. Derived
-- vectors must not survive their catalog rows. Existing vectors cannot be
-- distinguished from ones left behind by older refreshes: rebuild this cache.
DELETE FROM catalog_embeddings WHERE content_kind IN ('movie', 'live');
UPDATE channels SET enrich_status = NULL;
UPDATE vod_items SET enrich_status = NULL;

-- Remove identifiable orphan history, preserving history for existing media.
DELETE FROM watch_history
WHERE (media_kind = 'vod' AND NOT EXISTS (SELECT 1 FROM vod_items WHERE id = media_id))
   OR (media_kind = 'live' AND NOT EXISTS (SELECT 1 FROM channels WHERE id = media_id))
   OR (media_kind = 'episode' AND NOT EXISTS (SELECT 1 FROM series_episodes WHERE id = media_id));

-- A transcript's content hash survives refresh, but its numeric media reference
-- does not. Detach old references; stable-key lookup reattaches on next playback.
-- Each negative ID is unique, preserving the legacy media/track UNIQUE index.
-- Legacy rows with no content hash retain their existing lookup behavior.
UPDATE transcripts SET media_id = -id WHERE content_key <> '';

CREATE TRIGGER catalog_state_channel_ad AFTER DELETE ON channels
BEGIN
  DELETE FROM catalog_embeddings WHERE content_kind = 'live' AND content_id = OLD.id;
  DELETE FROM watch_history WHERE media_kind = 'live' AND media_id = OLD.id;
  UPDATE transcripts SET media_id = -id WHERE media_kind = 'live' AND media_id = OLD.id;
END;

CREATE TRIGGER catalog_state_vod_ad AFTER DELETE ON vod_items
BEGIN
  DELETE FROM catalog_embeddings WHERE content_kind = 'movie' AND content_id = OLD.id;
  DELETE FROM watch_history WHERE media_kind = 'vod' AND media_id = OLD.id;
  UPDATE transcripts SET media_id = -id WHERE media_kind = 'vod' AND media_id = OLD.id;
END;

CREATE TRIGGER catalog_state_episode_ad AFTER DELETE ON series_episodes
BEGIN
  DELETE FROM watch_history WHERE media_kind = 'episode' AND media_id = OLD.id;
  UPDATE transcripts SET media_id = -id WHERE media_kind = 'episode' AND media_id = OLD.id;
END;
