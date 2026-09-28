//! Canonical movies: the catalog compiler boundary plus canonical reads and
//! user-state actions.
//!
//! `vod_items` rows are source records and are replaced on refresh. Each row
//! is bound to a [`StreamVariant`](nova_core::catalog::StreamVariant) by its
//! source-scoped, secret-free binding key; variants belong to a canonical
//! entity, which owns favorites, watchlist and resume/completion. Refresh
//! unbinds variants (`ON DELETE SET NULL`) and [`compile_movies_tx`] re-binds
//! them inside the same import transaction, so identity and user state never
//! depend on a volatile row ID.
//!
//! Legacy per-row columns and `watch_history` rows are maintained as a
//! compatibility mirror for surfaces not yet migrated (search, rails that read
//! `VodItem`). The entity is the source of truth.
//!
//! [`compile_movies_tx`]: CanonicalRepository::compile_movies_tx

use nova_core::catalog::media::{
    binding_key, locator_binding_key, CanonicalMediaDetail, CanonicalMediaSummary,
    MoviePlaybackTarget, ResumeState, VariantSummary,
};
use nova_core::catalog::{ContentKind, EntityId, VariantId};
use nova_core::{now_ts, ContentSort};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::StorageError;

mod identity;
mod series_identity;

pub use identity::{LineageEntry, ResolutionReport, MAX_REDIRECT_HOPS};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CompileStats {
    /// Existing variants re-bound to a refreshed source record.
    pub rebound: u64,
    /// Locator-keyed variants upgraded to a stable upstream binding key.
    pub upgraded: u64,
    /// New entities (each with its first variant).
    pub created: u64,
}

/// Canonical movie browse filters (mirrors the legacy `ContentFilters`).
#[derive(Debug, Default, Clone)]
pub struct MovieFilters<'a> {
    pub category: Option<&'a str>,
    pub query: Option<&'a str>,
    pub favorites_only: bool,
    pub watchlist_only: bool,
}

pub struct CanonicalRepository<'a> {
    pool: &'a SqlitePool,
}

fn new_entity_id() -> EntityId {
    EntityId::new(format!("mov-{}", uuid::Uuid::new_v4().simple()))
        .expect("generated entity id is valid")
}

fn new_variant_id() -> VariantId {
    VariantId::new(format!("var-{}", uuid::Uuid::new_v4().simple()))
        .expect("generated variant id is valid")
}

pub(crate) fn parse_entity(id: String) -> Result<EntityId, StorageError> {
    EntityId::new(id).map_err(|_| StorageError::InvalidCanonicalRecord)
}

fn parse_variant(id: String) -> Result<VariantId, StorageError> {
    VariantId::new(id).map_err(|_| StorageError::InvalidCanonicalRecord)
}

fn like_query(query: Option<&str>) -> Option<String> {
    query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(|q| format!("%{q}%"))
}

impl<'a> CanonicalRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    // ── Compiler ────────────────────────────────────────────────────────────

    /// Bind every movie source record of `source_id` to a stream variant,
    /// creating entities for records never seen before. Idempotent; runs in
    /// the caller's import transaction after rows (and any legacy restore)
    /// are written, so a failed import rolls the whole generation back.
    ///
    /// Only records sharing a binding key within one source are treated as
    /// the same variant. Consolidating distinct entities is entity resolution
    /// ([`Self::resolve_movies_tx`]), which the import runs right after.
    pub async fn compile_movies_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<CompileStats, StorageError> {
        // 1. Re-bind unbound variants to the refreshed record with their key.
        //    Load-bearing invariant: UNIQUE(source_id, binding_key) gives each
        //    variant a disjoint candidate set, so this self-referencing UPDATE
        //    cannot bind one record to two variants within the statement.
        let rebound = sqlx::query!(
            r#"UPDATE stream_variants
               SET vod_item_id = (
                   SELECT MIN(v.id) FROM vod_items v
                   WHERE v.source_id = stream_variants.source_id
                     AND v.binding_key = stream_variants.binding_key
                     AND NOT EXISTS (SELECT 1 FROM stream_variants b WHERE b.vod_item_id = v.id)
               )
               WHERE source_id = ? AND vod_item_id IS NULL
                 AND EXISTS (
                   SELECT 1 FROM vod_items v
                   WHERE v.source_id = stream_variants.source_id
                     AND v.binding_key = stream_variants.binding_key
                     AND NOT EXISTS (SELECT 1 FROM stream_variants b WHERE b.vod_item_id = v.id)
                 )"#,
            source_id,
        )
        .execute(&mut **tx)
        .await?
        .rows_affected();
        let mut stats = CompileStats {
            rebound,
            ..CompileStats::default()
        };

        // 2. Upgrade variants compiled from a locator (before a stable ID was
        //    known) to the record's stable key, unless that key is taken.
        let upgrades = sqlx::query!(
            r#"SELECT s.id as "variant_id!: String",
                      MIN(v.id) as "vod_item_id!: i64",
                      v.binding_key as "binding_key!: String"
               FROM stream_variants s
               JOIN vod_items v
                 ON v.source_id = s.source_id
                AND v.locator_key = s.binding_key
                AND v.binding_key <> v.locator_key
               WHERE s.source_id = ? AND s.vod_item_id IS NULL
                 AND NOT EXISTS (SELECT 1 FROM stream_variants b WHERE b.vod_item_id = v.id)
                 AND NOT EXISTS (
                   SELECT 1 FROM stream_variants k
                   WHERE k.source_id = s.source_id AND k.binding_key = v.binding_key
                 )
               GROUP BY s.id"#,
            source_id,
        )
        .fetch_all(&mut **tx)
        .await?;
        for row in upgrades {
            let claimed = sqlx::query_scalar!(
                r#"SELECT COUNT(*) as "n!: i64" FROM stream_variants
                   WHERE (source_id = ? AND binding_key = ?) OR vod_item_id = ?"#,
                source_id,
                row.binding_key,
                row.vod_item_id,
            )
            .fetch_one(&mut **tx)
            .await?;
            if claimed > 0 {
                continue;
            }
            sqlx::query!(
                "UPDATE stream_variants SET binding_key = ?, vod_item_id = ? WHERE id = ?",
                row.binding_key,
                row.vod_item_id,
                row.variant_id,
            )
            .execute(&mut **tx)
            .await?;
            stats.upgraded += 1;
        }

        // 3. New records: one entity + variant per unclaimed binding key.
        //    Legacy per-row flags seed the entity (upgrade backfill and the
        //    legacy refresh restore); afterwards the entity is authoritative.
        let fresh = sqlx::query!(
            r#"SELECT MIN(v.id) as "vod_item_id!: i64",
                      v.binding_key as "binding_key!: String"
               FROM vod_items v
               WHERE v.source_id = ? AND v.binding_key IS NOT NULL
                 AND NOT EXISTS (
                   SELECT 1 FROM stream_variants s
                   WHERE s.source_id = v.source_id AND s.binding_key = v.binding_key
                 )
               GROUP BY v.binding_key"#,
            source_id,
        )
        .fetch_all(&mut **tx)
        .await?;
        let now = now_ts();
        for row in fresh {
            let entity_id = new_entity_id();
            let entity = entity_id.as_str();
            let variant_id = new_variant_id();
            let variant = variant_id.as_str();
            sqlx::query!(
                r#"INSERT INTO canonical_entities
                       (id, kind, title, year, is_favorite, is_in_watchlist, created_at)
                   SELECT ?, 'movie', COALESCE(clean_title, name), year,
                          is_favorite, is_in_watchlist, ?
                   FROM vod_items WHERE id = ?"#,
                entity,
                now,
                row.vod_item_id,
            )
            .execute(&mut **tx)
            .await?;
            sqlx::query!(
                r#"INSERT INTO stream_variants
                       (id, entity_id, source_id, binding_key, vod_item_id, created_at)
                   VALUES (?, ?, ?, ?, ?, ?)"#,
                variant,
                entity,
                source_id,
                row.binding_key,
                row.vod_item_id,
                now,
            )
            .execute(&mut **tx)
            .await?;
            stats.created += 1;
        }

        // 4. Seed resume state from the legacy mirror only where the entity
        //    has none (upgrade backfill); entity state always wins. History of
        //    every record sharing the key counts, most recent inserted first
        //    (so it is the one kept).
        sqlx::query!(
            r#"INSERT INTO entity_watch_state
                   (entity_id, position_sec, duration_sec, watched_at, completed, caption_mode)
               SELECT s.entity_id, wh.position_sec, wh.duration_sec, wh.watched_at,
                      wh.completed, wh.caption_mode
               FROM stream_variants s
               JOIN vod_items v
                 ON v.source_id = s.source_id AND v.binding_key = s.binding_key
               JOIN watch_history wh ON wh.media_kind = 'vod' AND wh.media_id = v.id
               WHERE s.source_id = ?
               ORDER BY wh.watched_at DESC
               ON CONFLICT(entity_id) DO NOTHING"#,
            source_id,
        )
        .execute(&mut **tx)
        .await?;

        // 5. Refresh the entity's parsed title/year from *all* its bound
        //    records, deterministically: the same choice whichever source was
        //    refreshed last (it used to be last-writer-wins). A mixed-case
        //    title is preferred over an ALL-CAPS copy, then the first in
        //    order. Then re-derive the legacy flag mirror from the entity.
        sqlx::query!(
            r#"UPDATE canonical_entities
               SET title = (SELECT COALESCE(v.clean_title, v.name)
                            FROM stream_variants s JOIN vod_items v ON v.id = s.vod_item_id
                            WHERE s.entity_id = canonical_entities.id
                            ORDER BY COALESCE(v.clean_title, v.name)
                                         = UPPER(COALESCE(v.clean_title, v.name)),
                                     COALESCE(v.clean_title, v.name)
                            LIMIT 1),
                   year  = (SELECT MIN(v.year)
                            FROM stream_variants s JOIN vod_items v ON v.id = s.vod_item_id
                            WHERE s.entity_id = canonical_entities.id)
               WHERE id IN (SELECT entity_id FROM stream_variants
                            WHERE source_id = ? AND vod_item_id IS NOT NULL)"#,
            source_id,
        )
        .execute(&mut **tx)
        .await?;
        Self::mirror_flags_for_source_tx(tx, source_id).await?;
        Ok(stats)
    }

    /// Re-derive the legacy flag mirror from entities, for every record of
    /// the source that maps to a variant by key (including duplicate rows).
    async fn mirror_flags_for_source_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            r#"UPDATE vod_items
               SET is_favorite = COALESCE((
                       SELECT e.is_favorite FROM stream_variants s
                       JOIN canonical_entities e ON e.id = s.entity_id
                       WHERE s.source_id = vod_items.source_id
                         AND s.binding_key = vod_items.binding_key), 0),
                   is_in_watchlist = COALESCE((
                       SELECT e.is_in_watchlist FROM stream_variants s
                       JOIN canonical_entities e ON e.id = s.entity_id
                       WHERE s.source_id = vod_items.source_id
                         AND s.binding_key = vod_items.binding_key), 0)
               WHERE source_id = ?
                 AND EXISTS (
                   SELECT 1 FROM stream_variants s
                   WHERE s.source_id = vod_items.source_id
                     AND s.binding_key = vod_items.binding_key)"#,
            source_id,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Upgrade step run after SQL migrations. Sources with rows written
    /// before migration 0021 (no binding key) are keyed and compiled in one
    /// transaction each, so an interrupted upgrade leaves no half-keyed
    /// source; legacy flags and resume state seed the new entities. Imports
    /// key rows as they insert them, so steady-state opens only probe the
    /// empty partial index `idx_vod_unkeyed`.
    pub async fn backfill_movies(&self) -> Result<CompileStats, StorageError> {
        let pending = sqlx::query_scalar!(
            r#"SELECT DISTINCT source_id as "source_id!: i64"
               FROM vod_items WHERE binding_key IS NULL"#,
        )
        .fetch_all(self.pool)
        .await?;
        let mut total = CompileStats::default();
        for source_id in pending {
            // Take the write lock up front: a concurrent opener of the same
            // profile waits (busy timeout) instead of failing a deferred
            // read→write upgrade, then finds the source already keyed.
            let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
            let rows = sqlx::query!(
                r#"SELECT id as "id!: i64", stream_url as "stream_url!: String"
                   FROM vod_items WHERE source_id = ? AND binding_key IS NULL"#,
                source_id,
            )
            .fetch_all(&mut *tx)
            .await?;
            for row in rows {
                // Pre-0021 rows carry no stable upstream ID.
                let key = binding_key(None, &row.stream_url);
                let locator = locator_binding_key(&row.stream_url);
                sqlx::query!(
                    "UPDATE vod_items SET binding_key = ?, locator_key = ? WHERE id = ?",
                    key,
                    locator,
                    row.id,
                )
                .execute(&mut *tx)
                .await?;
            }
            let stats = Self::compile_movies_tx(&mut tx, source_id).await?;
            tx.commit().await?;
            total.rebound += stats.rebound;
            total.upgraded += stats.upgraded;
            total.created += stats.created;
        }
        Ok(total)
    }

    // ── Canonical reads ─────────────────────────────────────────────────────

    /// One card per canonical movie with an available variant in `source_id`,
    /// shown through its lowest bound row (a merged entity can hold several
    /// variants of one source). Filters apply to that representative row.
    pub async fn list_movies(
        &self,
        source_id: i64,
        limit: i64,
        offset: i64,
        filters: &MovieFilters<'_>,
        sort: ContentSort,
    ) -> Result<Vec<CanonicalMediaSummary>, StorageError> {
        let search = like_query(filters.query);
        let search = search.as_deref();
        let category = filters.category;
        let fav: i64 = filters.favorites_only.into();
        let wl: i64 = filters.watchlist_only.into();
        let recent: i64 = matches!(sort, ContentSort::RecentlyAdded).into();
        let rows = sqlx::query!(
            r#"SELECT e.id as "entity_id!: String",
                      COALESCE(v.canonical_title, e.title, v.name) as "title!: String",
                      v.name as "source_title!: String",
                      v.year as "year?: i64",
                      v.category as "category?: String",
                      COALESCE(v.poster_url, v.logo_url) as "poster_url?: String",
                      v.backdrop_url as "backdrop_url?: String",
                      v.overview as "overview?: String",
                      e.is_favorite as "is_favorite!: i64",
                      e.is_in_watchlist as "is_in_watchlist!: i64",
                      v.added_at as "added_at?: i64",
                      v.dedup_group as "dedup_group?: String"
               FROM stream_variants s
               JOIN canonical_entities e ON e.id = s.entity_id
               JOIN vod_items v ON v.id = s.vod_item_id
               WHERE s.source_id = ?
                 AND s.vod_item_id = (
                   SELECT MIN(r.vod_item_id) FROM stream_variants r
                   WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id)
                 AND (? IS NULL OR v.category = ?)
                 AND (? IS NULL OR v.name LIKE ? OR v.clean_title LIKE ?
                      OR v.canonical_title LIKE ? OR v.category LIKE ?)
                 AND (? = 0 OR e.is_favorite = 1)
                 AND (? = 0 OR e.is_in_watchlist = 1)
               ORDER BY CASE WHEN ? = 1 THEN v.added_at END DESC,
                        CASE WHEN ? = 1 THEN v.id END DESC,
                        v.id ASC
               LIMIT ? OFFSET ?"#,
            source_id,
            category,
            category,
            search,
            search,
            search,
            search,
            search,
            fav,
            wl,
            recent,
            recent,
            limit,
            offset,
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok(CanonicalMediaSummary {
                    entity_id: parse_entity(r.entity_id)?,
                    kind: ContentKind::Movie,
                    title: r.title,
                    source_title: r.source_title,
                    year: r.year,
                    category: r.category,
                    poster_url: r.poster_url,
                    backdrop_url: r.backdrop_url,
                    overview: r.overview,
                    is_favorite: r.is_favorite != 0,
                    is_in_watchlist: r.is_in_watchlist != 0,
                    added_at: r.added_at,
                    dedup_group: r.dedup_group,
                })
            })
            .collect()
    }

    /// Cards for specific entities in `source_id`, in the order given
    /// (search results). Entities without an available variant there are
    /// skipped.
    pub async fn movies_by_ids(
        &self,
        source_id: i64,
        ids: &[EntityId],
    ) -> Result<Vec<CanonicalMediaSummary>, StorageError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let json = serde_json::to_string(&ids.iter().map(EntityId::as_str).collect::<Vec<_>>())?;
        let rows = sqlx::query!(
            r#"SELECT e.id as "entity_id!: String",
                      COALESCE(v.canonical_title, e.title, v.name) as "title!: String",
                      v.name as "source_title!: String",
                      v.year as "year?: i64",
                      v.category as "category?: String",
                      COALESCE(v.poster_url, v.logo_url) as "poster_url?: String",
                      v.backdrop_url as "backdrop_url?: String",
                      v.overview as "overview?: String",
                      e.is_favorite as "is_favorite!: i64",
                      e.is_in_watchlist as "is_in_watchlist!: i64",
                      v.added_at as "added_at?: i64",
                      v.dedup_group as "dedup_group?: String"
               FROM stream_variants s
               JOIN canonical_entities e ON e.id = s.entity_id
               JOIN vod_items v ON v.id = s.vod_item_id
               WHERE s.source_id = ?
                 AND s.entity_id IN (SELECT value FROM json_each(?))
                 AND s.vod_item_id = (
                   SELECT MIN(r.vod_item_id) FROM stream_variants r
                   WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id)"#,
            source_id,
            json,
        )
        .fetch_all(self.pool)
        .await?;
        let mut by_id: std::collections::HashMap<String, CanonicalMediaSummary> =
            std::collections::HashMap::new();
        for r in rows {
            let entity_id = parse_entity(r.entity_id.clone())?;
            by_id.insert(
                r.entity_id,
                CanonicalMediaSummary {
                    entity_id,
                    kind: ContentKind::Movie,
                    title: r.title,
                    source_title: r.source_title,
                    year: r.year,
                    category: r.category,
                    poster_url: r.poster_url,
                    backdrop_url: r.backdrop_url,
                    overview: r.overview,
                    is_favorite: r.is_favorite != 0,
                    is_in_watchlist: r.is_in_watchlist != 0,
                    added_at: r.added_at,
                    dedup_group: r.dedup_group,
                },
            );
        }
        Ok(ids
            .iter()
            .filter_map(|id| by_id.remove(id.as_str()))
            .collect())
    }

    pub async fn count_movies(
        &self,
        source_id: i64,
        filters: &MovieFilters<'_>,
    ) -> Result<i64, StorageError> {
        let search = like_query(filters.query);
        let search = search.as_deref();
        let category = filters.category;
        let fav: i64 = filters.favorites_only.into();
        let wl: i64 = filters.watchlist_only.into();
        let n = sqlx::query_scalar!(
            r#"SELECT COUNT(*) as "n!: i64"
               FROM stream_variants s
               JOIN canonical_entities e ON e.id = s.entity_id
               JOIN vod_items v ON v.id = s.vod_item_id
               WHERE s.source_id = ?
                 AND s.vod_item_id = (
                   SELECT MIN(r.vod_item_id) FROM stream_variants r
                   WHERE r.entity_id = s.entity_id AND r.source_id = s.source_id)
                 AND (? IS NULL OR v.category = ?)
                 AND (? IS NULL OR v.name LIKE ? OR v.clean_title LIKE ?
                      OR v.canonical_title LIKE ? OR v.category LIKE ?)
                 AND (? = 0 OR e.is_favorite = 1)
                 AND (? = 0 OR e.is_in_watchlist = 1)"#,
            source_id,
            category,
            category,
            search,
            search,
            search,
            search,
            search,
            fav,
            wl,
        )
        .fetch_one(self.pool)
        .await?;
        Ok(n)
    }

    /// Detail for one entity: display fields from its first available
    /// variant (or the retained entity title when none is available), every
    /// variant, and resume state.
    /// A historical (merged) ID returns the surviving entity's detail, with
    /// the survivor's ID so the caller can update its reference.
    pub async fn movie_detail(
        &self,
        entity_id: &EntityId,
    ) -> Result<Option<CanonicalMediaDetail>, StorageError> {
        let Some(entity_id) = self.canonical_entity_id(entity_id).await? else {
            return Ok(None);
        };
        let entity_id = &entity_id;
        let id = entity_id.as_str();
        let Some(entity) = sqlx::query!(
            r#"SELECT e.title as "title!: String",
                      e.year as "year?: i64",
                      e.is_favorite as "is_favorite!: i64",
                      e.is_in_watchlist as "is_in_watchlist!: i64",
                      COALESCE(v.canonical_title, e.title) as "display_title?: String",
                      v.name as "source_title?: String",
                      v.year as "variant_year?: i64",
                      v.category as "category?: String",
                      COALESCE(v.poster_url, v.logo_url) as "poster_url?: String",
                      v.backdrop_url as "backdrop_url?: String",
                      v.overview as "overview?: String",
                      v.added_at as "added_at?: i64",
                      v.dedup_group as "dedup_group?: String"
               FROM canonical_entities e
               LEFT JOIN vod_items v ON v.id = (
                   SELECT MIN(s.vod_item_id) FROM stream_variants s
                   WHERE s.entity_id = e.id AND s.vod_item_id IS NOT NULL)
               WHERE e.id = ? AND e.kind = 'movie'"#,
            id,
        )
        .fetch_optional(self.pool)
        .await?
        else {
            return Ok(None);
        };
        let variants = sqlx::query!(
            r#"SELECT s.id as "variant_id!: String",
                      s.source_id as "source_id!: i64",
                      src.name as "source_name!: String",
                      v.name as "label?: String",
                      s.vod_item_id as "vod_item_id?: i64"
               FROM stream_variants s
               JOIN sources src ON src.id = s.source_id
               LEFT JOIN vod_items v ON v.id = s.vod_item_id
               WHERE s.entity_id = ?
               ORDER BY s.vod_item_id IS NULL, s.vod_item_id, s.id"#,
            id,
        )
        .fetch_all(self.pool)
        .await?
        .into_iter()
        .map(|r| {
            Ok(VariantSummary {
                variant_id: parse_variant(r.variant_id)?,
                source_id: r.source_id,
                source_name: r.source_name,
                label: r.label.unwrap_or_default(),
                available: r.vod_item_id.is_some(),
            })
        })
        .collect::<Result<Vec<_>, StorageError>>()?;
        let resume = self.resume(entity_id).await?;
        Ok(Some(CanonicalMediaDetail {
            summary: CanonicalMediaSummary {
                entity_id: entity_id.clone(),
                kind: ContentKind::Movie,
                title: entity.display_title.unwrap_or(entity.title.clone()),
                source_title: entity.source_title.unwrap_or(entity.title),
                year: entity.variant_year.or(entity.year),
                category: entity.category,
                poster_url: entity.poster_url,
                backdrop_url: entity.backdrop_url,
                overview: entity.overview,
                is_favorite: entity.is_favorite != 0,
                is_in_watchlist: entity.is_in_watchlist != 0,
                added_at: entity.added_at,
                dedup_group: entity.dedup_group,
            },
            variants,
            resume,
        }))
    }

    pub async fn resume(&self, entity_id: &EntityId) -> Result<Option<ResumeState>, StorageError> {
        let mut conn = self.pool.acquire().await?;
        let Some(entity_id) = Self::canonical_entity_conn(&mut conn, entity_id).await? else {
            return Ok(None);
        };
        Self::resume_conn(&mut conn, &entity_id).await
    }

    /// Rust-side variant selection: the requested variant when it belongs to
    /// the entity and is available, otherwise the first available variant.
    /// `None` when the entity has no playable variant.
    pub async fn resolve_playback(
        &self,
        entity_id: &EntityId,
        variant_id: Option<&VariantId>,
    ) -> Result<Option<MoviePlaybackTarget>, StorageError> {
        let Some(entity_id) = self.canonical_entity_id(entity_id).await? else {
            return Ok(None);
        };
        let entity_id = &entity_id;
        let id = entity_id.as_str();
        let requested = variant_id.map(VariantId::as_str);
        let row = sqlx::query!(
            r#"SELECT s.id as "variant_id!: String", s.vod_item_id as "vod_item_id!: i64"
               FROM stream_variants s
               WHERE s.entity_id = ? AND s.vod_item_id IS NOT NULL
               ORDER BY (s.id = ?) DESC, s.vod_item_id ASC
               LIMIT 1"#,
            id,
            requested,
        )
        .fetch_optional(self.pool)
        .await?;
        row.map(|r| {
            Ok(MoviePlaybackTarget {
                entity_id: entity_id.clone(),
                variant_id: parse_variant(r.variant_id)?,
                media_kind: "vod".into(),
                media_id: r.vod_item_id,
            })
        })
        .transpose()
    }

    /// The entity of a legacy movie row: through its bound variant, or the
    /// variant sharing its binding key (duplicate rows of one provider item).
    pub async fn entity_for_vod(&self, vod_item_id: i64) -> Result<Option<EntityId>, StorageError> {
        let mut conn = self.pool.acquire().await?;
        Self::entity_for_vod_conn(&mut conn, vod_item_id).await
    }

    pub(crate) async fn entity_for_vod_tx(
        tx: &mut Transaction<'_, Sqlite>,
        vod_item_id: i64,
    ) -> Result<Option<EntityId>, StorageError> {
        Self::entity_for_vod_conn(tx, vod_item_id).await
    }

    pub(crate) async fn entity_for_vod_conn(
        conn: &mut sqlx::SqliteConnection,
        vod_item_id: i64,
    ) -> Result<Option<EntityId>, StorageError> {
        // Two indexed lookups (bound row first); called on every progress tick.
        let id = sqlx::query_scalar!(
            r#"SELECT COALESCE(
                   (SELECT entity_id FROM stream_variants WHERE vod_item_id = ?),
                   (SELECT s.entity_id FROM vod_items v
                    JOIN stream_variants s
                      ON s.source_id = v.source_id AND s.binding_key = v.binding_key
                    WHERE v.id = ?)
               ) as "entity_id?: String""#,
            vod_item_id,
            vod_item_id,
        )
        .fetch_one(&mut *conn)
        .await?;
        id.map(parse_entity).transpose()
    }

    // ── Canonical user-state actions ────────────────────────────────────────

    /// Returns false when the entity does not exist. A historical ID acts on
    /// the surviving entity.
    pub async fn set_favorite(
        &self,
        entity_id: &EntityId,
        is_favorite: bool,
    ) -> Result<bool, StorageError> {
        let flag: i64 = is_favorite.into();
        let mut tx = self.pool.begin().await?;
        let Some(entity_id) = Self::canonical_entity_conn(&mut tx, entity_id).await? else {
            return Ok(false);
        };
        let id = entity_id.as_str();
        let changed = sqlx::query!(
            "UPDATE canonical_entities SET is_favorite = ? WHERE id = ?",
            flag,
            id,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query!(
            r#"UPDATE vod_items SET is_favorite = ?
               WHERE id IN (
                   SELECT v.id FROM vod_items v
                   JOIN stream_variants s
                     ON s.source_id = v.source_id AND s.binding_key = v.binding_key
                   WHERE s.entity_id = ?)"#,
            flag,
            id,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(changed > 0)
    }

    /// Returns false when the entity does not exist. A historical ID acts on
    /// the surviving entity.
    pub async fn set_in_watchlist(
        &self,
        entity_id: &EntityId,
        in_watchlist: bool,
    ) -> Result<bool, StorageError> {
        let flag: i64 = in_watchlist.into();
        let mut tx = self.pool.begin().await?;
        let Some(entity_id) = Self::canonical_entity_conn(&mut tx, entity_id).await? else {
            return Ok(false);
        };
        let id = entity_id.as_str();
        let changed = sqlx::query!(
            "UPDATE canonical_entities SET is_in_watchlist = ? WHERE id = ?",
            flag,
            id,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query!(
            r#"UPDATE vod_items SET is_in_watchlist = ?
               WHERE id IN (
                   SELECT v.id FROM vod_items v
                   JOIN stream_variants s
                     ON s.source_id = v.source_id AND s.binding_key = v.binding_key
                   WHERE s.entity_id = ?)"#,
            flag,
            id,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(changed > 0)
    }

    /// Upsert entity-owned resume/completion. `caption_mode = None` keeps
    /// the stored preference (matches the legacy `record_progress` contract).
    pub async fn record_progress(
        &self,
        entity_id: &EntityId,
        position_sec: f64,
        duration_sec: Option<f64>,
        completed: bool,
        caption_mode: Option<&str>,
    ) -> Result<(), StorageError> {
        let mut conn = self.pool.acquire().await?;
        // A historical ID records onto its survivor; an unknown one is left
        // to the foreign key to reject, as before.
        let entity_id = Self::canonical_entity_conn(&mut conn, entity_id)
            .await?
            .unwrap_or_else(|| entity_id.clone());
        Self::record_progress_conn(
            &mut conn,
            &entity_id,
            position_sec,
            duration_sec,
            completed,
            caption_mode,
        )
        .await
    }

    pub(crate) async fn record_progress_conn(
        conn: &mut sqlx::SqliteConnection,
        entity_id: &EntityId,
        position_sec: f64,
        duration_sec: Option<f64>,
        completed: bool,
        caption_mode: Option<&str>,
    ) -> Result<(), StorageError> {
        let id = entity_id.as_str();
        let watched_at = now_ts();
        let completed: i64 = completed.into();
        sqlx::query!(
            r#"INSERT INTO entity_watch_state
                   (entity_id, position_sec, duration_sec, watched_at, completed, caption_mode)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT(entity_id) DO UPDATE SET
                   position_sec = excluded.position_sec,
                   duration_sec = COALESCE(excluded.duration_sec, entity_watch_state.duration_sec),
                   watched_at   = excluded.watched_at,
                   completed    = excluded.completed,
                   caption_mode = COALESCE(excluded.caption_mode, entity_watch_state.caption_mode)"#,
            id,
            position_sec,
            duration_sec,
            watched_at,
            completed,
            caption_mode,
        )
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    pub(crate) async fn resume_conn(
        conn: &mut sqlx::SqliteConnection,
        entity_id: &EntityId,
    ) -> Result<Option<ResumeState>, StorageError> {
        let id = entity_id.as_str();
        let row = sqlx::query!(
            r#"SELECT position_sec as "position_sec!: f64",
                      duration_sec as "duration_sec?: f64",
                      completed as "completed!: i64"
               FROM entity_watch_state WHERE entity_id = ?"#,
            id,
        )
        .fetch_optional(&mut *conn)
        .await?;
        Ok(row.map(|r| ResumeState {
            position_sec: r.position_sec,
            duration_sec: r.duration_sec,
            completed: r.completed != 0,
        }))
    }
}

#[cfg(test)]
mod tests;
