-- Phase 6: VOD transcription.
--
-- Source-language transcripts (one per media item per language per model) plus
-- their caption-sized segments. Translation tables (transcript_translations,
-- translated_vtt_files) and the FTS index land in Phase 7/8.

CREATE TABLE IF NOT EXISTS transcripts (
    id                     INTEGER PRIMARY KEY,
    media_kind             TEXT    NOT NULL,    -- 'vod' | 'episode' | 'live_recording'
    media_id               INTEGER NOT NULL,
    language               TEXT    NOT NULL,    -- BCP-47 source language
    model                  TEXT    NOT NULL,    -- e.g. 'large-v3-turbo-q5_0'
    vtt_path               TEXT    NOT NULL,
    json_path              TEXT    NOT NULL,
    created_at             INTEGER NOT NULL,
    duration_processed_sec REAL,
    UNIQUE(media_kind, media_id, language, model)
);
CREATE INDEX IF NOT EXISTS idx_transcripts_media
    ON transcripts(media_kind, media_id);

CREATE TABLE IF NOT EXISTS transcript_segments (
    id            INTEGER PRIMARY KEY,
    transcript_id INTEGER NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL,       -- ordinal within transcript, from 0
    start_sec     REAL    NOT NULL,
    end_sec       REAL    NOT NULL,
    text          TEXT    NOT NULL,
    words_json    TEXT                    -- optional [{startSec,endSec,word}]
);
CREATE INDEX IF NOT EXISTS idx_segments_transcript_seq
    ON transcript_segments(transcript_id, seq);

-- Job lifecycle columns the worker needs for claim/retry/complete transitions.
-- The base `jobs` table (migration 0001) only had id/kind/payload/state/
-- priority/created_at.
ALTER TABLE jobs ADD COLUMN attempts    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN last_error  TEXT;
ALTER TABLE jobs ADD COLUMN started_at  INTEGER;
ALTER TABLE jobs ADD COLUMN finished_at INTEGER;
