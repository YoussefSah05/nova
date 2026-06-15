//! Persisted embedding vectors for semantic search (hybrid retrieval).
//!
//! Enrichment already embeds every catalog row on-device; we store those vectors
//! here so meaning-based search costs no extra inference. Transcript segments are
//! embedded lazily on the first dialogue semantic search and cached the same way.
//! Vectors are float32, little-endian, in a BLOB; brute-force cosine in
//! `nova_search` ranks them (no ANN extension at Nova's scale).

use sqlx::SqlitePool;

use crate::error::StorageError;

pub struct EmbeddingRepository<'a> {
    pool: &'a SqlitePool,
}

/// Encode a vector as a little-endian f32 BLOB.
pub fn to_blob(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for v in vector {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

/// Decode a little-endian f32 BLOB back into a vector. Trailing bytes that don't
/// form a full f32 are ignored (defensive; the writer never produces them).
pub fn from_blob(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

impl<'a> EmbeddingRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Upsert `(content_id, vector)` rows for one `kind`/`model` (one
    /// transaction). Re-running enrichment overwrites cleanly.
    pub async fn upsert_many(
        &self,
        kind: &str,
        model: &str,
        rows: &[(i64, Vec<f32>)],
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        for (id, vector) in rows {
            let blob = to_blob(vector);
            sqlx::query!(
                "INSERT OR REPLACE INTO catalog_embeddings (content_kind, content_id, model, vector)
                 VALUES (?, ?, ?, ?)",
                kind,
                id,
                model,
                blob,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// How many catalog vectors exist for a source + kind + model. Cheap enough
    /// to call before every search as a cache-invalidation signal (the in-memory
    /// index reloads only when this count changes — e.g. after a re-enrichment).
    pub async fn count_catalog(
        &self,
        source_id: i64,
        kind: &str,
        model: &str,
    ) -> Result<i64, StorageError> {
        // `movie` joins vod_items, `live` joins channels — both filter by source.
        let row = if kind == "live" {
            sqlx::query!(
                r#"SELECT COUNT(*) as "n!: i64"
                   FROM catalog_embeddings ce
                   JOIN channels c ON c.id = ce.content_id
                   WHERE ce.content_kind = 'live' AND ce.model = ? AND c.source_id = ?"#,
                model,
                source_id,
            )
            .fetch_one(self.pool)
            .await?
            .n
        } else {
            sqlx::query!(
                r#"SELECT COUNT(*) as "n!: i64"
                   FROM catalog_embeddings ce
                   JOIN vod_items v ON v.id = ce.content_id
                   WHERE ce.content_kind = 'movie' AND ce.model = ? AND v.source_id = ?"#,
                model,
                source_id,
            )
            .fetch_one(self.pool)
            .await?
            .n
        };
        Ok(row)
    }

    /// Load every catalog vector for a source + kind + model as `(id, vector)`.
    pub async fn load_catalog(
        &self,
        source_id: i64,
        kind: &str,
        model: &str,
    ) -> Result<Vec<(i64, Vec<f32>)>, StorageError> {
        let rows = if kind == "live" {
            sqlx::query!(
                r#"SELECT ce.content_id as "id!: i64", ce.vector as "vector!: Vec<u8>"
                   FROM catalog_embeddings ce
                   JOIN channels c ON c.id = ce.content_id
                   WHERE ce.content_kind = 'live' AND ce.model = ? AND c.source_id = ?"#,
                model,
                source_id,
            )
            .fetch_all(self.pool)
            .await?
            .into_iter()
            .map(|r| (r.id, from_blob(&r.vector)))
            .collect()
        } else {
            sqlx::query!(
                r#"SELECT ce.content_id as "id!: i64", ce.vector as "vector!: Vec<u8>"
                   FROM catalog_embeddings ce
                   JOIN vod_items v ON v.id = ce.content_id
                   WHERE ce.content_kind = 'movie' AND ce.model = ? AND v.source_id = ?"#,
                model,
                source_id,
            )
            .fetch_all(self.pool)
            .await?
            .into_iter()
            .map(|r| (r.id, from_blob(&r.vector)))
            .collect()
        };
        Ok(rows)
    }

    /// Segments of a transcript that don't yet have an embedding for `model`,
    /// as `(segment_id, text)` — the lazy embed worklist for dialogue search.
    pub async fn segments_missing_embeddings(
        &self,
        transcript_id: i64,
        model: &str,
    ) -> Result<Vec<(i64, String)>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT ts.id as "id!: i64", ts.text as "text!: String"
               FROM transcript_segments ts
               WHERE ts.transcript_id = ?
                 AND NOT EXISTS (
                   SELECT 1 FROM catalog_embeddings ce
                   WHERE ce.content_kind = 'segment' AND ce.content_id = ts.id AND ce.model = ?
                 )
               ORDER BY ts.start_sec ASC"#,
            transcript_id,
            model,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| (r.id, r.text)).collect())
    }

    /// Load segment vectors for a transcript + model as `(segment_id, vector)`.
    pub async fn load_segments(
        &self,
        transcript_id: i64,
        model: &str,
    ) -> Result<Vec<(i64, Vec<f32>)>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT ce.content_id as "id!: i64", ce.vector as "vector!: Vec<u8>"
               FROM catalog_embeddings ce
               JOIN transcript_segments ts ON ts.id = ce.content_id
               WHERE ce.content_kind = 'segment' AND ts.transcript_id = ? AND ce.model = ?"#,
            transcript_id,
            model,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.id, from_blob(&r.vector)))
            .collect())
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

    #[test]
    fn blob_round_trips() {
        let v = vec![0.0f32, 1.5, -2.25, 3.125];
        assert_eq!(from_blob(&to_blob(&v)), v);
        assert_eq!(to_blob(&v).len(), v.len() * 4);
    }

    #[tokio::test]
    async fn segment_vectors_track_their_segments() {
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
                text: "hello".into(),
                words: None,
            }],
        )
        .await
        .unwrap();
        let seg: i64 = sqlx::query_scalar("SELECT id FROM transcript_segments LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();

        let repo = EmbeddingRepository::new(&pool);
        // Initially missing, then embedded.
        assert_eq!(
            repo.segments_missing_embeddings(id, "m").await.unwrap().len(),
            1
        );
        repo.upsert_many("segment", "m", &[(seg, vec![1.0, 0.0, 0.0])])
            .await
            .unwrap();
        assert!(repo
            .segments_missing_embeddings(id, "m")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(repo.load_segments(id, "m").await.unwrap().len(), 1);

        // Clearing the transcript fires the delete trigger → vectors gone.
        tr.clear_all().await.unwrap();
        assert!(repo.load_segments(id, "m").await.unwrap().is_empty());
    }
}
