-- Phase 6.4: per-audio-track transcripts + stable content key.
--
-- Multi-track content (e.g. an MKV with several audio languages) needs one
-- transcript per audio track, and the cache should survive a playlist refresh
-- (which renumbers media_id). So we add:
--   * audio_track  — 0-based audio-stream index the transcript was made from
--   * content_key  — stable hash of the upstream stream URL (set by the app)
--
-- The old `transcripts` table has a table-level UNIQUE(media_kind, media_id,
-- language, model) that SQLite can't drop in place (and which would reject a
-- second same-language track), so we rebuild it. Foreign keys are enabled and
-- migrations run inside a transaction (so `PRAGMA foreign_keys` is a no-op), and
-- dropping a referenced parent triggers ON DELETE CASCADE — so we:
--   1. copy the child rows into FK-less temp tables,
--   2. drop the children, then the now-childless old parent (no cascade fires),
--   3. rename the rebuilt parent into place,
--   4. recreate the children referencing `transcripts` (the final name) and
--      reload them from the temp tables.

CREATE TABLE transcripts_new (
    id                     INTEGER PRIMARY KEY,
    media_kind             TEXT    NOT NULL,
    media_id               INTEGER NOT NULL,
    audio_track            INTEGER NOT NULL DEFAULT 0,
    content_key            TEXT    NOT NULL DEFAULT '',
    language               TEXT    NOT NULL,
    model                  TEXT    NOT NULL,
    vtt_path               TEXT    NOT NULL,
    json_path              TEXT    NOT NULL,
    created_at             INTEGER NOT NULL,
    duration_processed_sec REAL,
    complete               INTEGER NOT NULL DEFAULT 0,
    UNIQUE(media_kind, media_id, audio_track, language, model)
);
INSERT INTO transcripts_new
    (id, media_kind, media_id, audio_track, content_key, language, model,
     vtt_path, json_path, created_at, duration_processed_sec, complete)
  SELECT id, media_kind, media_id, 0, '', language, model,
         vtt_path, json_path, created_at, duration_processed_sec, complete
  FROM transcripts;

-- Park child rows in FK-less temp tables so the parent can be dropped safely.
CREATE TABLE transcript_segments_tmp AS SELECT * FROM transcript_segments;
CREATE TABLE transcript_windows_tmp  AS SELECT * FROM transcript_windows;

DROP TABLE transcript_windows;
DROP TABLE transcript_segments;
DROP TABLE transcripts;

ALTER TABLE transcripts_new RENAME TO transcripts;

CREATE TABLE transcript_segments (
    id            INTEGER PRIMARY KEY,
    transcript_id INTEGER NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL,
    start_sec     REAL    NOT NULL,
    end_sec       REAL    NOT NULL,
    text          TEXT    NOT NULL,
    words_json    TEXT
);
CREATE TABLE transcript_windows (
    id            INTEGER PRIMARY KEY,
    transcript_id INTEGER NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
    start_sec     REAL    NOT NULL,
    end_sec       REAL    NOT NULL
);

INSERT INTO transcript_segments (id, transcript_id, seq, start_sec, end_sec, text, words_json)
  SELECT id, transcript_id, seq, start_sec, end_sec, text, words_json FROM transcript_segments_tmp;
INSERT INTO transcript_windows (id, transcript_id, start_sec, end_sec)
  SELECT id, transcript_id, start_sec, end_sec FROM transcript_windows_tmp;

DROP TABLE transcript_segments_tmp;
DROP TABLE transcript_windows_tmp;

CREATE INDEX IF NOT EXISTS idx_transcripts_media
    ON transcripts(media_kind, media_id);
-- Stable per-track lookup; unique only once a real content key is set.
CREATE UNIQUE INDEX IF NOT EXISTS idx_transcripts_content
    ON transcripts(content_key, audio_track, model) WHERE content_key <> '';
CREATE INDEX IF NOT EXISTS idx_segments_transcript_seq
    ON transcript_segments(transcript_id, seq);
CREATE INDEX IF NOT EXISTS idx_transcript_windows
    ON transcript_windows(transcript_id, start_sec);
