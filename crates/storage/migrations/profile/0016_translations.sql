-- Phase 7: VOD translation.
--
-- Translations cached per (source segment, target language, backend). Keyed off
-- transcript_segments(id) so they cascade-delete with their parent transcript
-- when the transcription cache is cleared or a transcript is regenerated.
-- (translated_vtt_files + the FTS index over translations land in Phase 8.)

CREATE TABLE IF NOT EXISTS transcript_translations (
    id          INTEGER PRIMARY KEY,
    segment_id  INTEGER NOT NULL REFERENCES transcript_segments(id) ON DELETE CASCADE,
    target_lang TEXT    NOT NULL,   -- BCP-47 reading language
    backend     TEXT    NOT NULL,   -- e.g. 'nllb-200-distilled-600M'
    text        TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    UNIQUE(segment_id, target_lang, backend)
);
CREATE INDEX IF NOT EXISTS idx_translations_segment
    ON transcript_translations(segment_id, target_lang);
