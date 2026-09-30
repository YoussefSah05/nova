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

/// A source segment with its DB id — the id is the translation-cache key, so the
/// Phase 7 translation session needs it (plain `get_segments` omits it).
#[derive(Debug, Clone)]
pub struct SourceSegment {
    pub id: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
}

/// One transcript line for the Phase 8 companion panel: the source text plus its
/// translation in the active target language (when one is cached).
#[derive(Debug, Clone)]
pub struct TranscriptLine {
    pub segment_id: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
    pub translation: Option<String>,
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

    /// Re-link transcripts of films and episodes whose catalog row a refresh
    /// replaced (the delete trigger negates `media_id`) to the source's new
    /// row with the same stream: `key_of` is the content key of a stream URL.
    /// Only hashes the source's URLs when orphans exist. Returns re-linked.
    pub async fn reattach_orphans_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        source_id: i64,
        key_of: fn(&str) -> String,
    ) -> Result<u64, StorageError> {
        let orphans = sqlx::query!(
            r#"SELECT t.id as "id!: i64", t.media_kind as "kind!: String",
                      t.content_key as "key!: String"
               FROM transcripts t
               WHERE t.content_key <> '' AND t.media_kind IN ('vod', 'episode')
                 AND NOT EXISTS (SELECT 1 FROM vod_items v
                                 WHERE t.media_kind = 'vod' AND v.id = t.media_id)
                 AND NOT EXISTS (SELECT 1 FROM series_episodes e
                                 WHERE t.media_kind = 'episode' AND e.id = t.media_id)"#
        )
        .fetch_all(&mut **tx)
        .await?;
        if orphans.is_empty() {
            return Ok(0);
        }
        let mut wanted: std::collections::HashMap<(String, String), Vec<i64>> =
            std::collections::HashMap::new();
        for o in orphans {
            wanted.entry((o.kind, o.key)).or_default().push(o.id);
        }
        let mut rows: Vec<(&'static str, i64, String)> = Vec::new();
        if wanted.keys().any(|(k, _)| k == "vod") {
            for r in sqlx::query!(
                r#"SELECT id as "id!: i64", stream_url FROM vod_items WHERE source_id = ?"#,
                source_id
            )
            .fetch_all(&mut **tx)
            .await?
            {
                rows.push(("vod", r.id, r.stream_url));
            }
        }
        if wanted.keys().any(|(k, _)| k == "episode") {
            for r in sqlx::query!(
                r#"SELECT id as "id!: i64", stream_url FROM series_episodes WHERE source_id = ?"#,
                source_id
            )
            .fetch_all(&mut **tx)
            .await?
            {
                rows.push(("episode", r.id, r.stream_url));
            }
        }
        let mut relinked = 0;
        for (kind, row_id, url) in rows {
            let Some(transcripts) = wanted.remove(&(kind.to_string(), key_of(&url))) else {
                continue;
            };
            for transcript in transcripts {
                relinked += sqlx::query!(
                    "UPDATE transcripts SET media_id = ? WHERE id = ?",
                    row_id,
                    transcript
                )
                .execute(&mut **tx)
                .await?
                .rows_affected();
            }
        }
        Ok(relinked)
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

    /// Number of cached transcripts (for the settings cache view).
    pub async fn count(&self) -> Result<i64, StorageError> {
        let row = sqlx::query!(r#"SELECT COUNT(*) as "n!: i64" FROM transcripts"#)
            .fetch_one(self.pool)
            .await?;
        Ok(row.n)
    }

    /// Delete every cached transcript (segments/windows cascade) and return the
    /// on-disk artifact paths so the caller can remove the VTT/JSON files.
    pub async fn clear_all(&self) -> Result<Vec<String>, StorageError> {
        let rows = sqlx::query!("SELECT vtt_path, json_path FROM transcripts")
            .fetch_all(self.pool)
            .await?;
        let mut paths = Vec::with_capacity(rows.len() * 2);
        for r in rows {
            paths.push(r.vtt_path);
            paths.push(r.json_path);
        }
        sqlx::query!("DELETE FROM transcripts")
            .execute(self.pool)
            .await?;
        Ok(paths)
    }

    /// Source segments with their DB ids, ordered by start. The id is the
    /// translation-cache key (`transcript_translations.segment_id`), so the
    /// Phase 7 streaming translator joins translations back to start/end times.
    pub async fn segments_with_ids(
        &self,
        transcript_id: i64,
    ) -> Result<Vec<SourceSegment>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", start_sec as "start_sec!: f64",
                      end_sec as "end_sec!: f64", text
               FROM transcript_segments
               WHERE transcript_id = ?
               ORDER BY start_sec ASC"#,
            transcript_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| SourceSegment {
                id: r.id,
                start_sec: r.start_sec,
                end_sec: r.end_sec,
                text: r.text,
            })
            .collect())
    }

    /// Source lines joined to their translation in `target_lang`/`backend` (when
    /// cached), ordered by time — the Phase 8 companion panel's read. Passing an
    /// empty `target_lang` (or one with no cached translations) leaves every
    /// `translation` as `None`. `UNIQUE(segment_id,target_lang,backend)` keeps the
    /// join to at most one translation row per segment.
    pub async fn transcript_with_translations(
        &self,
        transcript_id: i64,
        target_lang: &str,
        backend: &str,
    ) -> Result<Vec<TranscriptLine>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT ts.id as "segment_id!: i64",
                      ts.start_sec as "start_sec!: f64",
                      ts.end_sec   as "end_sec!: f64",
                      ts.text,
                      tt.text      as "translation?: String"
               FROM transcript_segments ts
               LEFT JOIN transcript_translations tt
                 ON tt.segment_id = ts.id AND tt.target_lang = ? AND tt.backend = ?
               WHERE ts.transcript_id = ?
               ORDER BY ts.start_sec ASC"#,
            target_lang,
            backend,
            transcript_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| TranscriptLine {
                segment_id: r.segment_id,
                start_sec: r.start_sec,
                end_sec: r.end_sec,
                text: r.text,
                translation: r.translation,
            })
            .collect())
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
    async fn a_refresh_keeps_a_titles_transcript_attached() {
        let (_dir, pool) = temp_pool().await;
        let source = crate::SourceRepository::new(&pool)
            .create("m3u", "a", "http://a")
            .await
            .unwrap()
            .id;
        let key_of: fn(&str) -> String = |url| format!("key:{url}");
        let insert = || {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO vod_items (source_id, name, stream_url) VALUES (?, 'Heat', 'http://a/heat.mkv')",
                )
                .bind(source)
                .execute(&pool)
                .await
                .unwrap()
                .last_insert_rowid()
            }
        };
        let old_row = insert().await;
        let t = TranscriptRepository::new(&pool)
            .get_or_create_transcript(
                &key_of("http://a/heat.mkv"),
                "vod",
                old_row,
                0,
                "en",
                "m",
                "/a.vtt",
                "/a.json",
            )
            .await
            .unwrap();
        // A refresh replaces the row (the delete trigger detaches the transcript).
        sqlx::query("DELETE FROM vod_items WHERE id = ?")
            .bind(old_row)
            .execute(&pool)
            .await
            .unwrap();
        let new_row = insert().await;
        let mut tx = pool.begin().await.unwrap();
        let relinked = TranscriptRepository::reattach_orphans_tx(&mut tx, source, key_of)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(relinked, 1);
        let media: i64 = sqlx::query_scalar("SELECT media_id FROM transcripts WHERE id = ?")
            .bind(t)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(media, new_row);
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
