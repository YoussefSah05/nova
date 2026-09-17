//! Dialogue search (Phase 8) over the `transcript_fts` index.
//!
//! Both source and translation rows in `transcript_fts` carry the source
//! segment's id, so a single join to `transcript_segments` recovers the timing
//! (for deep-linking) and the parent transcript (for scoping a search to the
//! title being watched). Ranked by FTS5 `bm25` (most relevant first).

use sqlx::SqlitePool;

use crate::error::StorageError;

pub struct SearchRepository<'a> {
    pool: &'a SqlitePool,
}

/// One transcript-search match. `kind` distinguishes a spoken (`source`) line
/// from a translated one; `start_sec` is where to seek on click.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub segment_id: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
    pub kind: String,
    pub score: f64,
}

impl<'a> SearchRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Full-text search within one transcript. `match_expr` must come from
    /// `nova_search::fts_match` (already escaped); an empty expr should be
    /// guarded by the caller (FTS5 rejects an empty MATCH).
    pub async fn search_in_transcript(
        &self,
        transcript_id: i64,
        match_expr: &str,
        limit: i64,
    ) -> Result<Vec<SearchHit>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT
                 CAST(f.segment_id AS INTEGER) as "segment_id!: i64",
                 ts.start_sec as "start_sec!: f64",
                 ts.end_sec   as "end_sec!: f64",
                 f.text       as "text!: String",
                 f.kind       as "kind!: String",
                 bm25(transcript_fts) as "score!: f64"
               FROM transcript_fts f
               JOIN transcript_segments ts ON ts.id = CAST(f.segment_id AS INTEGER)
               WHERE ts.transcript_id = ? AND f.text MATCH ?
               ORDER BY bm25(transcript_fts)
               LIMIT ?"#,
            transcript_id,
            match_expr,
            limit,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| SearchHit {
                segment_id: r.segment_id,
                start_sec: r.start_sec,
                end_sec: r.end_sec,
                text: r.text,
                kind: r.kind,
                score: r.score,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_profile_db;
    use crate::repositories::{TranscriptRepository, TranslationRepository, TranslationRow};
    use nova_core::TranscriptSegment;

    async fn temp_pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.db");
        let pool = init_profile_db(path.to_str().unwrap()).await.unwrap();
        (dir, pool)
    }

    #[tokio::test]
    async fn finds_source_and_translation_with_timing() {
        let (_dir, pool) = temp_pool().await;
        let tr = TranscriptRepository::new(&pool);
        let id = tr
            .get_or_create_transcript("k", "vod", 1, 0, "ar", "m", "/a.vtt", "/a.json")
            .await
            .unwrap();
        tr.append_segments(
            id,
            &[
                TranscriptSegment {
                    seq: 0,
                    start_sec: 1.0,
                    end_sec: 3.0,
                    text: "مرحبا بالعالم".into(),
                    words: None,
                },
                TranscriptSegment {
                    seq: 1,
                    start_sec: 3.0,
                    end_sec: 5.0,
                    text: "كيف حالك".into(),
                    words: None,
                },
            ],
        )
        .await
        .unwrap();
        let seg0: i64 =
            sqlx::query_scalar("SELECT id FROM transcript_segments ORDER BY id LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        TranslationRepository::new(&pool)
            .insert_batch(&[TranslationRow {
                segment_id: seg0,
                target_lang: "en".into(),
                backend: "nllb".into(),
                text: "Hello world".into(),
            }])
            .await
            .unwrap();

        let repo = SearchRepository::new(&pool);
        // Found by translated (English) text — kind 'translation', with timing.
        let hits = repo
            .search_in_transcript(id, &nova_search::fts_match("hello"), 50)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, "translation");
        assert_eq!(hits[0].segment_id, seg0);
        assert_eq!(hits[0].start_sec, 1.0);

        // Found by spoken (Arabic) text — kind 'source'.
        let hits2 = repo
            .search_in_transcript(id, &nova_search::fts_match("حالك"), 50)
            .await
            .unwrap();
        assert_eq!(hits2.len(), 1);
        assert_eq!(hits2[0].kind, "source");

        // A different transcript id sees nothing.
        assert!(repo
            .search_in_transcript(id + 999, &nova_search::fts_match("hello"), 50)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn delete_triggers_clear_the_index() {
        let (_dir, pool) = temp_pool().await;
        let tr = TranscriptRepository::new(&pool);
        let id = tr
            .get_or_create_transcript("k", "vod", 1, 0, "en", "m", "/a.vtt", "/a.json")
            .await
            .unwrap();
        tr.append_segments(
            id,
            &[TranscriptSegment {
                seq: 0,
                start_sec: 0.0,
                end_sec: 1.0,
                text: "findable phrase".into(),
                words: None,
            }],
        )
        .await
        .unwrap();
        let repo = SearchRepository::new(&pool);
        assert_eq!(
            repo.search_in_transcript(id, &nova_search::fts_match("findable"), 50)
                .await
                .unwrap()
                .len(),
            1
        );
        tr.clear_all().await.unwrap();
        assert!(repo
            .search_in_transcript(id, &nova_search::fts_match("findable"), 50)
            .await
            .unwrap()
            .is_empty());
    }
}
