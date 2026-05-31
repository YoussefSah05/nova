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
    pub audio_track: i64,
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

    /// Find or create the transcript row for a specific audio track, returning
    /// its id. Identity is the stable `content_key` (a hash of the stream URL)
    /// plus `audio_track` and `model` — so a playlist refresh that renumbers
    /// `media_id` still resolves the same cached transcript, and each audio
    /// track is cached independently. Falls back to `(media_id, audio_track)`
    /// for legacy rows written before `content_key` existed, backfilling it.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_or_create_transcript(
        &self,
        content_key: &str,
        media_kind: &str,
        media_id: i64,
        audio_track: i64,
        language: &str,
        model: &str,
        vtt_path: &str,
        json_path: &str,
    ) -> Result<i64, StorageError> {
        // 1. Stable lookup by content key + track. Re-point media_id to the live
        //    row when it changed (the title was re-imported under a new id).
        if !content_key.is_empty() {
            if let Some(row) = sqlx::query!(
                r#"SELECT id as "id!: i64", media_id as "media_id!: i64" FROM transcripts
                   WHERE content_key = ? AND audio_track = ? AND model = ?"#,
                content_key,
                audio_track,
                model,
            )
            .fetch_optional(self.pool)
            .await?
            {
                if row.media_id != media_id {
                    sqlx::query!(
                        "UPDATE transcripts SET media_id = ? WHERE id = ?",
                        media_id,
                        row.id
                    )
                    .execute(self.pool)
                    .await?;
                }
                return Ok(row.id);
            }
        }

        // 2. Legacy / just-created row keyed by media + track; backfill the key.
        if let Some(row) = sqlx::query!(
            r#"SELECT id as "id!: i64" FROM transcripts
               WHERE media_kind = ? AND media_id = ? AND audio_track = ? AND model = ?"#,
            media_kind,
            media_id,
            audio_track,
            model,
        )
        .fetch_optional(self.pool)
        .await?
        {
            if !content_key.is_empty() {
                sqlx::query!(
                    "UPDATE transcripts SET content_key = ? WHERE id = ?",
                    content_key,
                    row.id
                )
                .execute(self.pool)
                .await?;
            }
            return Ok(row.id);
        }

        // 3. Create it.
        let now = now_ts();
        let row = sqlx::query!(
            r#"INSERT INTO transcripts
                 (content_key, media_kind, media_id, audio_track, language, model,
                  vtt_path, json_path, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
               RETURNING id as "id!: i64""#,
            content_key,
            media_kind,
            media_id,
            audio_track,
            language,
            model,
            vtt_path,
            json_path,
            now,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(row.id)
    }

    /// Append a window's segments to a transcript (one transaction).
    pub async fn append_segments(
        &self,
        transcript_id: i64,
        segments: &[TranscriptSegment],
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
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
        Ok(())
    }

    /// Record a transcribed window `[start_sec, end_sec)` for coverage tracking.
    pub async fn record_window(
        &self,
        transcript_id: i64,
        start_sec: f64,
        end_sec: f64,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "INSERT INTO transcript_windows (transcript_id, start_sec, end_sec)
             VALUES (?, ?, ?)",
            transcript_id,
            start_sec,
            end_sec,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// All covered windows for a transcript, ordered by start.
    pub async fn covered_windows(
        &self,
        transcript_id: i64,
    ) -> Result<Vec<(f64, f64)>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT start_sec as "start_sec!: f64", end_sec as "end_sec!: f64"
               FROM transcript_windows WHERE transcript_id = ? ORDER BY start_sec ASC"#,
            transcript_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| (r.start_sec, r.end_sec)).collect())
    }

    /// Flag a transcript as fully covering its media duration.
    pub async fn mark_complete(&self, transcript_id: i64) -> Result<(), StorageError> {
        sqlx::query!(
            "UPDATE transcripts SET complete = 1 WHERE id = ?",
            transcript_id
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// The cached transcript for a specific audio track, if any. Resolves by the
    /// stable `content_key` first (survives a media re-import) and falls back to
    /// `(media_kind, media_id)` for rows written before content keys existed.
    /// Used by the caption session to replay cached cues on (re)start.
    pub async fn get_for_media_track(
        &self,
        content_key: &str,
        media_kind: &str,
        media_id: i64,
        audio_track: i64,
    ) -> Result<Option<StoredTranscript>, StorageError> {
        if !content_key.is_empty() {
            let row = sqlx::query!(
                r#"SELECT
                     id                     as "id!: i64",
                     media_kind,
                     media_id               as "media_id!: i64",
                     audio_track            as "audio_track!: i64",
                     language,
                     model,
                     vtt_path,
                     json_path,
                     created_at             as "created_at!: i64",
                     duration_processed_sec as "duration_processed_sec?: f64"
                   FROM transcripts
                   WHERE content_key = ? AND audio_track = ?
                   ORDER BY created_at DESC
                   LIMIT 1"#,
                content_key,
                audio_track,
            )
            .fetch_optional(self.pool)
            .await?;
            if let Some(r) = row {
                return Ok(Some(StoredTranscript {
                    id: r.id,
                    media_kind: r.media_kind,
                    media_id: r.media_id,
                    audio_track: r.audio_track,
                    language: r.language,
                    model: r.model,
                    vtt_path: r.vtt_path,
                    json_path: r.json_path,
                    created_at: r.created_at,
                    duration_processed_sec: r.duration_processed_sec,
                }));
            }
        }

        let row = sqlx::query!(
            r#"SELECT
                 id                     as "id!: i64",
                 media_kind,
                 media_id               as "media_id!: i64",
                 audio_track            as "audio_track!: i64",
                 language,
                 model,
                 vtt_path,
                 json_path,
                 created_at             as "created_at!: i64",
                 duration_processed_sec as "duration_processed_sec?: f64"
               FROM transcripts
               WHERE media_kind = ? AND media_id = ? AND audio_track = ?
               ORDER BY created_at DESC
               LIMIT 1"#,
            media_kind,
            media_id,
            audio_track,
        )
        .fetch_optional(self.pool)
        .await?;

        Ok(row.map(|r| StoredTranscript {
            id: r.id,
            media_kind: r.media_kind,
            media_id: r.media_id,
            audio_track: r.audio_track,
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
               ORDER BY start_sec ASC"#,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_profile_db;

    async fn temp_pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.db");
        let pool = init_profile_db(path.to_str().unwrap()).await.unwrap();
        (dir, pool)
    }

    #[tokio::test]
    async fn content_key_identity_per_track_and_refresh_stable() {
        let (_dir, pool) = temp_pool().await;
        let repo = TranscriptRepository::new(&pool);

        // Track 0 and track 1 of the same content get distinct transcripts.
        let t0 = repo
            .get_or_create_transcript("keyA", "vod", 1, 0, "en", "m", "/a0.vtt", "/a0.json")
            .await
            .unwrap();
        let t1 = repo
            .get_or_create_transcript("keyA", "vod", 1, 1, "fr", "m", "/a1.vtt", "/a1.json")
            .await
            .unwrap();
        assert_ne!(t0, t1, "different tracks → different transcripts");

        // Same content key + track resolves to the same row even when media_id
        // changed (post-refresh re-import), and re-points media_id to the live row.
        let again = repo
            .get_or_create_transcript("keyA", "vod", 999, 0, "en", "m", "/a0.vtt", "/a0.json")
            .await
            .unwrap();
        assert_eq!(again, t0, "stable content key survives media_id change");
        let found = repo
            .get_for_media_track("keyA", "vod", 999, 0)
            .await
            .unwrap()
            .expect("found by content key");
        assert_eq!(found.id, t0);
        assert_eq!(found.media_id, 999, "media_id re-pointed to the live row");
    }

    #[tokio::test]
    async fn legacy_row_backfills_content_key() {
        let (_dir, pool) = temp_pool().await;
        let repo = TranscriptRepository::new(&pool);

        // A row created before content keys existed (empty key).
        let legacy = repo
            .get_or_create_transcript("", "vod", 7, 0, "en", "m", "/l.vtt", "/l.json")
            .await
            .unwrap();
        // Re-opening with a real key backfills it onto the same row (no dup).
        let same = repo
            .get_or_create_transcript("keyB", "vod", 7, 0, "en", "m", "/l.vtt", "/l.json")
            .await
            .unwrap();
        assert_eq!(legacy, same);
        let by_key = repo.get_for_media_track("keyB", "vod", 7, 0).await.unwrap();
        assert!(by_key.is_some(), "now resolvable by the backfilled key");
    }
}
