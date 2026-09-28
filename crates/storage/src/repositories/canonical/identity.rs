//! Canonical identity: deterministic resolution, merge application, durable
//! redirects/aliases/lineage and historical-ID resolution.
//!
//! [`resolution::resolve`] decides; this module only applies what it decided.
//! It accepts a [`MergeDecision`], which only the deterministic resolver can
//! construct, so no model output or UI input can merge identities through it.
//!
//! A merge is atomic, inside the caller's transaction:
//! - every variant of the absorbed entity moves to the survivor;
//! - favorite/watchlist combine by logical OR;
//! - resume/completion: the most recently watched state wins, exactly as if
//!   both had been watched on one entity;
//! - the absorbed entity's title/year and aliases become survivor aliases, and
//!   its cannot-link decisions follow it;
//! - the absorbed entity is tombstoned (never deleted) and redirected, and
//!   older redirects are compressed onto the survivor;
//! - an append-only lineage row records the rule and both sides' prior state.

use std::collections::{BTreeMap, BTreeSet};

use nova_core::catalog::resolution::{
    self, CannotLink, EntityEvidence, MergeDecision, ResolutionStats, TrustedId, RULESET_VERSION,
};
use nova_core::catalog::{ContentKind, EntityId};
use nova_core::now_ts;
use sqlx::{Sqlite, SqliteConnection, Transaction};

use super::{parse_entity, CanonicalRepository};
use crate::error::StorageError;

/// Active-entity evidence, cannot-link decisions, and `(entity, trusted ID)`
/// aliases seen on bindings but not yet persisted.
type MovieEvidence = (
    Vec<EntityEvidence>,
    BTreeSet<CannotLink>,
    Vec<(String, String)>,
);

/// Upper bound on redirect hops when resolving a historical ID. Chains are
/// compressed on every merge, so a well-formed database needs one hop.
pub const MAX_REDIRECT_HOPS: usize = 16;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionReport {
    /// Merges actually applied (a decision can be skipped if the state it was
    /// computed from changed within the transaction).
    pub applied: u64,
    pub stats: ResolutionStats,
}

/// One append-only identity operation, for inspection and correction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageEntry {
    pub id: i64,
    pub operation: String,
    /// merge: survivor. cannot_link: the lower ID.
    pub entity_a: EntityId,
    /// merge: absorbed. cannot_link: the higher ID.
    pub entity_b: EntityId,
    pub actor: String,
    pub rule: Option<String>,
    pub ruleset_version: Option<i64>,
    pub evidence_json: String,
    pub prior_state_json: String,
    pub created_at: i64,
}

impl CanonicalRepository<'_> {
    // ── Resolution ──────────────────────────────────────────────────────────

    /// Resolve every active movie entity of the profile and apply the merges,
    /// in the caller's transaction (the import transaction, so a failed import
    /// never leaves a half-applied resolution).
    pub async fn resolve_movies_tx(
        tx: &mut Transaction<'_, Sqlite>,
    ) -> Result<ResolutionReport, StorageError> {
        let (evidence, cannot_link, new_external) = Self::load_movie_evidence(tx).await?;
        Self::persist_external_aliases(tx, &new_external).await?;
        let plan = resolution::resolve(&evidence, &cannot_link);
        let mut report = ResolutionReport {
            applied: 0,
            stats: plan.stats,
        };
        for decision in &plan.merges {
            if Self::apply_merge_tx(tx, decision).await? {
                report.applied += 1;
            }
        }
        let version = i64::from(RULESET_VERSION);
        let now = now_ts();
        sqlx::query!(
            r#"INSERT INTO entity_resolution_state (id, ruleset_version, resolved_at)
               VALUES (1, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   ruleset_version = excluded.ruleset_version,
                   resolved_at = excluded.resolved_at"#,
            version,
            now,
        )
        .execute(&mut **tx)
        .await?;
        Ok(report)
    }

    /// Open-time step: re-resolve once when this build's rule set is newer
    /// than the one that last resolved the profile (including profiles that
    /// predate resolution). A no-op otherwise.
    pub async fn ensure_movies_resolved(&self) -> Result<Option<ResolutionReport>, StorageError> {
        let current = i64::from(RULESET_VERSION);
        let stored = sqlx::query_scalar!(
            r#"SELECT ruleset_version as "v!: i64" FROM entity_resolution_state WHERE id = 1"#
        )
        .fetch_optional(self.pool)
        .await?;
        if stored.is_some_and(|v| v >= current) {
            return Ok(None);
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        // Re-check under the write lock: a concurrent opener may have won.
        let stored = sqlx::query_scalar!(
            r#"SELECT ruleset_version as "v!: i64" FROM entity_resolution_state WHERE id = 1"#
        )
        .fetch_optional(&mut *tx)
        .await?;
        if stored.is_some_and(|v| v >= current) {
            return Ok(None);
        }
        let report = Self::resolve_movies_tx(&mut tx).await?;
        tx.commit().await?;
        Ok(Some(report))
    }

    async fn load_movie_evidence(
        tx: &mut Transaction<'_, Sqlite>,
    ) -> Result<MovieEvidence, StorageError> {
        let mut by_id: BTreeMap<String, EntityEvidence> = BTreeMap::new();
        let entities = sqlx::query!(
            r#"SELECT id as "id!: String", created_at as "created_at!: i64",
                      title as "title!: String", year as "year?: i64"
               FROM canonical_entities
               WHERE kind = 'movie' AND state = 'active'"#
        )
        .fetch_all(&mut **tx)
        .await?;
        for row in entities {
            let mut evidence = EntityEvidence::new(
                parse_entity(row.id.clone())?,
                ContentKind::Movie,
                row.created_at,
            );
            evidence.titles.insert(row.title);
            evidence
                .years
                .extend(row.year.and_then(|y| i32::try_from(y).ok()));
            by_id.insert(row.id, evidence);
        }

        let bound = sqlx::query!(
            r#"SELECT s.entity_id as "entity_id!: String",
                      COALESCE(v.clean_title, v.name) as "title!: String",
                      v.year as "year?: i64",
                      v.provider_tmdb_id as "tmdb?: i64"
               FROM stream_variants s
               JOIN vod_items v ON v.id = s.vod_item_id
               JOIN canonical_entities e ON e.id = s.entity_id
               WHERE e.kind = 'movie' AND e.state = 'active'"#
        )
        .fetch_all(&mut **tx)
        .await?;
        let mut bound_ids: Vec<(String, TrustedId)> = Vec::new();
        for row in bound {
            let Some(evidence) = by_id.get_mut(&row.entity_id) else {
                continue;
            };
            evidence.titles.insert(row.title);
            evidence
                .years
                .extend(row.year.and_then(|y| i32::try_from(y).ok()));
            if let Some(trusted) = row.tmdb.and_then(TrustedId::tmdb_movie) {
                evidence.trusted_ids.insert(trusted.clone());
                bound_ids.push((row.entity_id, trusted));
            }
        }

        let aliases = sqlx::query!(
            r#"SELECT a.entity_id as "entity_id!: String",
                      a.alias_kind as "kind!: String",
                      a.value as "value!: String"
               FROM entity_aliases a
               JOIN canonical_entities e ON e.id = a.entity_id
               WHERE e.kind = 'movie' AND e.state = 'active'"#
        )
        .fetch_all(&mut **tx)
        .await?;
        let mut known_external: BTreeSet<(String, String)> = BTreeSet::new();
        for row in aliases {
            let Some(evidence) = by_id.get_mut(&row.entity_id) else {
                continue;
            };
            match row.kind.as_str() {
                "title" => {
                    evidence.titles.insert(row.value);
                }
                "year" => evidence.years.extend(row.value.parse::<i32>().ok()),
                "external" => {
                    evidence.trusted_ids.extend(TrustedId::decode(&row.value));
                    known_external.insert((row.entity_id, row.value));
                }
                _ => {}
            }
        }
        let mut new_external: Vec<(String, String)> = bound_ids
            .into_iter()
            .map(|(entity, trusted)| (entity, trusted.encoded()))
            .filter(|pair| !known_external.contains(pair))
            .collect();
        new_external.sort();
        new_external.dedup();

        let links = sqlx::query!(
            r#"SELECT entity_a as "a!: String", entity_b as "b!: String"
               FROM entity_cannot_link"#
        )
        .fetch_all(&mut **tx)
        .await?;
        let mut cannot_link = BTreeSet::new();
        for row in links {
            cannot_link.extend(CannotLink::new(parse_entity(row.a)?, parse_entity(row.b)?));
        }
        Ok((by_id.into_values().collect(), cannot_link, new_external))
    }

    /// Trusted IDs are durable identity evidence: keep them as aliases so they
    /// still count after the provider row that carried them disappears. Only
    /// new aliases are written, so steady-state imports write nothing.
    async fn persist_external_aliases(
        tx: &mut Transaction<'_, Sqlite>,
        new_external: &[(String, String)],
    ) -> Result<(), StorageError> {
        let now = now_ts();
        for (entity, value) in new_external {
            sqlx::query!(
                r#"INSERT OR IGNORE INTO entity_aliases (entity_id, alias_kind, value, created_at)
                   VALUES (?, 'external', ?, ?)"#,
                entity,
                value,
                now,
            )
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// Apply one resolver decision. Returns false (and changes nothing) when
    /// either side is no longer active or a cannot-link now separates them.
    async fn apply_merge_tx(
        tx: &mut Transaction<'_, Sqlite>,
        decision: &MergeDecision,
    ) -> Result<bool, StorageError> {
        let survivor = decision.survivor().as_str();
        let absorbed = decision.absorbed().as_str();
        let active = sqlx::query_scalar!(
            r#"SELECT COUNT(*) as "n!: i64" FROM canonical_entities
               WHERE id IN (?, ?) AND state = 'active' AND kind = 'movie'"#,
            survivor,
            absorbed,
        )
        .fetch_one(&mut **tx)
        .await?;
        let (lo, hi) = if survivor < absorbed {
            (survivor, absorbed)
        } else {
            (absorbed, survivor)
        };
        let separated = sqlx::query_scalar!(
            r#"SELECT COUNT(*) as "n!: i64" FROM entity_cannot_link
               WHERE entity_a = ? AND entity_b = ?"#,
            lo,
            hi,
        )
        .fetch_one(&mut **tx)
        .await?;
        if active != 2 || separated > 0 {
            return Ok(false);
        }

        let prior = serde_json::json!({
            "survivor": Self::state_snapshot(tx, survivor).await?,
            "absorbed": Self::state_snapshot(tx, absorbed).await?,
        })
        .to_string();
        let evidence = serde_json::json!({
            "rule": decision.rule().as_str(),
            "partner": decision.partner().as_str(),
            "rulesetVersion": decision.ruleset_version(),
        })
        .to_string();
        let rule = decision.rule().as_str();
        let version = i64::from(decision.ruleset_version());
        let now = now_ts();
        let operation = sqlx::query!(
            r#"INSERT INTO entity_lineage
                   (operation, entity_a, entity_b, actor, rule, ruleset_version,
                    evidence_json, prior_state_json, created_at)
               VALUES ('merge', ?, ?, 'resolver', ?, ?, ?, ?, ?)"#,
            survivor,
            absorbed,
            rule,
            version,
            evidence,
            prior,
            now,
        )
        .execute(&mut **tx)
        .await?
        .last_insert_rowid();

        sqlx::query!(
            "UPDATE stream_variants SET entity_id = ? WHERE entity_id = ?",
            survivor,
            absorbed,
        )
        .execute(&mut **tx)
        .await?;

        // The survivor now owns more records: re-derive its parsed title/year
        // with the compiler's deterministic rule (mixed case first, then
        // order), so the result never depends on which entity survived.
        sqlx::query!(
            r#"UPDATE canonical_entities
               SET title = COALESCE(
                       (SELECT COALESCE(v.clean_title, v.name)
                        FROM stream_variants s JOIN vod_items v ON v.id = s.vod_item_id
                        WHERE s.entity_id = canonical_entities.id
                        ORDER BY COALESCE(v.clean_title, v.name)
                                     = UPPER(COALESCE(v.clean_title, v.name)),
                                 COALESCE(v.clean_title, v.name)
                        LIMIT 1),
                       title),
                   year = COALESCE(
                       (SELECT MIN(v.year)
                        FROM stream_variants s JOIN vod_items v ON v.id = s.vod_item_id
                        WHERE s.entity_id = canonical_entities.id),
                       year)
               WHERE id = ?"#,
            survivor,
        )
        .execute(&mut **tx)
        .await?;

        sqlx::query!(
            r#"UPDATE canonical_entities
               SET is_favorite = MAX(is_favorite,
                       (SELECT is_favorite FROM canonical_entities WHERE id = ?)),
                   is_in_watchlist = MAX(is_in_watchlist,
                       (SELECT is_in_watchlist FROM canonical_entities WHERE id = ?))
               WHERE id = ?"#,
            absorbed,
            absorbed,
            survivor,
        )
        .execute(&mut **tx)
        .await?;

        // Latest watch wins, as if both had been watched on one entity. The
        // losing side is preserved in the lineage row's prior state.
        sqlx::query!(
            r#"INSERT INTO entity_watch_state
                   (entity_id, position_sec, duration_sec, watched_at, completed, caption_mode)
               SELECT ?, position_sec, duration_sec, watched_at, completed, caption_mode
               FROM entity_watch_state WHERE entity_id = ?
               ON CONFLICT(entity_id) DO UPDATE SET
                   position_sec = excluded.position_sec,
                   duration_sec = COALESCE(excluded.duration_sec, entity_watch_state.duration_sec),
                   watched_at   = excluded.watched_at,
                   completed    = excluded.completed,
                   caption_mode = COALESCE(excluded.caption_mode, entity_watch_state.caption_mode)
               WHERE excluded.watched_at > entity_watch_state.watched_at"#,
            survivor,
            absorbed,
        )
        .execute(&mut **tx)
        .await?;
        sqlx::query!(
            "DELETE FROM entity_watch_state WHERE entity_id = ?",
            absorbed
        )
        .execute(&mut **tx)
        .await?;

        // Durable aliases: everything the absorbed identity was known by.
        sqlx::query!(
            r#"INSERT OR IGNORE INTO entity_aliases (entity_id, alias_kind, value, created_at)
               SELECT ?, alias_kind, value, ? FROM entity_aliases WHERE entity_id = ?"#,
            survivor,
            now,
            absorbed,
        )
        .execute(&mut **tx)
        .await?;
        let absorbed_row = sqlx::query!(
            r#"SELECT title as "title!: String", year as "year?: i64"
               FROM canonical_entities WHERE id = ?"#,
            absorbed,
        )
        .fetch_one(&mut **tx)
        .await?;
        let title_key = nova_core::clean::dedup_key(&absorbed_row.title);
        if !title_key.is_empty() {
            sqlx::query!(
                r#"INSERT OR IGNORE INTO entity_aliases (entity_id, alias_kind, value, created_at)
                   VALUES (?, 'title', ?, ?)"#,
                survivor,
                title_key,
                now,
            )
            .execute(&mut **tx)
            .await?;
        }
        if let Some(year) = absorbed_row.year.map(|y| y.to_string()) {
            sqlx::query!(
                r#"INSERT OR IGNORE INTO entity_aliases (entity_id, alias_kind, value, created_at)
                   VALUES (?, 'year', ?, ?)"#,
                survivor,
                year,
                now,
            )
            .execute(&mut **tx)
            .await?;
        }

        // Cannot-link decisions describe the identity, so they follow it.
        let links = sqlx::query!(
            r#"SELECT entity_a as "a!: String", entity_b as "b!: String",
                      operation_id as "op!: i64"
               FROM entity_cannot_link WHERE entity_a = ? OR entity_b = ?"#,
            absorbed,
            absorbed,
        )
        .fetch_all(&mut **tx)
        .await?;
        for link in links {
            let other = if link.a == absorbed { link.b } else { link.a };
            let Some(pair) =
                CannotLink::new(parse_entity(survivor.to_owned())?, parse_entity(other)?)
            else {
                continue;
            };
            let (a, b) = (pair.pair().0.as_str(), pair.pair().1.as_str());
            sqlx::query!(
                r#"INSERT OR IGNORE INTO entity_cannot_link (entity_a, entity_b, operation_id, created_at)
                   VALUES (?, ?, ?, ?)"#,
                a,
                b,
                link.op,
                now,
            )
            .execute(&mut **tx)
            .await?;
        }

        // Redirect, compressing any chain that pointed at the absorbed entity.
        sqlx::query!(
            "UPDATE entity_redirects SET new_id = ? WHERE new_id = ?",
            survivor,
            absorbed,
        )
        .execute(&mut **tx)
        .await?;
        sqlx::query!(
            r#"INSERT INTO entity_redirects (old_id, new_id, operation_id, created_at)
               VALUES (?, ?, ?, ?)"#,
            absorbed,
            survivor,
            operation,
            now,
        )
        .execute(&mut **tx)
        .await?;
        sqlx::query!(
            r#"UPDATE canonical_entities
               SET state = 'merged', is_favorite = 0, is_in_watchlist = 0
               WHERE id = ?"#,
            absorbed,
        )
        .execute(&mut **tx)
        .await?;

        // Legacy row mirror follows the survivor.
        sqlx::query!(
            r#"UPDATE vod_items
               SET is_favorite = (SELECT is_favorite FROM canonical_entities WHERE id = ?),
                   is_in_watchlist = (SELECT is_in_watchlist FROM canonical_entities WHERE id = ?)
               WHERE id IN (
                   SELECT v.id FROM vod_items v
                   JOIN stream_variants s
                     ON s.source_id = v.source_id AND s.binding_key = v.binding_key
                   WHERE s.entity_id = ?)"#,
            survivor,
            survivor,
            survivor,
        )
        .execute(&mut **tx)
        .await?;
        Ok(true)
    }

    /// Secret-free snapshot of one entity's user state and variant IDs.
    async fn state_snapshot(
        tx: &mut Transaction<'_, Sqlite>,
        entity: &str,
    ) -> Result<serde_json::Value, StorageError> {
        let row = sqlx::query!(
            r#"SELECT e.is_favorite as "is_favorite!: i64",
                      e.is_in_watchlist as "is_in_watchlist!: i64",
                      w.position_sec as "position_sec?: f64",
                      w.duration_sec as "duration_sec?: f64",
                      w.watched_at as "watched_at?: i64",
                      w.completed as "completed?: i64",
                      w.caption_mode as "caption_mode?: String"
               FROM canonical_entities e
               LEFT JOIN entity_watch_state w ON w.entity_id = e.id
               WHERE e.id = ?"#,
            entity,
        )
        .fetch_one(&mut **tx)
        .await?;
        let variants = sqlx::query_scalar!(
            r#"SELECT id as "id!: String" FROM stream_variants WHERE entity_id = ? ORDER BY id"#,
            entity,
        )
        .fetch_all(&mut **tx)
        .await?;
        let watch = row.watched_at.map(|watched_at| {
            serde_json::json!({
                "positionSec": row.position_sec,
                "durationSec": row.duration_sec,
                "watchedAt": watched_at,
                "completed": row.completed.unwrap_or(0) != 0,
                "captionMode": row.caption_mode,
            })
        });
        Ok(serde_json::json!({
            "id": entity,
            "isFavorite": row.is_favorite != 0,
            "isInWatchlist": row.is_in_watchlist != 0,
            "watch": watch,
            "variants": variants,
        }))
    }

    // ── Historical IDs ──────────────────────────────────────────────────────

    /// The active entity an ID refers to today: itself, or the survivor of
    /// the merges it went through. `None` for an unknown ID.
    pub async fn canonical_entity_id(
        &self,
        entity_id: &EntityId,
    ) -> Result<Option<EntityId>, StorageError> {
        let mut conn = self.pool.acquire().await?;
        Self::canonical_entity_conn(&mut conn, entity_id).await
    }

    /// Bounded, cycle-checked redirect walk. A dangling tombstone or a cycle
    /// is a corrupt record, never silently resolved to something arbitrary.
    /// [`Self::canonical_entity_conn`] restricted to movies: the movie
    /// user-state APIs must not act on a series entity ID.
    pub(crate) async fn movie_entity_conn(
        conn: &mut SqliteConnection,
        entity_id: &EntityId,
    ) -> Result<Option<EntityId>, StorageError> {
        let Some(current) = Self::canonical_entity_conn(conn, entity_id).await? else {
            return Ok(None);
        };
        let id = current.as_str();
        let is_movie = sqlx::query_scalar!(
            r#"SELECT COUNT(*) as "n!: i64" FROM canonical_entities WHERE id = ? AND kind = 'movie'"#,
            id,
        )
        .fetch_one(&mut *conn)
        .await?;
        Ok((is_movie > 0).then_some(current))
    }

    pub(crate) async fn canonical_entity_conn(
        conn: &mut SqliteConnection,
        entity_id: &EntityId,
    ) -> Result<Option<EntityId>, StorageError> {
        let mut current = entity_id.as_str().to_owned();
        let mut seen = BTreeSet::new();
        for _ in 0..=MAX_REDIRECT_HOPS {
            let row = sqlx::query!(
                r#"SELECT e.state as "state!: String", r.new_id as "new_id?: String"
                   FROM canonical_entities e
                   LEFT JOIN entity_redirects r ON r.old_id = e.id
                   WHERE e.id = ?"#,
                current,
            )
            .fetch_optional(&mut *conn)
            .await?;
            let Some(row) = row else {
                return Ok(None);
            };
            if row.state == "active" {
                return parse_entity(current).map(Some);
            }
            let Some(next) = row.new_id else {
                return Err(StorageError::InvalidCanonicalRecord);
            };
            if !seen.insert(current) {
                return Err(StorageError::InvalidCanonicalRecord);
            }
            current = next;
        }
        Err(StorageError::InvalidCanonicalRecord)
    }

    // ── Corrections and history ─────────────────────────────────────────────

    /// Record a user's decision that two entities are different works. It
    /// prevents any future automatic merge between them (and their
    /// survivors). Returns false when either is unknown or they are already
    /// one entity (undoing a merge is a split, which is not supported yet).
    pub async fn record_cannot_link(
        &self,
        a: &EntityId,
        b: &EntityId,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        let (Some(a), Some(b)) = (
            Self::canonical_entity_conn(&mut tx, a).await?,
            Self::canonical_entity_conn(&mut tx, b).await?,
        ) else {
            return Ok(false);
        };
        let Some(pair) = CannotLink::new(a, b) else {
            return Ok(false);
        };
        let (lo, hi) = (pair.pair().0.as_str(), pair.pair().1.as_str());
        let now = now_ts();
        let operation = sqlx::query!(
            r#"INSERT INTO entity_lineage
                   (operation, entity_a, entity_b, actor, evidence_json, prior_state_json, created_at)
               VALUES ('cannot_link', ?, ?, 'user', '{}', '{}', ?)"#,
            lo,
            hi,
            now,
        )
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
        sqlx::query!(
            r#"INSERT OR IGNORE INTO entity_cannot_link (entity_a, entity_b, operation_id, created_at)
               VALUES (?, ?, ?, ?)"#,
            lo,
            hi,
            operation,
            now,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Every identity operation that involved `entity_id`, oldest first.
    pub async fn lineage(&self, entity_id: &EntityId) -> Result<Vec<LineageEntry>, StorageError> {
        let id = entity_id.as_str();
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", operation as "operation!: String",
                      entity_a as "entity_a!: String", entity_b as "entity_b!: String",
                      actor as "actor!: String", rule as "rule?: String",
                      ruleset_version as "ruleset_version?: i64",
                      evidence_json as "evidence_json!: String",
                      prior_state_json as "prior_state_json!: String",
                      created_at as "created_at!: i64"
               FROM entity_lineage
               WHERE entity_a = ? OR entity_b = ?
               ORDER BY id"#,
            id,
            id,
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok(LineageEntry {
                    id: r.id,
                    operation: r.operation,
                    entity_a: parse_entity(r.entity_a)?,
                    entity_b: parse_entity(r.entity_b)?,
                    actor: r.actor,
                    rule: r.rule,
                    ruleset_version: r.ruleset_version,
                    evidence_json: r.evidence_json,
                    prior_state_json: r.prior_state_json,
                    created_at: r.created_at,
                })
            })
            .collect()
    }
}
