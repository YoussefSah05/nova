//! Canonical series, slice 1: series identity and series-level user state.
//!
//! `series_episodes` rows are source records, replaced on refresh (and, for
//! Xtream, replaced again when a series is first opened: the catalog sentinel
//! `xtream://series/<id>` gives way to the fetched episodes). Each series of a
//! source is bound through `series_bindings` to a canonical entity of kind
//! `series`, which owns favorite and watchlist. The binding key is the
//! provider series ID when the sentinel carries one, else the exact parsed
//! title ([`series_binding_key`]), so a provider rename of an Xtream series
//! keeps its entity and state even after the series was opened.
//!
//! Per-row `is_favorite` / `is_in_watchlist` stay as a compatibility mirror
//! re-derived from the entity, as for movies. Episodes, resume and TMDB
//! matches are not yet entity-owned.

use std::collections::{HashMap, HashSet};

use nova_core::catalog::media::{series_binding_key, xtream_series_id};
use nova_core::catalog::EntityId;
use nova_core::now_ts;
use sqlx::{Sqlite, Transaction};

use super::{CanonicalRepository, CompileStats};
use crate::error::StorageError;

fn new_series_entity_id() -> EntityId {
    EntityId::new(format!("ser-{}", uuid::Uuid::new_v4().simple()))
        .expect("generated entity id is valid")
}

impl CanonicalRepository<'_> {
    /// Bind every series of `source_id` to a canonical series entity,
    /// creating entities for series never seen before. Idempotent; runs in
    /// the caller's import transaction after rows (and the legacy flag
    /// restore) are written, so a failed import rolls the whole generation
    /// back. Every listed series ends up bound: when a provider ID is already
    /// taken by another title of the same source, that title binds by title.
    pub async fn compile_series_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<CompileStats, StorageError> {
        // 1. Unbind: this generation's titles are bound again below.
        sqlx::query!(
            "UPDATE series_bindings SET series_title = NULL WHERE source_id = ?",
            source_id,
        )
        .execute(&mut **tx)
        .await?;

        // 2. One group per listed series (title), with its sentinel if any.
        let groups = sqlx::query!(
            r#"SELECT series_title as "series_title!: String",
                      MAX(CASE WHEN stream_url LIKE 'xtream://series/%' THEN stream_url END)
                          as "sentinel?: String",
                      MAX(is_favorite) as "fav!: i64",
                      MAX(is_in_watchlist) as "wl!: i64"
               FROM series_episodes WHERE source_id = ?
               GROUP BY series_title ORDER BY series_title"#,
            source_id,
        )
        .fetch_all(&mut **tx)
        .await?;
        let mut bound: HashMap<String, String> = sqlx::query!(
            r#"SELECT binding_key as "binding_key!: String", entity_id as "entity_id!: String"
               FROM series_bindings WHERE source_id = ?"#,
            source_id,
        )
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|r| (r.binding_key, r.entity_id))
        .collect();

        let mut claimed: HashSet<String> = HashSet::new();
        let mut stats = CompileStats::default();
        let now = now_ts();
        for g in groups {
            let stable = g
                .sentinel
                .as_deref()
                .and_then(xtream_series_id)
                .map(|id| series_binding_key(Some(id), &g.series_title))
                .filter(|key| !claimed.contains(key));
            let by_title = series_binding_key(None, &g.series_title);

            let key = match stable {
                // 3a. Known provider ID: re-bind (possibly under a new title).
                Some(key) if bound.contains_key(&key) => {
                    stats.rebound += 1;
                    key
                }
                // 3b. Bound by title before the provider ID was known (an
                //     opened Xtream series at upgrade time): upgrade in place.
                Some(key) if bound.contains_key(&by_title) && !claimed.contains(&by_title) => {
                    sqlx::query!(
                        "UPDATE series_bindings SET binding_key = ? WHERE source_id = ? AND binding_key = ?",
                        key,
                        source_id,
                        by_title,
                    )
                    .execute(&mut **tx)
                    .await?;
                    let entity = bound.remove(&by_title).expect("checked above");
                    bound.insert(key.clone(), entity);
                    stats.upgraded += 1;
                    key
                }
                // 3c. Title-keyed series seen before (M3U).
                None if bound.contains_key(&by_title) && !claimed.contains(&by_title) => {
                    stats.rebound += 1;
                    by_title
                }
                // 3d. New series: its entity is seeded from the legacy flags
                //     (upgrade backfill and the refresh restore).
                new_key => {
                    let key = new_key.unwrap_or(by_title);
                    let entity_id = new_series_entity_id();
                    let entity = entity_id.as_str();
                    sqlx::query!(
                        r#"INSERT INTO canonical_entities
                               (id, kind, title, is_favorite, is_in_watchlist, created_at)
                           VALUES (?, 'series', ?, ?, ?, ?)"#,
                        entity,
                        g.series_title,
                        g.fav,
                        g.wl,
                        now,
                    )
                    .execute(&mut **tx)
                    .await?;
                    sqlx::query!(
                        r#"INSERT INTO series_bindings
                               (source_id, binding_key, entity_id, series_title, created_at)
                           VALUES (?, ?, ?, NULL, ?)"#,
                        source_id,
                        key,
                        entity,
                        now,
                    )
                    .execute(&mut **tx)
                    .await?;
                    bound.insert(key.clone(), entity.to_string());
                    stats.created += 1;
                    key
                }
            };
            sqlx::query!(
                "UPDATE series_bindings SET series_title = ? WHERE source_id = ? AND binding_key = ?",
                g.series_title,
                source_id,
                key,
            )
            .execute(&mut **tx)
            .await?;
            claimed.insert(key);
        }

        // 4. The entity follows its current parsed title, then the legacy
        //    per-row flags are re-derived from the entity.
        sqlx::query!(
            r#"UPDATE canonical_entities
               SET title = (SELECT b.series_title FROM series_bindings b
                            WHERE b.entity_id = canonical_entities.id
                              AND b.series_title IS NOT NULL
                            ORDER BY b.source_id, b.binding_key LIMIT 1)
               WHERE kind = 'series'
                 AND id IN (SELECT entity_id FROM series_bindings
                            WHERE source_id = ? AND series_title IS NOT NULL)"#,
            source_id,
        )
        .execute(&mut **tx)
        .await?;
        Self::mirror_series_flags_tx(tx, source_id).await?;
        Ok(stats)
    }

    /// Re-derive the legacy per-row flags of `source_id`'s bound series.
    pub(crate) async fn mirror_series_flags_tx(
        tx: &mut Transaction<'_, Sqlite>,
        source_id: i64,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            r#"UPDATE series_episodes
               SET is_favorite = (
                       SELECT e.is_favorite FROM series_bindings b
                       JOIN canonical_entities e ON e.id = b.entity_id
                       WHERE b.source_id = series_episodes.source_id
                         AND b.series_title = series_episodes.series_title),
                   is_in_watchlist = (
                       SELECT e.is_in_watchlist FROM series_bindings b
                       JOIN canonical_entities e ON e.id = b.entity_id
                       WHERE b.source_id = series_episodes.source_id
                         AND b.series_title = series_episodes.series_title)
               WHERE source_id = ?
                 AND EXISTS (
                   SELECT 1 FROM series_bindings b
                   WHERE b.source_id = series_episodes.source_id
                     AND b.series_title = series_episodes.series_title)"#,
            source_id,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Upgrade step run after SQL migrations: compile every source that
    /// lists a series with no binding (profiles that predate 0036). Each
    /// source compiles in one `BEGIN IMMEDIATE` transaction, so an
    /// interrupted upgrade leaves no half-bound source and a concurrent
    /// opener waits. Imports bind every series they write, so steady-state
    /// opens find nothing to do.
    pub async fn backfill_series(&self) -> Result<CompileStats, StorageError> {
        let pending = sqlx::query_scalar!(
            r#"SELECT DISTINCT se.source_id as "source_id!: i64"
               FROM series_episodes se
               WHERE NOT EXISTS (
                   SELECT 1 FROM series_bindings b
                   WHERE b.source_id = se.source_id AND b.series_title = se.series_title)"#,
        )
        .fetch_all(self.pool)
        .await?;
        let mut total = CompileStats::default();
        for source_id in pending {
            let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
            let stats = Self::compile_series_tx(&mut tx, source_id).await?;
            tx.commit().await?;
            total.rebound += stats.rebound;
            total.upgraded += stats.upgraded;
            total.created += stats.created;
        }
        Ok(total)
    }

    /// The canonical series entity a source's series is bound to.
    pub async fn series_entity(
        &self,
        source_id: i64,
        series_title: &str,
    ) -> Result<Option<EntityId>, StorageError> {
        let id = sqlx::query_scalar!(
            r#"SELECT entity_id as "entity_id!: String" FROM series_bindings
               WHERE source_id = ? AND series_title = ?"#,
            source_id,
            series_title,
        )
        .fetch_optional(self.pool)
        .await?;
        id.map(super::parse_entity).transpose()
    }
}
