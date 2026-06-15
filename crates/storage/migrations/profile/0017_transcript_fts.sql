-- Phase 8: full-text search over transcripts + translations.
--
-- A single FTS5 virtual table indexes both the source segments and every cached
-- translation, so a line can be found by what was *said* or by its *translated*
-- text. `lang`/`segment_id`/`kind` are UNINDEXED — carried for filtering and for
-- deep-linking back to a segment's timing (join on `segment_id`). Standalone
-- (not external-content) so normal DELETE works; rowids are auto-assigned and we
-- delete by (segment_id, kind[, lang]) — never by rowid — to avoid any clash
-- between a source segment id and an auto rowid.

CREATE VIRTUAL TABLE IF NOT EXISTS transcript_fts USING fts5(
    text,
    lang       UNINDEXED,   -- source language or target language (BCP-47)
    segment_id UNINDEXED,   -- transcript_segments.id (join for start/end + transcript)
    kind       UNINDEXED,   -- 'source' | 'translation'
    tokenize='unicode61 remove_diacritics 2'
);

-- Keep the index in sync with source segments.
CREATE TRIGGER IF NOT EXISTS transcript_fts_seg_ai
AFTER INSERT ON transcript_segments
BEGIN
  INSERT INTO transcript_fts(text, lang, segment_id, kind)
  SELECT NEW.text, tr.language, NEW.id, 'source'
  FROM transcripts tr
  WHERE tr.id = NEW.transcript_id;
END;

CREATE TRIGGER IF NOT EXISTS transcript_fts_seg_ad
AFTER DELETE ON transcript_segments
BEGIN
  DELETE FROM transcript_fts WHERE segment_id = OLD.id AND kind = 'source';
END;

-- Keep the index in sync with translations.
CREATE TRIGGER IF NOT EXISTS transcript_fts_tr_ai
AFTER INSERT ON transcript_translations
BEGIN
  INSERT INTO transcript_fts(text, lang, segment_id, kind)
  VALUES (NEW.text, NEW.target_lang, NEW.segment_id, 'translation');
END;

CREATE TRIGGER IF NOT EXISTS transcript_fts_tr_ad
AFTER DELETE ON transcript_translations
BEGIN
  DELETE FROM transcript_fts
  WHERE segment_id = OLD.segment_id AND kind = 'translation' AND lang = OLD.target_lang;
END;

-- Backfill already-watched content (triggers only fire on new rows).
INSERT INTO transcript_fts(text, lang, segment_id, kind)
SELECT ts.text, tr.language, ts.id, 'source'
FROM transcript_segments ts
JOIN transcripts tr ON tr.id = ts.transcript_id;

INSERT INTO transcript_fts(text, lang, segment_id, kind)
SELECT tt.text, tt.target_lang, tt.segment_id, 'translation'
FROM transcript_translations tt;
