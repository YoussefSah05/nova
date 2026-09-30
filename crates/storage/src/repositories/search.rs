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

/// A spoken line found across the library: where it was said (a film or an
/// episode of the playlist) and when, so a click plays from that line.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomentHit {
    pub segment_id: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
    /// `source` (spoken) or `translation`.
    pub kind: String,
    /// `vod` or `episode`: what plays.
    pub media_kind: String,
    pub media_id: i64,
    /// The film's title, or the series' title for an episode.
    pub title: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub episode_title: Option<String>,
    /// Whether `season`/`episode` are real (`season_episode` numbering).
    pub numbered: bool,
    pub poster_url: Option<String>,
    /// The canonical movie or series entity, when bound.
    pub entity_id: Option<String>,
}

/// Most of the playlist's FTS matches weighed.
const MOMENT_CANDIDATES: i64 = 400;

impl<'a> SearchRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Dialogue search across every transcript of the playlist's films and
    /// episodes (live transcripts have no replayable timeline), best match
    /// first, one row per spoken line. `match_expr` comes from
    /// `nova_search::fts_match`.
    pub async fn search_moments(
        &self,
        source_id: i64,
        match_expr: &str,
        limit: i64,
    ) -> Result<Vec<MomentHit>, StorageError> {
        let rows = sqlx::query!(
            r#"WITH scoped AS (
                 -- This playlist's transcribed films and episodes: ranking
                 -- happens among them, so another playlist can't crowd them out.
                 SELECT t.id AS transcript_id FROM transcripts t
                 WHERE (t.media_kind = 'vod' AND EXISTS (
                          SELECT 1 FROM vod_items v WHERE v.id = t.media_id AND v.source_id = ?1))
                    OR (t.media_kind = 'episode' AND EXISTS (
                          SELECT 1 FROM series_episodes e
                          WHERE e.id = t.media_id AND e.source_id = ?1))
               ),
               hits AS (
                 SELECT CAST(f.segment_id AS INTEGER) AS segment_id, f.text AS text,
                        f.kind AS kind, bm25(transcript_fts) AS score
                 FROM transcript_fts f
                 JOIN transcript_segments s ON s.id = CAST(f.segment_id AS INTEGER)
                 WHERE f.text MATCH ?2
                   AND s.transcript_id IN (SELECT transcript_id FROM scoped)
                 ORDER BY bm25(transcript_fts)
                 LIMIT ?4
               ),
               best AS (
                 SELECT h.segment_id, h.text, h.kind, h.score,
                        ROW_NUMBER() OVER (
                          PARTITION BY t.media_kind, t.media_id, ROUND(s.start_sec * 2.0)
                          ORDER BY h.score, h.segment_id
                        ) AS rn
                 FROM hits h
                 JOIN transcript_segments s ON s.id = h.segment_id
                 JOIN transcripts t ON t.id = s.transcript_id
               )
               SELECT b.segment_id as "segment_id!: i64",
                      ts.start_sec as "start_sec!: f64", ts.end_sec as "end_sec!: f64",
                      b.text as "text!: String", b.kind as "kind!: String",
                      t.media_kind as "media_kind!: String", t.media_id as "media_id!: i64",
                      CASE WHEN t.media_kind = 'vod'
                           THEN COALESCE(v.canonical_title, v.clean_title, v.name)
                           ELSE COALESCE(sm.canonical_title, se.series_title) END
                          as "title!: String",
                      CASE WHEN t.media_kind = 'episode' THEN se.season END as "season?: i64",
                      CASE WHEN t.media_kind = 'episode' THEN se.episode END as "episode?: i64",
                      se.episode_title as "episode_title?: String",
                      COALESCE(se.numbering = 'season_episode', 0) as "numbered!: bool",
                      CASE WHEN t.media_kind = 'vod' THEN COALESCE(v.poster_url, v.logo_url)
                           ELSE COALESCE(sm.poster_url, se.logo_url) END as "poster_url?: String",
                      CASE WHEN t.media_kind = 'vod'
                           THEN (SELECT s.entity_id FROM stream_variants s
                                 WHERE s.vod_item_id = v.id LIMIT 1)
                           ELSE (SELECT sb.entity_id FROM series_bindings sb
                                 WHERE sb.source_id = se.source_id
                                   AND sb.series_title = se.series_title) END
                          as "entity_id?: String"
               FROM best b
               JOIN transcript_segments ts ON ts.id = b.segment_id
               JOIN transcripts t ON t.id = ts.transcript_id
               LEFT JOIN vod_items v ON t.media_kind = 'vod' AND v.id = t.media_id
               LEFT JOIN series_episodes se ON t.media_kind = 'episode' AND se.id = t.media_id
               LEFT JOIN series_meta sm
                      ON sm.source_id = se.source_id AND sm.series_title = se.series_title
               WHERE b.rn = 1
                 AND ((v.id IS NOT NULL AND v.source_id = ?1)
                      OR (se.id IS NOT NULL AND se.source_id = ?1))
               ORDER BY b.score, b.segment_id
               LIMIT ?3"#,
            source_id,
            match_expr,
            limit,
            MOMENT_CANDIDATES,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| MomentHit {
                segment_id: r.segment_id,
                start_sec: r.start_sec,
                end_sec: r.end_sec,
                text: r.text,
                kind: r.kind,
                media_kind: r.media_kind,
                media_id: r.media_id,
                title: r.title,
                season: r.season,
                episode: r.episode,
                episode_title: r.episode_title,
                numbered: r.numbered,
                poster_url: r.poster_url,
                entity_id: r.entity_id,
            })
            .collect())
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

    #[tokio::test]
    async fn moments_are_found_across_the_playlists_films_and_episodes() {
        let (_dir, pool) = temp_pool().await;
        let sources = crate::SourceRepository::new(&pool);
        let a = sources.create("m3u", "a", "http://a").await.unwrap().id;
        let b = sources.create("m3u", "b", "http://b").await.unwrap().id;
        let insert = |sql: &'static str, source: i64| {
            let pool = pool.clone();
            async move {
                sqlx::query(sql)
                    .bind(source)
                    .execute(&pool)
                    .await
                    .unwrap()
                    .last_insert_rowid()
            }
        };
        let film = insert(
            "INSERT INTO vod_items (source_id, name, canonical_title, stream_url)
             VALUES (?, 'HEAT 1995', 'Heat', 'http://a/1.mkv')",
            a,
        )
        .await;
        let other_film = insert(
            "INSERT INTO vod_items (source_id, name, stream_url) VALUES (?, 'Other', 'http://b/1.mkv')",
            b,
        )
        .await;
        let episode = insert(
            "INSERT INTO series_episodes (source_id, series_title, season, episode, stream_url)
             VALUES (?, 'Dark', 1, 2, 'http://a/e.mkv')",
            a,
        )
        .await;
        let tr = TranscriptRepository::new(&pool);
        let line = |text: &str| TranscriptSegment {
            seq: 0,
            start_sec: 42.0,
            end_sec: 45.0,
            text: text.into(),
            words: None,
        };
        for (kind, id, text) in [
            ("vod", film, "Don't let yourself get attached to anything"),
            ("vod", other_film, "attached to another playlist"),
            ("episode", episode, "Everything is attached"),
            ("live", 1, "a live line, attached"),
        ] {
            let t = tr
                .get_or_create_transcript(
                    &format!("{kind}{id}"),
                    kind,
                    id,
                    0,
                    "en",
                    "m",
                    "/a.vtt",
                    "/a.json",
                )
                .await
                .unwrap();
            tr.append_segments(t, &[line(text)]).await.unwrap();
        }

        let hits = SearchRepository::new(&pool)
            .search_moments(a, &nova_search::fts_match("attached"), 20)
            .await
            .unwrap();
        let mut found: Vec<(String, String, Option<i64>, bool)> = hits
            .iter()
            .map(|h| (h.media_kind.clone(), h.title.clone(), h.episode, h.numbered))
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                ("episode".to_string(), "Dark".to_string(), Some(2), true),
                ("vod".to_string(), "Heat".to_string(), None, false),
            ],
            "the playlist's films and episodes, never another playlist or live"
        );
        assert!(hits.iter().all(|h| h.start_sec == 42.0));

        // Another playlist full of the word can't crowd this one out, and a
        // second transcript of the same title doesn't repeat the line.
        let crowd = tr
            .get_or_create_transcript("crowd", "vod", other_film, 1, "en", "m", "/c", "/c")
            .await
            .unwrap();
        let many: Vec<TranscriptSegment> = (0..MOMENT_CANDIDATES + 50)
            .map(|i| TranscriptSegment {
                seq: i,
                start_sec: i as f64,
                end_sec: i as f64 + 1.0,
                text: "attached".into(),
                words: None,
            })
            .collect();
        tr.append_segments(crowd, &many).await.unwrap();
        let again = tr
            .get_or_create_transcript("film-2", "vod", film, 1, "en", "m2", "/b", "/b")
            .await
            .unwrap();
        tr.append_segments(
            again,
            &[line("Don't let yourself get attached to anything")],
        )
        .await
        .unwrap();
        let hits = SearchRepository::new(&pool)
            .search_moments(a, &nova_search::fts_match("attached"), 20)
            .await
            .unwrap();
        assert_eq!(hits.len(), 2, "{hits:?}");

        // Duplicate higher-ranked episode lines must not consume the film's
        // slot before the requested result limit is applied.
        let duplicate = tr
            .get_or_create_transcript("episode-2", "episode", episode, 1, "en", "m2", "/d", "/d")
            .await
            .unwrap();
        tr.append_segments(duplicate, &[line("Everything is attached")])
            .await
            .unwrap();
        let hits = SearchRepository::new(&pool)
            .search_moments(a, &nova_search::fts_match("attached"), 2)
            .await
            .unwrap();
        assert_eq!(
            hits.len(),
            2,
            "unique moments fill the requested page: {hits:?}"
        );
        assert!(hits.iter().any(|h| h.media_kind == "vod"));
    }
}
