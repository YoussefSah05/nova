-- Semantic search: persist the on-device embedding vectors that enrichment
-- already computes (and currently throws away), so meaning-based retrieval is a
-- near-free byproduct. One table covers every embeddable thing:
--   content_kind = 'movie' | 'live'   → content_id = vod_items.id / channels.id
--   content_kind = 'segment'          → content_id = transcript_segments.id
-- `vector` is the model's float32 output, little-endian, length × 4 bytes. The
-- model id is part of the key so a model change never mixes incompatible spaces.
-- Brute-force cosine in Rust ranks these (no ANN extension at Nova's scale).

CREATE TABLE IF NOT EXISTS catalog_embeddings (
    content_kind TEXT    NOT NULL,
    content_id   INTEGER NOT NULL,
    model        TEXT    NOT NULL,
    vector       BLOB    NOT NULL,
    PRIMARY KEY (content_kind, content_id, model)
);

-- Catalog rows are re-embedded on a refresh/re-enrichment (the writer upserts),
-- but transcript segments are deleted when the cache is cleared — drop their
-- vectors with them so a cleared transcript also disappears from dialogue search.
CREATE TRIGGER IF NOT EXISTS catalog_embeddings_seg_ad
AFTER DELETE ON transcript_segments
BEGIN
  DELETE FROM catalog_embeddings
  WHERE content_kind = 'segment' AND content_id = OLD.id;
END;
