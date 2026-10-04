//! Shared checks for offline profile tools and their tests. No profile content is logged.
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

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
const PRESERVED: &[(&str, &str)] = &[
    ("sources", "id,kind,name,url"),
    (
        "channels",
        "id,source_id,name,stream_url,is_favorite,is_in_watchlist",
    ),
    (
        "vod_items",
        "id,source_id,name,stream_url,is_favorite,is_in_watchlist",
    ),
    (
        "series_episodes",
        "id,source_id,series_title,season,episode,stream_url,is_favorite,is_in_watchlist",
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
