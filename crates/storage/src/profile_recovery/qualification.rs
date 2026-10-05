//! Shared checks for offline profile tools and their tests. No profile content is logged.
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};

/// Check all applied checksums before calling the migration runner. A database
/// at head qualifies reopen only, not an upgrade from an older schema.
pub async fn lineage(pool: &SqlitePool) -> Result<(i64, usize), String> {
    let migrator = sqlx::migrate!("./migrations/profile");
    let applied =
        sqlx::query("SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .map_err(|_| "cannot read migration ledger".to_owned())?;
    let mut latest = 0;
    let mut versions = Vec::new();
    for row in applied {
        let version: i64 = row.get("version");
        let checksum: Vec<u8> = row.get("checksum");
        let success: bool = row.get("success");
        if !success {
            return Err(format!("migration {version} is incomplete"));
        }
        let Some(expected) = migrator
            .iter()
            .find(|migration| migration.version == version)
        else {
            return Err(format!("migration {version} is unknown to this build"));
        };
        if expected.checksum.as_ref() != checksum.as_slice() {
            return Err(format!("migration {version} has a different checksum; unsupported lineage, no override performed"));
        }
        latest = version;
        versions.push(version);
    }
    let pending = migrator
        .iter()
        .filter(|migration| !versions.contains(&migration.version))
        .count();
    Ok((latest, pending))
}

// Stable source/user columns only: enrichment fields, canonical binding keys
// and detached transcript media IDs are intentionally changed by migrations.
// Film and episode flags are checked by `verify_flags` instead: canonical
// merging may promote them across copies of the same title, never drop them.
const PRESERVED: &[(&str, &str)] = &[
    ("sources", "id,kind,name,url"),
    (
        "channels",
        "id,source_id,name,stream_url,is_favorite,is_in_watchlist",
    ),
    ("vod_items", "id,source_id,name,stream_url"),
    (
        "series_episodes",
        "id,source_id,series_title,season,episode,stream_url",
    ),
    (
        "watch_history",
        "id,media_kind,media_id,position_sec,duration_sec,watched_at,completed,caption_mode",
    ),
    (
        "transcripts",
        "id,media_kind,language,model,vtt_path,json_path,created_at,duration_processed_sec",
    ),
    (
        "transcript_segments",
        "id,transcript_id,seq,start_sec,end_sec,text,words_json",
    ),
    ("transcript_windows", "id,transcript_id,start_sec,end_sec"),
    (
        "caption_preferences",
        "id,scope,scope_key,caption_mode,updated_at",
    ),
    ("profile_settings", "key,value"),
];

pub struct PreservedRows {
    pub table: &'static str,
    pub columns: &'static str,
    pub rows: usize,
    pub digest: Vec<u8>,
}

/// Values and digests stay in memory and are never printed on assertion failure.
/// Sorting encoded rows makes the comparison independent of SQLite row order.
pub async fn fingerprint(
    pool: &SqlitePool,
    table: &'static str,
    columns: &'static str,
) -> Result<PreservedRows, String> {
    let projection = columns
        .split(',')
        // Decode each value using its actual storage type. SQL quote() truncates
        // embedded NUL, and casting REAL to text loses low floating-point bits.
        .map(|column| format!("typeof({column}), {column}"))
        .collect::<Vec<_>>()
        .join(",");
    // Migration 20 deliberately deletes orphan history; only those rows are
    // excluded. Valid history must survive even if all resume rows disappear.
    let filter = if table == "watch_history" {
        "WHERE NOT (
            (media_kind = 'vod' AND NOT EXISTS (SELECT 1 FROM vod_items WHERE id = media_id))
            OR (media_kind = 'live' AND NOT EXISTS (SELECT 1 FROM channels WHERE id = media_id))
            OR (media_kind = 'episode' AND NOT EXISTS (SELECT 1 FROM series_episodes WHERE id = media_id))
        )"
    } else {
        ""
    };
    let result = sqlx::query(&format!("SELECT {projection} FROM {table} {filter}"))
        .fetch_all(pool)
        .await
        .map_err(|_| "cannot read preservation data".to_owned())?;
    let mut rows = Vec::with_capacity(result.len());
    for row in &result {
        let mut bytes = Vec::new();
        for i in (0..row.len()).step_by(2) {
            let kind: String = row
                .try_get(i)
                .map_err(|_| "cannot decode storage type".to_owned())?;
            let value = match kind.as_str() {
                "null" => Vec::new(),
                "integer" => row
                    .try_get::<i64, _>(i + 1)
                    .map_err(|_| "cannot decode integer".to_owned())?
                    .to_le_bytes()
                    .to_vec(),
                "real" => row
                    .try_get::<f64, _>(i + 1)
                    .map_err(|_| "cannot decode real".to_owned())?
                    .to_bits()
                    .to_le_bytes()
                    .to_vec(),
                "text" => row
                    .try_get::<String, _>(i + 1)
                    .map_err(|_| "cannot decode text".to_owned())?
                    .into_bytes(),
                "blob" => row
                    .try_get::<Vec<u8>, _>(i + 1)
                    .map_err(|_| "cannot decode blob".to_owned())?,
                _ => return Err("unknown storage type".to_owned()),
            };
            bytes.extend_from_slice(kind.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
            bytes.extend_from_slice(&value);
        }
        rows.push(bytes);
    }
    rows.sort_unstable();
    let mut digest = Sha256::new();
    for row in &rows {
        digest.update((row.len() as u64).to_le_bytes());
        digest.update(row);
    }
    Ok(PreservedRows {
        table,
        columns,
        rows: rows.len(),
        digest: digest.finalize().to_vec(),
    })
}

pub async fn preserved_rows(pool: &SqlitePool) -> Result<Vec<PreservedRows>, String> {
    let mut result = Vec::new();
    for &(table, columns) in PRESERVED {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
        )
        .bind(table)
        .fetch_one(pool)
        .await
        .map_err(|_| "cannot read preservation data".to_owned())?;
        if exists {
            result.push(fingerprint(pool, table, columns).await?);
        }
    }
    Ok(result)
}

pub async fn verify_preserved(pool: &SqlitePool, baseline: &[PreservedRows]) -> Result<(), String> {
    for before in baseline {
        let after = fingerprint(pool, before.table, before.columns).await?;
        if before.rows != after.rows || before.digest != after.digest {
            return Err(format!(
                "source/user data changed in {} (contents withheld)",
                before.table
            ));
        }
    }
    Ok(())
}

/// Per-row favorite/watchlist flags of the tables whose rows canonical
/// merging groups into one film or series. Kept in memory, never printed.
pub struct FlagBaseline {
    rows: HashMap<(&'static str, i64), [bool; 2]>,
}

const FLAGGED: [&str; 2] = ["vod_items", "series_episodes"];

pub async fn legacy_flags(pool: &SqlitePool) -> Result<FlagBaseline, String> {
    let mut rows = HashMap::new();
    for table in FLAGGED {
        let result = sqlx::query(&format!(
            "SELECT id, is_favorite, is_in_watchlist FROM {table}"
        ))
        .fetch_all(pool)
        .await
        .map_err(|_| "cannot read legacy flags".to_owned())?;
        for row in result {
            let flags = [row.get::<bool, _>(1), row.get::<bool, _>(2)];
            rows.insert((table, row.get::<i64, _>(0)), flags);
        }
    }
    Ok(FlagBaseline { rows })
}

/// A flag may never be lost. It may only appear on a row whose canonical
/// title (film entity or series entity) already had that flag on another
/// of its rows before conversion: the merge mirroring the user's choice.
pub async fn verify_flags(pool: &SqlitePool, baseline: &FlagBaseline) -> Result<(), String> {
    for table in FLAGGED {
        let groups = match table {
            "vod_items" => {
                "SELECT v.id, v.is_favorite, v.is_in_watchlist, s.entity_id FROM vod_items v
                 LEFT JOIN stream_variants s
                   ON s.source_id = v.source_id AND s.binding_key = v.binding_key"
            }
            _ => {
                "SELECT e.id, e.is_favorite, e.is_in_watchlist, b.entity_id FROM series_episodes e
                 LEFT JOIN series_bindings b
                   ON b.source_id = e.source_id AND b.series_title = e.series_title"
            }
        };
        let result = sqlx::query(groups)
            .fetch_all(pool)
            .await
            .map_err(|_| "cannot read converted flags".to_owned())?;
        let mut flagged_before: [HashSet<String>; 2] = Default::default();
        let mut rows = Vec::with_capacity(result.len());
        for row in result {
            let id: i64 = row.get(0);
            let after = [row.get::<bool, _>(1), row.get::<bool, _>(2)];
            let group: Option<String> = row.get(3);
            let before = *baseline
                .rows
                .get(&(table, id))
                .ok_or_else(|| format!("unexpected row in {table} (contents withheld)"))?;
            if let Some(group) = &group {
                for flag in 0..2 {
                    if before[flag] {
                        flagged_before[flag].insert(group.clone());
                    }
                }
            }
            rows.push((before, after, group));
        }
        for (before, after, group) in rows {
            for flag in 0..2 {
                if before[flag] && !after[flag] {
                    return Err(format!(
                        "a user flag was lost in {table} (contents withheld)"
                    ));
                }
                let earned = group
                    .as_ref()
                    .is_some_and(|g| flagged_before[flag].contains(g));
                if !before[flag] && after[flag] && !earned {
                    return Err(format!(
                        "a user flag appeared without a source in {table} (contents withheld)"
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod flag_tests {
    use super::*;

    async fn catalog() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE vod_items (id INTEGER PRIMARY KEY, source_id INTEGER, binding_key TEXT,
                                     is_favorite INTEGER, is_in_watchlist INTEGER);
             CREATE TABLE stream_variants (source_id INTEGER, binding_key TEXT, entity_id TEXT);
             CREATE TABLE series_episodes (id INTEGER PRIMARY KEY, source_id INTEGER, series_title TEXT,
                                           is_favorite INTEGER, is_in_watchlist INTEGER);
             CREATE TABLE series_bindings (source_id INTEGER, series_title TEXT, entity_id TEXT);
             -- Rows 1 and 2 are one film; row 3 is another.
             INSERT INTO vod_items VALUES (1,1,'a',1,0), (2,1,'b',0,0), (3,1,'c',0,0);
             INSERT INTO stream_variants VALUES (1,'a','m1'), (1,'b','m1'), (1,'c','m2');",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn flags_may_spread_within_a_title_but_never_be_lost_or_invented() {
        let pool = catalog().await;
        let baseline = legacy_flags(&pool).await.unwrap();
        // Mirroring the film's favourite onto its other copy is accepted.
        sqlx::query("UPDATE vod_items SET is_favorite = 1 WHERE id = 2")
            .execute(&pool)
            .await
            .unwrap();
        verify_flags(&pool, &baseline).await.unwrap();
        // A flag on a different film with no flagged copy is refused.
        sqlx::query("UPDATE vod_items SET is_in_watchlist = 1 WHERE id = 3")
            .execute(&pool)
            .await
            .unwrap();
        assert!(verify_flags(&pool, &baseline).await.is_err());
        sqlx::query("UPDATE vod_items SET is_in_watchlist = 0 WHERE id = 3")
            .execute(&pool)
            .await
            .unwrap();
        // Losing the user's own flag is refused.
        sqlx::query("UPDATE vod_items SET is_favorite = 0 WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        assert!(verify_flags(&pool, &baseline).await.is_err());
    }
}
