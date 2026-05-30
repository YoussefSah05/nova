//! Transcript persistence (Phase 6).
//!
//! A `transcripts` row points at the on-disk `.vtt`/`.json` artifacts and owns
//! its caption-sized `transcript_segments`. Word timings (when present) are
//! stored as JSON in `transcript_segments.words_json`.

use nova_core::{now_ts, TranscriptSegment, TranscriptWord};
use sqlx::SqlitePool;

use crate::error::StorageError;

pub struct TranscriptRepository<'a> {
    pool: &'a SqlitePool,
}

/// A persisted transcript header (without segments).
#[derive(Debug, Clone)]
pub struct StoredTranscript {
    pub id: i64,
    pub media_kind: String,
    pub media_id: i64,
    pub language: String,
    pub model: String,
    pub vtt_path: String,
    pub json_path: String,
    pub created_at: i64,
    pub duration_processed_sec: Option<f64>,
}

impl<'a> TranscriptRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Replace any existing transcript for `(media_kind, media_id, language,
    /// model)` and its segments, returning the new transcript id. Runs in one
    /// transaction so a partial write never leaves a half-populated transcript.
    #[allow(clippy::too_many_arguments)]
    pub async fn replace_transcript(
        &self,
        media_kind: &str,
        media_id: i64,
        language: &str,
        model: &str,
        vtt_path: &str,
        json_path: &str,
        duration_processed_sec: Option<f64>,
        segments: &[TranscriptSegment],
    ) -> Result<i64, StorageError> {
        let now = now_ts();
        let mut tx = self.pool.begin().await?;

        sqlx::query!(
            "DELETE FROM transcripts
             WHERE media_kind = ? AND media_id = ? AND language = ? AND model = ?",
            media_kind,
            media_id,
            language,
            model,
        )
        .execute(&mut *tx)
        .await?;

        let row = sqlx::query!(
            r#"INSERT INTO transcripts
                 (media_kind, media_id, language, model, vtt_path, json_path,
                  created_at, duration_processed_sec)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)
               RETURNING id as "id!: i64""#,
            media_kind,
            media_id,
            language,
            model,
            vtt_path,
            json_path,
            now,
            duration_processed_sec,
        )
        .fetch_one(&mut *tx)
        .await?;
        let transcript_id = row.id;

        for seg in segments {
            let words_json = match &seg.words {
                Some(words) => Some(serde_json::to_string(words)?),
                None => None,
            };
            sqlx::query!(
                r#"INSERT INTO transcript_segments
                     (transcript_id, seq, start_sec, end_sec, text, words_json)
                   VALUES (?, ?, ?, ?, ?, ?)"#,
                transcript_id,
                seg.seq,
                seg.start_sec,
                seg.end_sec,
                seg.text,
                words_json,
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(transcript_id)
    }

    /// The most recent transcript for a media item, in any language. Used by
    /// the player to decide whether captions are ready.
    pub async fn get_for_media(
        &self,
        media_kind: &str,
        media_id: i64,
    ) -> Result<Option<StoredTranscript>, StorageError> {
        let row = sqlx::query!(
            r#"SELECT
                 id                     as "id!: i64",
                 media_kind,
                 media_id               as "media_id!: i64",
                 language,
                 model,
                 vtt_path,
                 json_path,
                 created_at             as "created_at!: i64",
                 duration_processed_sec as "duration_processed_sec?: f64"
               FROM transcripts
               WHERE media_kind = ? AND media_id = ?
               ORDER BY created_at DESC
               LIMIT 1"#,
            media_kind,
            media_id,
        )
        .fetch_optional(self.pool)
        .await?;

        Ok(row.map(|r| StoredTranscript {
            id: r.id,
            media_kind: r.media_kind,
            media_id: r.media_id,
            language: r.language,
            model: r.model,
            vtt_path: r.vtt_path,
            json_path: r.json_path,
            created_at: r.created_at,
            duration_processed_sec: r.duration_processed_sec,
        }))
    }

    /// All segments of a transcript, ordered. Used by Phase 7 translation and
    /// Phase 8 search; kept here so the segment ↔ `words_json` mapping lives in
    /// one place.
    pub async fn get_segments(
        &self,
        transcript_id: i64,
    ) -> Result<Vec<TranscriptSegment>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT
                 seq        as "seq!: i64",
                 start_sec  as "start_sec!: f64",
                 end_sec    as "end_sec!: f64",
                 text,
                 words_json as "words_json?: String"
               FROM transcript_segments
               WHERE transcript_id = ?
               ORDER BY seq ASC"#,
            transcript_id,
        )
        .fetch_all(self.pool)
        .await?;

        let mut segments = Vec::with_capacity(rows.len());
        for r in rows {
            let words: Option<Vec<TranscriptWord>> = match r.words_json {
                Some(json) => Some(serde_json::from_str(&json)?),
                None => None,
            };
            segments.push(TranscriptSegment {
                seq: r.seq,
                start_sec: r.start_sec,
                end_sec: r.end_sec,
                text: r.text,
                words,
            });
        }
        Ok(segments)
    }
}
