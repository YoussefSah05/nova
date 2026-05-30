-- Phase 6.1: streaming windowed transcription.
--
-- Transcripts are now built incrementally from ~30s windows transcribed at/ahead
-- of the playhead. `transcript_windows` records which time ranges have been
-- transcribed (coverage) so a re-watch loads instantly and only gaps ahead of
-- the playhead get transcribed. `complete` flips true once the whole media
-- duration is covered.

CREATE TABLE IF NOT EXISTS transcript_windows (
    id            INTEGER PRIMARY KEY,
    transcript_id INTEGER NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
    start_sec     REAL    NOT NULL,
    end_sec       REAL    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_transcript_windows
    ON transcript_windows(transcript_id, start_sec);

ALTER TABLE transcripts ADD COLUMN complete INTEGER NOT NULL DEFAULT 0;
