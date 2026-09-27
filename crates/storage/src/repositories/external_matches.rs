//! External metadata matches (TMDB) attached to Nova's canonical entities.
//!
//! A match is **evidence on** an entity, never its identity: the entity keeps
//! its Nova ID, and a TMDB ID is stored as `external_id` with the provenance of
//! how it was chosen. Search-derived IDs are deliberately not turned into
//! identity aliases, so they can never merge entities (P09's resolver only trusts
//! provider-supplied IDs).
//!
//! Matching is progressive: [`ExternalMatchRepository::movie_work`] returns the
//! entities still needing a decision, most important first. Callers apply
//! results in small batches ([`ExternalMatchRepository::apply_movie_matches`]),
//! each committed on its own, so progress is visible and a restart resumes.

use nova_core::catalog::EntityId;
use nova_core::now_ts;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;
use crate::repositories::canonical::parse_entity;

/// Namespace of TMDB movie IDs (authority + media kind).
pub const TMDB_MOVIE: &str = "tmdb:movie";

/// Order in which titles are matched: what the user is most likely to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchPriority {
    /// Favourite or on the watchlist.
    Personal,
    /// Started but not finished.
    InProgress,
    /// Everything else, most recently added first.
    Catalog,
}

/// One movie entity awaiting a match decision.
#[derive(Debug, Clone)]
pub struct MovieMatchWork {
    pub entity_id: EntityId,
    /// Entity display title (cleaned at compile time).
    pub title: String,
    /// Raw provider title of the representative row in this source.
    pub source_title: Option<String>,
    pub year: Option<i32>,
    /// Provider-supplied TMDB IDs from durable identity aliases.
    pub trusted_tmdb_ids: Vec<i64>,
    pub priority: MatchPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMethod {
    TrustedId,
    Search,
}

impl MatchMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::TrustedId => "trusted_id",
            Self::Search => "search",
        }
    }
}

/// Metadata attached only for an accepted match.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MatchedMetadata {
    pub external_id: String,
    /// TMDB title in `title_language`.
    pub canonical_title: String,
    /// Title in the work's original language, kept apart from the display title.
    pub original_title: Option<String>,
    /// BCP-47 tag of `canonical_title` (the metadata language requested).
    pub title_language: String,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub overview: Option<String>,
    /// Canonical genre names, primary first.
    pub genres: Vec<String>,
    /// Directors then top-billed cast (searchable metadata).
    pub people: Vec<String>,
    /// TMDB keywords (searchable metadata).
    pub keywords: Vec<String>,
}

/// One decision to persist. `matched: None` records an unresolved title.
#[derive(Debug, Clone)]
pub struct ExternalMatchWrite {
    pub entity_id: EntityId,
    pub matched: Option<MatchedMetadata>,
    pub method: MatchMethod,
    pub confidence_bp: Option<u16>,
    pub matcher_version: u32,
    /// Secret-free provenance (JSON).
    pub evidence_json: String,
}

/// A stored decision.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalMatch {
    pub entity_id: EntityId,
    pub status: String,
    pub external_id: Option<String>,
    pub method: String,
    pub confidence_bp: Option<i64>,
    pub matcher_version: i64,
    pub evidence_json: String,
    pub canonical_title: Option<String>,
    pub poster_url: Option<String>,
    pub checked_at: i64,
}

pub struct ExternalMatchRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ExternalMatchRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Active movie entities with an available variant in `source_id` that
    /// need a decision: never matched, matched by an older matcher or in
    /// another metadata language, or left unresolved before
    /// `retry_unresolved_before`. Most important first.
    pub async fn movie_work(
        &self,
        source_id: i64,
        matcher_version: u32,
        retry_unresolved_before: i64,
        language: &str,
    ) -> Result<Vec<MovieMatchWork>, StorageError> {
        let version = i64::from(matcher_version);
        let rows = sqlx::query!(
            r#"SELECT e.id as "id!: String",
                      e.title as "title!: String",
                      e.year as "year?: i64",
                      (SELECT v.name FROM stream_variants s
                       JOIN vod_items v ON v.id = s.vod_item_id
                       WHERE s.entity_id = e.id AND s.source_id = ?
                       ORDER BY v.id LIMIT 1) as "source_title?: String",
                      (SELECT group_concat(a.value, ' ') FROM entity_aliases a
                       WHERE a.entity_id = e.id AND a.alias_kind = 'external'
                         AND a.value LIKE 'tmdb:movie:%') as "trusted?: String",
                      CASE WHEN e.is_favorite = 1 OR e.is_in_watchlist = 1 THEN 0
                           WHEN w.entity_id IS NOT NULL AND w.completed = 0 THEN 1
                           ELSE 2 END as "tier!: i64",
                      (SELECT MAX(v.added_at) FROM stream_variants s
                       JOIN vod_items v ON v.id = s.vod_item_id
                       WHERE s.entity_id = e.id) as "added?: i64"
               FROM canonical_entities e
               LEFT JOIN entity_watch_state w ON w.entity_id = e.id
               LEFT JOIN entity_external_matches m
                 ON m.entity_id = e.id AND m.namespace = 'tmdb:movie'
               WHERE e.kind = 'movie' AND e.state = 'active'
                 AND EXISTS (SELECT 1 FROM stream_variants s
                             WHERE s.entity_id = e.id AND s.source_id = ?
                               AND s.vod_item_id IS NOT NULL)
                 AND (m.entity_id IS NULL
                      OR m.matcher_version < ?
                      OR (m.status = 'unresolved' AND m.checked_at < ?)
                      OR (m.status = 'matched' AND COALESCE(m.title_language, 'en-US') <> ?))
               ORDER BY 6, 7 DESC, e.id"#,
            source_id,
            source_id,
            version,
            retry_unresolved_before,
            language,
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                let trusted_tmdb_ids = r
                    .trusted
                    .as_deref()
                    .unwrap_or("")
                    .split(' ')
                    .filter_map(|v| v.strip_prefix("tmdb:movie:"))
                    .filter_map(|v| v.parse::<i64>().ok())
                    .filter(|id| *id > 0)
                    .collect();
                Ok(MovieMatchWork {
                    entity_id: parse_entity(r.id)?,
                    title: r.title,
                    source_title: r.source_title,
                    year: r.year.and_then(|y| i32::try_from(y).ok()),
                    trusted_tmdb_ids,
                    priority: match r.tier {
                        0 => MatchPriority::Personal,
                        1 => MatchPriority::InProgress,
                        _ => MatchPriority::Catalog,
                    },
                })
            })
            .collect()
    }

    /// Persist one batch of decisions atomically, and mirror them onto the
    /// entities' provider rows for surfaces that still read `vod_items`.
    pub async fn apply_movie_matches(
        &self,
        writes: &[ExternalMatchWrite],
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        let now = now_ts();
        for w in writes {
            let entity = w.entity_id.as_str();
            let method = w.method.as_str();
            let confidence = w.confidence_bp.map(i64::from);
            let version = i64::from(w.matcher_version);
            let m = w.matched.as_ref();
            let status = if m.is_some() { "matched" } else { "unresolved" };
            let external_id = m.map(|m| m.external_id.as_str());
            let title = m.map(|m| m.canonical_title.as_str());
            let original_title = m.and_then(|m| m.original_title.as_deref());
            let people = m
                .filter(|m| !m.people.is_empty())
                .map(|m| m.people.join(", "));
            let keywords = m
                .filter(|m| !m.keywords.is_empty())
                .map(|m| m.keywords.join(", "));
            let title_language = m.map(|m| m.title_language.as_str());
            let poster = m.and_then(|m| m.poster_url.as_deref());
            let backdrop = m.and_then(|m| m.backdrop_url.as_deref());
            let overview = m.and_then(|m| m.overview.as_deref());
            let genres = m
                .filter(|m| !m.genres.is_empty())
                .map(|m| m.genres.join(", "));
            let primary = m.and_then(|m| m.genres.first().map(String::as_str));
            sqlx::query!(
                r#"INSERT INTO entity_external_matches
                       (entity_id, namespace, status, external_id, method, confidence_bp,
                        matcher_version, evidence_json, canonical_title, poster_url,
                        backdrop_url, overview, genres, primary_genre, checked_at,
                        original_title, title_language, people, keywords)
                   VALUES (?, 'tmdb:movie', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(entity_id, namespace) DO UPDATE SET
                       status = excluded.status,
                       external_id = excluded.external_id,
                       method = excluded.method,
                       confidence_bp = excluded.confidence_bp,
                       matcher_version = excluded.matcher_version,
                       evidence_json = excluded.evidence_json,
                       canonical_title = excluded.canonical_title,
                       poster_url = excluded.poster_url,
                       backdrop_url = excluded.backdrop_url,
                       overview = excluded.overview,
                       genres = excluded.genres,
                       primary_genre = excluded.primary_genre,
                       checked_at = excluded.checked_at,
                       original_title = excluded.original_title,
                       title_language = excluded.title_language,
                       people = excluded.people,
                       keywords = excluded.keywords"#,
                entity,
                status,
                external_id,
                method,
                confidence,
                version,
                w.evidence_json,
                title,
                poster,
                backdrop,
                overview,
                genres,
                primary,
                now,
                original_title,
                title_language,
                people,
                keywords,
            )
            .execute(&mut *tx)
            .await?;
            let row_status = if m.is_some() { "matched" } else { "unmatched" };
            let matched: i64 = m.is_some().into();
            // Genre: a confident TMDB genre wins over provider/inferred ones;
            // an unresolved decision removes only a TMDB genre. `category`
            // (the browse facet) is recomputed as COALESCE(genre, collection).
            sqlx::query!(
                r#"UPDATE vod_items
                   SET canonical_title = ?, poster_url = ?, backdrop_url = ?, overview = ?,
                       tmdb_genres = ?, tmdb_status = ?,
                       genre = CASE WHEN ? IS NOT NULL THEN ?
                                    WHEN ? = 0 AND genre_source = 'tmdb' THEN NULL
                                    ELSE genre END,
                       genre_source = CASE WHEN ? IS NOT NULL THEN 'tmdb'
                                           WHEN ? = 0 AND genre_source = 'tmdb' THEN NULL
                                           ELSE genre_source END,
                       category = COALESCE(CASE WHEN ? IS NOT NULL THEN ?
                                                WHEN ? = 0 AND genre_source = 'tmdb' THEN NULL
                                                ELSE genre END,
                                           collection, category)
                   WHERE id IN (
                       SELECT v.id FROM vod_items v
                       JOIN stream_variants s
                         ON s.source_id = v.source_id AND s.binding_key = v.binding_key
                       WHERE s.entity_id = ?)"#,
                title,
                poster,
                backdrop,
                overview,
                genres,
                row_status,
                primary,
                primary,
                matched,
                primary,
                matched,
                primary,
                primary,
                matched,
                entity,
            )
            .execute(&mut *tx)
            .await?;
        }
        // Search documents carry TMDB titles, genres, keywords and people.
        let entities: Vec<EntityId> = writes.iter().map(|w| w.entity_id.clone()).collect();
        crate::CatalogSearchRepository::refresh_movies_tx(&mut tx, &entities).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Re-derive the row mirror for a source after an import replaced its rows
    /// (runs in the import transaction, after entity resolution).
    pub async fn mirror_movies_for_source_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            r#"UPDATE vod_items
               SET canonical_title = m.canonical_title,
                   poster_url = m.poster_url,
                   backdrop_url = m.backdrop_url,
                   overview = m.overview,
                   tmdb_genres = m.genres,
                   tmdb_status = CASE m.status WHEN 'matched' THEN 'matched' ELSE 'unmatched' END,
                   genre = COALESCE(CASE WHEN m.status = 'matched' THEN m.primary_genre END,
                                    vod_items.genre),
                   genre_source = CASE WHEN m.status = 'matched' AND m.primary_genre IS NOT NULL
                                       THEN 'tmdb' ELSE vod_items.genre_source END,
                   category = COALESCE(CASE WHEN m.status = 'matched' THEN m.primary_genre END,
                                       vod_items.genre, vod_items.collection, vod_items.category)
               FROM stream_variants s
               JOIN entity_external_matches m
                 ON m.entity_id = s.entity_id AND m.namespace = 'tmdb:movie'
               WHERE vod_items.source_id = ?
                 AND s.source_id = vod_items.source_id
                 AND s.binding_key = vod_items.binding_key"#,
            source_id,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn movie_match(
        &self,
        entity_id: &EntityId,
    ) -> Result<Option<ExternalMatch>, StorageError> {
        let id = entity_id.as_str();
        let row = sqlx::query!(
            r#"SELECT entity_id as "entity_id!: String", status as "status!: String",
                      external_id, method as "method!: String", confidence_bp,
                      matcher_version as "matcher_version!: i64",
                      evidence_json as "evidence_json!: String",
                      canonical_title, poster_url, checked_at as "checked_at!: i64"
               FROM entity_external_matches
               WHERE entity_id = ? AND namespace = 'tmdb:movie'"#,
            id,
        )
        .fetch_optional(self.pool)
        .await?;
        row.map(|r| {
            Ok(ExternalMatch {
                entity_id: parse_entity(r.entity_id)?,
                status: r.status,
                external_id: r.external_id,
                method: r.method,
                confidence_bp: r.confidence_bp,
                matcher_version: r.matcher_version,
                evidence_json: r.evidence_json,
                canonical_title: r.canonical_title,
                poster_url: r.poster_url,
                checked_at: r.checked_at,
            })
        })
        .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        init_profile_db, CanonicalRepository, MovieFilters, SourceRepository, VodRepository,
        WatchHistoryRepository,
    };
    use nova_core::{ContentSort, VodItem};

    struct Db {
        _dir: tempfile::TempDir,
        pool: SqlitePool,
        source: i64,
    }

    async fn db() -> Db {
        let dir = tempfile::tempdir().unwrap();
        let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
            .await
            .unwrap();
        let source = SourceRepository::new(&pool)
            .create("xtream", "t", "http://host/get.php")
            .await
            .unwrap()
            .id;
        Db {
            _dir: dir,
            pool,
            source,
        }
    }

    fn movie(source_id: i64, name: &str, n: u32, year: Option<i64>) -> VodItem {
        VodItem {
            id: 0,
            source_id,
            name: name.into(),
            clean_title: Some(name.into()),
            year,
            logo_url: None,
            stream_url: format!("http://host/movie/u/p/{n}.mkv"),
            category: None,
            is_favorite: false,
            is_in_watchlist: false,
            added_at: None,
            dedup_group: None,
            canonical_title: None,
            poster_url: None,
            backdrop_url: None,
            overview: None,
            upstream_id: Some(format!("xtream:vod:{n}")),
            provider_tmdb_id: None,
            evidence: Default::default(),
        }
    }

    /// Same pipeline order as the importer: (replace), insert, compile,
    /// resolve, mirror.
    async fn import(db: &Db, rows: &[VodItem], replace: bool) {
        let mut tx = db.pool.begin().await.unwrap();
        if replace {
            WatchHistoryRepository::delete_for_source_media_tx(&mut tx, db.source)
                .await
                .unwrap();
            VodRepository::delete_by_source_tx(&mut tx, db.source)
                .await
                .unwrap();
        }
        VodRepository::insert_batch_tx(&mut tx, rows).await.unwrap();
        CanonicalRepository::compile_movies_tx(&mut tx, db.source)
            .await
            .unwrap();
        CanonicalRepository::resolve_movies_tx(&mut tx)
            .await
            .unwrap();
        ExternalMatchRepository::mirror_movies_for_source_tx(&mut tx, db.source)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn entity_of(db: &Db, title: &str) -> EntityId {
        CanonicalRepository::new(&db.pool)
            .list_movies(
                db.source,
                100,
                0,
                &MovieFilters::default(),
                ContentSort::Default,
            )
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.title == title || m.source_title == title)
            .unwrap()
            .entity_id
    }

    fn matched(entity: &EntityId, tmdb: &str, title: &str) -> ExternalMatchWrite {
        ExternalMatchWrite {
            entity_id: entity.clone(),
            matched: Some(MatchedMetadata {
                external_id: tmdb.into(),
                canonical_title: title.into(),
                poster_url: Some(format!("https://image.tmdb.org/t/p/w500/{tmdb}.jpg")),
                backdrop_url: None,
                overview: Some("Overview".into()),
                genres: vec!["Drama".into(), "Crime".into()],
                people: Vec::new(),
                keywords: Vec::new(),
                original_title: Some(title.into()),
                title_language: "en-US".into(),
            }),
            method: MatchMethod::Search,
            confidence_bp: Some(9_000),
            matcher_version: 2,
            evidence_json: r#"{"outcome":"matched"}"#.into(),
        }
    }

    fn unresolved(entity: &EntityId) -> ExternalMatchWrite {
        ExternalMatchWrite {
            entity_id: entity.clone(),
            matched: None,
            method: MatchMethod::Search,
            confidence_bp: Some(4_000),
            matcher_version: 2,
            evidence_json: r#"{"outcome":"low_confidence"}"#.into(),
        }
    }

    async fn work_ids(db: &Db, version: u32, retry_before: i64) -> Vec<EntityId> {
        ExternalMatchRepository::new(&db.pool)
            .movie_work(db.source, version, retry_before, "en-US")
            .await
            .unwrap()
            .into_iter()
            .map(|w| w.entity_id)
            .collect()
    }

    #[tokio::test]
    async fn work_is_ordered_personal_then_in_progress_then_catalog() {
        let db = db().await;
        import(
            &db,
            &[
                movie(db.source, "Alpha", 1, Some(2001)),
                movie(db.source, "Bravo", 2, Some(2002)),
                movie(db.source, "Charlie", 3, Some(2003)),
            ],
            false,
        )
        .await;
        let (a, b, c) = (
            entity_of(&db, "Alpha").await,
            entity_of(&db, "Bravo").await,
            entity_of(&db, "Charlie").await,
        );
        let canonical = CanonicalRepository::new(&db.pool);
        canonical.set_in_watchlist(&c, true).await.unwrap();
        canonical
            .record_progress(&b, 30.0, Some(100.0), false, None)
            .await
            .unwrap();
        let work = ExternalMatchRepository::new(&db.pool)
            .movie_work(db.source, 2, 0, "en-US")
            .await
            .unwrap();
        let order: Vec<_> = work.iter().map(|w| (&w.entity_id, w.priority)).collect();
        assert_eq!(
            order,
            vec![
                (&c, MatchPriority::Personal),
                (&b, MatchPriority::InProgress),
                (&a, MatchPriority::Catalog),
            ]
        );
        assert_eq!(work[2].year, Some(2001));
        assert_eq!(work[2].source_title.as_deref(), Some("Alpha"));
    }

    #[tokio::test]
    async fn decided_titles_leave_the_queue_until_a_version_bump_or_retry_age() {
        let db = db().await;
        import(
            &db,
            &[
                movie(db.source, "Alpha", 1, Some(2001)),
                movie(db.source, "Bravo", 2, Some(2002)),
            ],
            false,
        )
        .await;
        let (a, b) = (entity_of(&db, "Alpha").await, entity_of(&db, "Bravo").await);
        let repo = ExternalMatchRepository::new(&db.pool);
        repo.apply_movie_matches(&[matched(&a, "603", "The Alpha"), unresolved(&b)])
            .await
            .unwrap();
        // Same matcher version, unresolved retried only after its retry age.
        assert!(work_ids(&db, 2, 0).await.is_empty());
        assert_eq!(work_ids(&db, 2, now_ts() + 1).await, vec![b.clone()]);
        // A newer matcher revisits everything it has not decided itself.
        let mut all = work_ids(&db, 3, 0).await;
        all.sort();
        let mut expected = vec![a.clone(), b.clone()];
        expected.sort();
        assert_eq!(all, expected);
    }

    #[tokio::test]
    async fn a_metadata_language_change_requeues_matched_titles_only() {
        let db = db().await;
        import(
            &db,
            &[
                movie(db.source, "Alpha", 1, Some(2001)),
                movie(db.source, "Bravo", 2, Some(2002)),
            ],
            false,
        )
        .await;
        let (a, b) = (entity_of(&db, "Alpha").await, entity_of(&db, "Bravo").await);
        let repo = ExternalMatchRepository::new(&db.pool);
        repo.apply_movie_matches(&[matched(&a, "603", "The Alpha"), unresolved(&b)])
            .await
            .unwrap();
        let queued = |lang: &'static str| {
            let pool = db.pool.clone();
            let source = db.source;
            async move {
                ExternalMatchRepository::new(&pool)
                    .movie_work(source, 2, 0, lang)
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|w| w.entity_id)
                    .collect::<Vec<_>>()
            }
        };
        assert!(queued("en-US").await.is_empty());
        assert_eq!(
            queued("fr-FR").await,
            vec![a.clone()],
            "only the localized title is refetched"
        );
        let original: Option<String> = sqlx::query_scalar(
            "SELECT original_title FROM entity_external_matches WHERE entity_id = ?",
        )
        .bind(a.as_str())
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(original.as_deref(), Some("The Alpha"));
    }

    #[tokio::test]
    async fn matches_are_mirrored_and_survive_a_refresh_without_rematching() {
        let db = db().await;
        import(&db, &[movie(db.source, "Heat FHD", 1, Some(1995))], false).await;
        let heat = entity_of(&db, "Heat FHD").await;
        let repo = ExternalMatchRepository::new(&db.pool);
        repo.apply_movie_matches(&[matched(&heat, "949", "Heat")])
            .await
            .unwrap();
        let rows = |db: &Db| {
            let pool = db.pool.clone();
            let source = db.source;
            async move {
                VodRepository::new(&pool)
                    .list(
                        source,
                        10,
                        0,
                        None,
                        None,
                        false,
                        false,
                        ContentSort::Default,
                    )
                    .await
                    .unwrap()
            }
        };
        let before = rows(&db).await;
        assert_eq!(before[0].canonical_title.as_deref(), Some("Heat"));
        assert_eq!(
            before[0].category.as_deref(),
            Some("Drama"),
            "empty category filled"
        );

        // Refresh: new rows arrive without TMDB fields; the entity's match is
        // mirrored back and the entity is not queued again.
        // (A new title listed first makes SQLite hand Heat a new row ID.)
        import(
            &db,
            &[
                movie(db.source, "New arrival", 2, Some(2024)),
                movie(db.source, "Heat FHD", 1, Some(1995)),
            ],
            true,
        )
        .await;
        let after: Vec<_> = rows(&db)
            .await
            .into_iter()
            .filter(|r| r.name == "Heat FHD")
            .collect();
        assert_ne!(after[0].id, before[0].id, "rows were replaced");
        assert_eq!(after[0].canonical_title.as_deref(), Some("Heat"));
        assert!(after[0].poster_url.as_deref().unwrap().contains("949"));
        let arrival = entity_of(&db, "New arrival").await;
        assert_eq!(
            work_ids(&db, 2, 0).await,
            vec![arrival],
            "only the new title is queued"
        );
        let stored = repo.movie_match(&heat).await.unwrap().unwrap();
        assert_eq!(
            (
                stored.status.as_str(),
                stored.external_id.as_deref(),
                stored.method.as_str()
            ),
            ("matched", Some("949"), "search")
        );
        assert_eq!(stored.confidence_bp, Some(9_000));
        assert!(stored.evidence_json.contains("matched"));

        // Becoming unresolved (e.g. under a stricter matcher) clears the display.
        repo.apply_movie_matches(&[unresolved(&heat)])
            .await
            .unwrap();
        let cleared: Vec<_> = rows(&db)
            .await
            .into_iter()
            .filter(|r| r.name == "Heat FHD")
            .collect();
        assert!(cleared[0].canonical_title.is_none() && cleared[0].poster_url.is_none());
    }

    #[tokio::test]
    async fn a_shared_tmdb_id_never_merges_or_renames_nova_entities() {
        let db = db().await;
        // Two distinct provider items the resolver keeps apart (no year).
        import(
            &db,
            &[
                movie(db.source, "Solaris", 1, None),
                movie(db.source, "Solaris Remastered", 2, None),
            ],
            false,
        )
        .await;
        let (a, b) = (
            entity_of(&db, "Solaris").await,
            entity_of(&db, "Solaris Remastered").await,
        );
        ExternalMatchRepository::new(&db.pool)
            .apply_movie_matches(&[matched(&a, "593", "Solaris"), matched(&b, "593", "Solaris")])
            .await
            .unwrap();
        let mut tx = db.pool.begin().await.unwrap();
        let report = CanonicalRepository::resolve_movies_tx(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            report.applied, 0,
            "search matches are not identity evidence"
        );
        assert_ne!(a, b);
        assert!(!a.as_str().contains("593") && !b.as_str().contains("593"));
        let aliases: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM entity_aliases WHERE alias_kind = 'external'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(aliases, 0, "no identity alias from a search match");
    }

    #[tokio::test]
    async fn genre_precedence_keeps_provider_collection_and_origin_separate() {
        let db = db().await;
        let with_group = |name: &str, n: u32, collection: &str, genre: Option<&str>| {
            let mut m = movie(db.source, name, n, Some(2000));
            m.evidence.collection = Some(collection.into());
            m.evidence.provider_genre = genre.map(str::to_string);
            m.category = m.evidence.facet();
            m
        };
        // "FR | FILMS ACTION" carries genre evidence; "FR MOVIES 4K" does not.
        import(
            &db,
            &[
                with_group("Heat", 1, "Action", Some("Action")),
                with_group("Alien", 2, "Movies", None),
            ],
            false,
        )
        .await;
        let row = |title: &'static str| {
            let pool = db.pool.clone();
            async move {
                sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>, Option<String>)>(
                    "SELECT collection, genre, genre_source, category FROM vod_items WHERE name = ?",
                )
                .bind(title)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        let s = |v: &str| Some(v.to_string());
        assert_eq!(
            row("Heat").await,
            (s("Action"), s("Action"), s("provider"), s("Action"))
        );
        assert_eq!(
            row("Alien").await,
            (s("Movies"), None, None, s("Movies")),
            "no genre from a bucket"
        );

        // An inferred genre fills only a missing genre, never over provider evidence.
        let ids: Vec<(i64, String)> = sqlx::query_as("SELECT id, name FROM vod_items")
            .fetch_all(&db.pool)
            .await
            .unwrap();
        let updates: Vec<_> = ids
            .iter()
            .map(|(id, _)| crate::EnrichUpdate {
                id: *id,
                dedup_group: None,
                genre: Some("Comedy".into()),
            })
            .collect();
        VodRepository::new(&db.pool)
            .apply_enrichment(&updates)
            .await
            .unwrap();
        assert_eq!(row("Heat").await.1, s("Action"));
        assert_eq!(
            row("Alien").await,
            (s("Movies"), s("Comedy"), s("inferred"), s("Comedy"))
        );

        // A confident TMDB genre wins; the provider collection is untouched.
        let heat = entity_of(&db, "Heat").await;
        let repo = ExternalMatchRepository::new(&db.pool);
        repo.apply_movie_matches(&[matched(&heat, "949", "Heat")])
            .await
            .unwrap();
        assert_eq!(
            row("Heat").await,
            (s("Action"), s("Drama"), s("tmdb"), s("Drama"))
        );

        // Unresolved removes only the TMDB genre; the facet falls back to the collection.
        repo.apply_movie_matches(&[unresolved(&heat)])
            .await
            .unwrap();
        assert_eq!(row("Heat").await, (s("Action"), None, None, s("Action")));
        // An inferred genre is not touched by an unresolved TMDB decision.
        let alien = entity_of(&db, "Alien").await;
        repo.apply_movie_matches(&[unresolved(&alien)])
            .await
            .unwrap();
        assert_eq!(row("Alien").await.2, s("inferred"));
    }

    #[tokio::test]
    async fn stored_matches_reject_inconsistent_rows() {
        let db = db().await;
        import(&db, &[movie(db.source, "Heat", 1, Some(1995))], false).await;
        let heat = entity_of(&db, "Heat").await;
        for sql in [
            // matched without an external ID
            "INSERT INTO entity_external_matches (entity_id, namespace, status, method, matcher_version, evidence_json, checked_at) VALUES (?, 'tmdb:movie', 'matched', 'search', 2, '{}', 0)",
            // unresolved carrying display metadata
            "INSERT INTO entity_external_matches (entity_id, namespace, status, method, matcher_version, evidence_json, canonical_title, checked_at) VALUES (?, 'tmdb:movie', 'unresolved', 'search', 2, '{}', 'Heat', 0)",
        ] {
            assert!(sqlx::query(sql)
                .bind(heat.as_str())
                .execute(&db.pool)
                .await
                .is_err());
        }
    }
}
