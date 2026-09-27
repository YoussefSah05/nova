//! Catalog publication: measuring a candidate, validating it and recording the
//! outcome.
//!
//! The candidate is built in the import transaction. [`Self::measure_tx`] and
//! [`Self::candidate_report_tx`] read it inside that transaction, the core
//! policy (`nova_core::catalog::publish`) decides, and
//! [`Self::record_published_tx`] marks it active in the same commit. A failed
//! or rejected candidate is rolled back first and then logged with
//! [`Self::record_failure`], which never touches catalog rows.

use nova_core::catalog::publish::{CandidateReport, KindCounts};
use nova_core::now_ts;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationMode {
    Append,
    Replace,
}

impl PublicationMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Replace => "replace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureStatus {
    /// Validation refused the candidate (collapse, integrity).
    Rejected,
    /// The candidate could not be built (endpoint, parse, storage error).
    Failed,
}

/// A candidate that was rolled back. `reason_code` and `reason` must be
/// secret-free (no URLs, credentials or provider bodies); `counts` are the
/// entries received, per kind.
#[derive(Debug, Clone, Copy)]
pub struct FailedAttempt<'r> {
    pub source_id: i64,
    pub mode: PublicationMode,
    pub status: FailureStatus,
    pub reason_code: &'r str,
    pub reason: &'r str,
    pub counts: KindCounts,
    pub started_at: i64,
}

/// One recorded import attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub id: i64,
    pub source_id: i64,
    pub mode: String,
    pub status: String,
    pub reason_code: Option<String>,
    pub reason: Option<String>,
    pub counts: KindCounts,
    pub started_at: i64,
    pub finished_at: i64,
}

fn to_u64(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn to_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

pub struct PublicationRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> PublicationRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Breadth of the source's catalog as seen by this transaction: live and
    /// movie rows, and distinct series (episode rows fetched lazily after an
    /// import don't count as catalog breadth).
    pub async fn measure_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<KindCounts, StorageError> {
        let row = sqlx::query!(
            r#"SELECT
                 (SELECT COUNT(*) FROM channels WHERE source_id = ?) as "live!: i64",
                 (SELECT COUNT(*) FROM vod_items WHERE source_id = ?) as "movie!: i64",
                 (SELECT COUNT(DISTINCT series_title) FROM series_episodes
                  WHERE source_id = ?) as "series!: i64""#,
            source_id,
            source_id,
            source_id,
        )
        .fetch_one(&mut **tx)
        .await?;
        Ok(KindCounts {
            live: to_u64(row.live),
            movie: to_u64(row.movie),
            series: to_u64(row.series),
        })
    }

    /// Everything integrity validation needs about the compiled candidate.
    /// Series are counted as rows here, to compare with rows received.
    pub async fn candidate_report_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<CandidateReport, StorageError> {
        let row = sqlx::query!(
            r#"SELECT
                 (SELECT COUNT(*) FROM channels WHERE source_id = ?) as "live!: i64",
                 (SELECT COUNT(*) FROM vod_items WHERE source_id = ?) as "movie!: i64",
                 (SELECT COUNT(*) FROM series_episodes WHERE source_id = ?) as "series!: i64",
                 (SELECT COUNT(*) FROM vod_items v
                  WHERE v.source_id = ? AND NOT EXISTS (
                      SELECT 1 FROM stream_variants s
                      WHERE s.source_id = v.source_id AND s.binding_key = v.binding_key))
                     as "unbound!: i64",
                 (SELECT COUNT(*) FROM stream_variants s
                  JOIN canonical_entities e ON e.id = s.entity_id
                  WHERE e.state <> 'active') as "on_merged!: i64",
                 (SELECT COUNT(*) FROM canonical_entities e
                  WHERE e.state = 'merged' AND NOT EXISTS (
                      SELECT 1 FROM entity_redirects r
                      JOIN canonical_entities t ON t.id = r.new_id AND t.state = 'active'
                      WHERE r.old_id = e.id)) as "dangling!: i64""#,
            source_id,
            source_id,
            source_id,
            source_id,
        )
        .fetch_one(&mut **tx)
        .await?;
        Ok(CandidateReport {
            rows: KindCounts {
                live: to_u64(row.live),
                movie: to_u64(row.movie),
                series: to_u64(row.series),
            },
            unbound_movie_rows: to_u64(row.unbound),
            variants_on_merged_entities: to_u64(row.on_merged),
            dangling_merged_entities: to_u64(row.dangling),
        })
    }

    /// Mark the candidate active. Must run inside the import transaction so
    /// the record commits (or rolls back) together with the catalog.
    pub async fn record_published_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
        mode: PublicationMode,
        counts: KindCounts,
        started_at: i64,
    ) -> Result<i64, StorageError> {
        let mode = mode.as_str();
        let (live, movie, series) = (
            to_i64(counts.live),
            to_i64(counts.movie),
            to_i64(counts.series),
        );
        let finished_at = now_ts();
        let id = sqlx::query!(
            r#"INSERT INTO catalog_publications
                   (source_id, mode, status, live_count, movie_count, series_count,
                    started_at, finished_at)
               VALUES (?, ?, 'published', ?, ?, ?, ?, ?)"#,
            source_id,
            mode,
            live,
            movie,
            series,
            started_at,
            finished_at,
        )
        .execute(&mut **tx)
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    /// Log a failed or rejected attempt, after its transaction rolled back.
    /// A source deleted meanwhile is not an error: there is nothing to log.
    pub async fn record_failure(&self, attempt: &FailedAttempt<'_>) -> Result<(), StorageError> {
        let mode = attempt.mode.as_str();
        let status = match attempt.status {
            FailureStatus::Rejected => "rejected",
            FailureStatus::Failed => "failed",
        };
        let counts = attempt.counts;
        let (live, movie, series) = (
            to_i64(counts.live),
            to_i64(counts.movie),
            to_i64(counts.series),
        );
        let finished_at = now_ts();
        sqlx::query!(
            r#"INSERT INTO catalog_publications
                   (source_id, mode, status, reason_code, reason, live_count, movie_count,
                    series_count, started_at, finished_at)
               SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
               WHERE EXISTS (SELECT 1 FROM sources WHERE id = ?)"#,
            attempt.source_id,
            mode,
            status,
            attempt.reason_code,
            attempt.reason,
            live,
            movie,
            series,
            attempt.started_at,
            finished_at,
            attempt.source_id,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// The source's active generation: its latest published attempt.
    pub async fn active(&self, source_id: i64) -> Result<Option<Publication>, StorageError> {
        let row = sqlx::query_as!(
            PublicationRow,
            r#"SELECT id as "id!: i64", source_id as "source_id!: i64", mode as "mode!: String",
                      status as "status!: String", reason_code, reason,
                      live_count as "live_count!: i64", movie_count as "movie_count!: i64",
                      series_count as "series_count!: i64",
                      started_at as "started_at!: i64", finished_at as "finished_at!: i64"
               FROM catalog_publications
               WHERE source_id = ? AND status = 'published'
               ORDER BY id DESC LIMIT 1"#,
            source_id,
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Publication::from))
    }

    /// Recent attempts for the source, newest first.
    pub async fn history(
        &self,
        source_id: i64,
        limit: i64,
    ) -> Result<Vec<Publication>, StorageError> {
        let rows = sqlx::query_as!(
            PublicationRow,
            r#"SELECT id as "id!: i64", source_id as "source_id!: i64", mode as "mode!: String",
                      status as "status!: String", reason_code, reason,
                      live_count as "live_count!: i64", movie_count as "movie_count!: i64",
                      series_count as "series_count!: i64",
                      started_at as "started_at!: i64", finished_at as "finished_at!: i64"
               FROM catalog_publications
               WHERE source_id = ?
               ORDER BY id DESC LIMIT ?"#,
            source_id,
            limit,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Publication::from).collect())
    }
}

struct PublicationRow {
    id: i64,
    source_id: i64,
    mode: String,
    status: String,
    reason_code: Option<String>,
    reason: Option<String>,
    live_count: i64,
    movie_count: i64,
    series_count: i64,
    started_at: i64,
    finished_at: i64,
}

impl From<PublicationRow> for Publication {
    fn from(r: PublicationRow) -> Self {
        Self {
            id: r.id,
            source_id: r.source_id,
            mode: r.mode,
            status: r.status,
            reason_code: r.reason_code,
            reason: r.reason,
            counts: KindCounts {
                live: to_u64(r.live_count),
                movie: to_u64(r.movie_count),
                series: to_u64(r.series_count),
            },
            started_at: r.started_at,
            finished_at: r.finished_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{init_profile_db, CanonicalRepository, SourceRepository, VodRepository};
    use nova_core::catalog::publish::{check_integrity, IntegrityViolation, PublishRejection};
    use nova_core::VodItem;

    fn movie(source_id: i64, n: u32) -> VodItem {
        VodItem {
            id: 0,
            source_id,
            name: format!("Film {n}"),
            clean_title: Some(format!("Film {n}")),
            year: None,
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
        }
    }

    async fn setup() -> (tempfile::TempDir, SqlitePool, i64) {
        let dir = tempfile::tempdir().unwrap();
        let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
            .await
            .unwrap();
        let id = SourceRepository::new(&pool)
            .create("xtream", "t", "http://host/get.php")
            .await
            .unwrap()
            .id;
        (dir, pool, id)
    }

    #[tokio::test]
    async fn the_candidate_report_catches_uncompiled_movie_rows() {
        let (_dir, pool, id) = setup().await;
        let mut tx = pool.begin().await.unwrap();
        VodRepository::insert_batch_tx(&mut tx, &[movie(id, 1), movie(id, 2)])
            .await
            .unwrap();
        let received = KindCounts {
            movie: 2,
            ..KindCounts::default()
        };
        // Inserted but never compiled: both rows are unbound.
        let report = PublicationRepository::candidate_report_tx(&mut tx, id)
            .await
            .unwrap();
        assert_eq!(report.unbound_movie_rows, 2);
        assert_eq!(
            check_integrity(received, &report),
            Err(PublishRejection::Integrity(
                IntegrityViolation::UnboundMovieRows(2)
            ))
        );
        CanonicalRepository::compile_movies_tx(&mut tx, id)
            .await
            .unwrap();
        let report = PublicationRepository::candidate_report_tx(&mut tx, id)
            .await
            .unwrap();
        assert_eq!(check_integrity(received, &report), Ok(()));
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn the_candidate_report_catches_variants_on_merged_entities() {
        let (_dir, pool, id) = setup().await;
        let mut tx = pool.begin().await.unwrap();
        VodRepository::insert_batch_tx(&mut tx, &[movie(id, 1)])
            .await
            .unwrap();
        CanonicalRepository::compile_movies_tx(&mut tx, id)
            .await
            .unwrap();
        // Corrupt: tombstone the entity without moving its variant.
        sqlx::query("UPDATE canonical_entities SET state = 'merged'")
            .execute(&mut *tx)
            .await
            .unwrap();
        let report = PublicationRepository::candidate_report_tx(&mut tx, id)
            .await
            .unwrap();
        assert_eq!(report.variants_on_merged_entities, 1);
        assert_eq!(report.dangling_merged_entities, 1);
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn failures_never_become_active_and_deleted_sources_are_skipped() {
        let (_dir, pool, id) = setup().await;
        let repo = PublicationRepository::new(&pool);
        let mut tx = pool.begin().await.unwrap();
        let counts = KindCounts {
            live: 1,
            movie: 2,
            series: 3,
        };
        let published = PublicationRepository::record_published_tx(
            &mut tx,
            id,
            PublicationMode::Append,
            counts,
            1,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let attempt = FailedAttempt {
            source_id: id,
            mode: PublicationMode::Replace,
            status: FailureStatus::Rejected,
            reason_code: "collapse",
            reason: "provider returned 0 movies instead of about 2",
            counts: KindCounts::default(),
            started_at: 2,
        };
        repo.record_failure(&attempt).await.unwrap();
        let active = repo.active(id).await.unwrap().unwrap();
        assert_eq!((active.id, active.counts), (published, counts));
        assert_eq!(repo.history(id, 10).await.unwrap()[0].status, "rejected");

        // A published row cannot carry a reason, nor a failure lack one.
        let bad = sqlx::query(
            "INSERT INTO catalog_publications (source_id, mode, status, reason_code, started_at, finished_at)
             VALUES (?, 'append', 'published', 'x', 0, 0)",
        )
        .bind(id)
        .execute(&pool)
        .await;
        assert!(bad.is_err());

        // The source is deleted while its import fails: nothing to record.
        SourceRepository::new(&pool).delete(id).await.unwrap();
        repo.record_failure(&attempt).await.unwrap();
        assert!(repo.history(id, 10).await.unwrap().is_empty());
    }
}
