//! Translation cache (Phase 7).
//!
//! One row per `(source segment, target language, backend)`. Lookups are scoped
//! to a transcript (join through `transcript_segments`) so the streaming
//! translation session can fetch everything it's already translated for the open
//! title in one query and skip re-running the model.

use std::collections::HashMap;

use nova_core::now_ts;
use sqlx::SqlitePool;

use crate::error::StorageError;

pub struct TranslationRepository<'a> {
    pool: &'a SqlitePool,
}

/// A translation to cache for one source segment.
#[derive(Debug, Clone)]
pub struct TranslationRow {
    pub segment_id: i64,
    pub target_lang: String,
    pub backend: String,
    pub text: String,
}

impl<'a> TranslationRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Look up one segment with a complete versioned provenance key. Legacy
    /// backend names do not match keys emitted by the new caption pipeline.
    pub async fn cached_segment(
        &self,
        segment_id: i64,
        target_lang: &str,
        backend: &str,
    ) -> Result<Option<String>, StorageError> {
        Ok(sqlx::query!(
            "SELECT text FROM transcript_translations
             WHERE segment_id = ? AND target_lang = ? AND backend = ?",
            segment_id,
            target_lang,
            backend,
        )
        .fetch_optional(self.pool)
        .await?
        .map(|r| r.text))
    }

    /// Cached translations for a transcript's segments, keyed by `segment_id`.
    pub async fn cached(
        &self,
        transcript_id: i64,
        target_lang: &str,
        backend: &str,
    ) -> Result<HashMap<i64, String>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT tt.segment_id as "segment_id!: i64", tt.text
               FROM transcript_translations tt
               JOIN transcript_segments s ON s.id = tt.segment_id
               WHERE s.transcript_id = ? AND tt.target_lang = ? AND tt.backend = ?"#,
            transcript_id,
            target_lang,
            backend,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| (r.segment_id, r.text)).collect())
    }

    /// Insert a batch of translations (one transaction). Idempotent: a row that
    /// already exists for `(segment_id, target_lang, backend)` is left untouched.
    pub async fn insert_batch(&self, rows: &[TranslationRow]) -> Result<(), StorageError> {
        let now = now_ts();
        let mut tx = self.pool.begin().await?;
        for r in rows {
            sqlx::query!(
                "INSERT OR IGNORE INTO transcript_translations
                   (segment_id, target_lang, backend, text, created_at)
                 VALUES (?, ?, ?, ?, ?)",
                r.segment_id,
                r.target_lang,
                r.backend,
                r.text,
                now,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Total cached translations (for the Settings cache view).
    pub async fn count(&self) -> Result<i64, StorageError> {
        let row = sqlx::query!(r#"SELECT COUNT(*) as "n!: i64" FROM transcript_translations"#)
            .fetch_one(self.pool)
            .await?;
        Ok(row.n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_profile_db;
    use crate::repositories::TranscriptRepository;
    use nova_core::TranscriptSegment;

    async fn temp_pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.db");
        let pool = init_profile_db(path.to_str().unwrap()).await.unwrap();
        (dir, pool)
    }

    #[tokio::test]
    async fn cache_roundtrip_scoped_to_transcript() {
        let (_dir, pool) = temp_pool().await;
        let tr = TranscriptRepository::new(&pool);
        let id = tr
            .get_or_create_transcript("k", "vod", 1, 0, "ar", "m", "/a.vtt", "/a.json")
            .await
            .unwrap();
        tr.append_segments(
            id,
            &[TranscriptSegment {
                seq: 0,
                start_sec: 0.0,
                end_sec: 1.0,
                text: "مرحبا".into(),
                words: None,
            }],
        )
        .await
        .unwrap();
        // The just-inserted segment's id (single segment → id 1 in a fresh db).
        let seg_id: i64 = sqlx::query_scalar("SELECT id FROM transcript_segments LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();

        let repo = TranslationRepository::new(&pool);
        assert!(repo.cached(id, "en", "nllb").await.unwrap().is_empty());

        repo.insert_batch(&[TranslationRow {
            segment_id: seg_id,
            target_lang: "en".into(),
            backend: "nllb".into(),
            text: "Hello".into(),
        }])
        .await
        .unwrap();

        let cached = repo.cached(id, "en", "nllb").await.unwrap();
        assert_eq!(cached.get(&seg_id).map(String::as_str), Some("Hello"));
        // A different target/backend is a miss.
        assert!(repo.cached(id, "fr", "nllb").await.unwrap().is_empty());
        assert_eq!(repo.count().await.unwrap(), 1);
        assert_eq!(
            repo.cached_segment(seg_id, "en", "nllb").await.unwrap(),
            Some("Hello".into())
        );
        assert_eq!(
            repo.cached_segment(seg_id, "en", "new-versioned-key")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            repo.cached_segment(seg_id, "fr", "nllb").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn cascades_when_transcript_cleared() {
        let (_dir, pool) = temp_pool().await;
        let tr = TranscriptRepository::new(&pool);
        let id = tr
            .get_or_create_transcript("k", "vod", 1, 0, "ar", "m", "/a.vtt", "/a.json")
            .await
            .unwrap();
        tr.append_segments(
            id,
            &[TranscriptSegment {
                seq: 0,
                start_sec: 0.0,
                end_sec: 1.0,
                text: "x".into(),
                words: None,
            }],
        )
        .await
        .unwrap();
        let seg_id: i64 = sqlx::query_scalar("SELECT id FROM transcript_segments LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let repo = TranslationRepository::new(&pool);
        repo.insert_batch(&[TranslationRow {
            segment_id: seg_id,
            target_lang: "en".into(),
            backend: "nllb".into(),
            text: "X".into(),
        }])
        .await
        .unwrap();
        // Clearing transcripts cascades segments → translations.
        tr.clear_all().await.unwrap();
        assert_eq!(repo.count().await.unwrap(), 0);
    }
}
