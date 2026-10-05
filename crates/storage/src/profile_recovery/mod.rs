//! Explicit converter for one recovered, divergent migration lineage.
//! Historical-schema SQL is confined to this conversion boundary.
mod interruption;
mod qualification;
use std::{borrow::Cow, path::Path, time::Duration};

use sqlx::{
    migrate::{Migration, MigrationType, Migrator},
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use thiserror::Error;

use self::qualification::{
    fingerprint, legacy_flags, lineage, preserved_rows, verify_flags, verify_preserved,
};

#[derive(Debug, Error)]
pub enum ConversionError {
    #[error("another recovery is in progress")]
    Busy,
    #[error("unsupported, incomplete or altered migration lineage")]
    Lineage,
    #[error("schema differs from the recovered legacy schema")]
    Schema,
    #[error("filesystem operation failed or output already exists")]
    Filesystem,
    #[error("database operation failed (details withheld)")]
    Database,
    #[error("current migration or backfill failed (details withheld)")]
    Migration,
    #[error("preservation, integrity or reopen check failed (details withheld)")]
    Validation,
}
impl From<sqlx::Error> for ConversionError {
    fn from(_: sqlx::Error) -> Self {
        Self::Database
    }
}
impl From<std::io::Error> for ConversionError {
    fn from(_: std::io::Error) -> Self {
        Self::Filesystem
    }
}

pub fn historical_migrator() -> Migrator {
    let mut old = sqlx::migrate!("./migrations/profile");
    let mut migrations: Vec<_> = old.iter().filter(|m| m.version <= 19).cloned().collect();
    for (version, description, sql) in [
        (
            20,
            "llm about",
            include_str!("local_llm_about/0020_llm_about.sql"),
        ),
        (
            21,
            "tmdb credits",
            include_str!("local_llm_about/0021_tmdb_credits.sql"),
        ),
    ] {
        migrations.push(Migration::new(
            version,
            Cow::Borrowed(description),
            MigrationType::Simple,
            Cow::Borrowed(sql),
            false,
        ));
    }
    old.migrations = Cow::Owned(migrations);
    old
}

pub async fn open(path: &Path, read_only: bool) -> Result<SqlitePool, ConversionError> {
    let mut options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(read_only)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    // Do not request a journal-mode change on the original read-only connection.
    if !read_only {
        options = options
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal);
    }
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?)
}

async fn validate_legacy(pool: &SqlitePool) -> Result<(), ConversionError> {
    let applied =
        sqlx::query("SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await?;
    let expected = historical_migrator();
    if applied.len() != expected.iter().count() {
        return Err(ConversionError::Lineage);
    }
    for (row, migration) in applied.iter().zip(expected.iter()) {
        let version: i64 = row.try_get("version")?;
        let checksum: Vec<u8> = row.try_get("checksum")?;
        let success: bool = row.try_get("success")?;
        if version != migration.version
            || !success
            || checksum.as_slice() != migration.checksum.as_ref()
        {
            return Err(ConversionError::Lineage);
        }
    }
    // Check actual structure as well as ledger claims. No repairs based on a
    // checksum alone: a changed schema could hide triggers or incompatible data.
    let reference = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    expected
        .run(&reference)
        .await
        .map_err(|_| ConversionError::Schema)?;
    let expected_schema = fingerprint(&reference, "sqlite_master", "type,name,tbl_name,sql")
        .await
        .map_err(|_| ConversionError::Schema)?;
    reference.close().await;
    let actual_schema = fingerprint(pool, "sqlite_master", "type,name,tbl_name,sql")
        .await
        .map_err(|_| ConversionError::Schema)?;
    if expected_schema.digest != actual_schema.digest {
        return Err(ConversionError::Schema);
    }
    integrity(pool).await
}

async fn integrity(pool: &SqlitePool) -> Result<(), ConversionError> {
    let rows: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_all(pool)
        .await?;
    if rows != ["ok"]
        || !sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await?
            .is_empty()
    {
        return Err(ConversionError::Validation);
    }
    Ok(())
}

pub async fn catalog_valid(pool: &SqlitePool) -> Result<(), ConversionError> {
    let missing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM vod_items v
         LEFT JOIN stream_variants s ON s.source_id=v.source_id AND s.binding_key=v.binding_key
         LEFT JOIN canonical_entities e ON e.id=s.entity_id
         WHERE e.id IS NULL OR (v.is_favorite=1 AND e.is_favorite=0)
            OR (v.is_in_watchlist=1 AND e.is_in_watchlist=0)
            OR (EXISTS(SELECT 1 FROM watch_history h WHERE h.media_kind='vod' AND h.media_id=v.id)
                AND NOT EXISTS(SELECT 1 FROM entity_watch_state w WHERE w.entity_id=e.id))",
    )
    .fetch_one(pool)
    .await?;
    if missing != 0 {
        return Err(ConversionError::Validation);
    }
    let series_missing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM series_episodes e
         LEFT JOIN series_bindings b ON b.source_id=e.source_id AND b.series_title=e.series_title
         LEFT JOIN canonical_entities c ON c.id=b.entity_id
         WHERE c.id IS NULL OR c.kind<>'series' OR e.episode_key IS NULL
            OR (e.is_favorite=1 AND c.is_favorite=0)
            OR (e.is_in_watchlist=1 AND c.is_in_watchlist=0)",
    )
    .fetch_one(pool)
    .await?;
    // Merges retain latest progress, with nullable duration/caption values
    // optionally carried from another member. Equal-time ties may retain either
    // actual latest row, but never invented/reset position or completion values.
    let movie_progress: i64 = sqlx::query_scalar(
        "WITH history AS (
           SELECT s.entity_id, h.* FROM watch_history h
           JOIN vod_items v ON h.media_kind='vod' AND h.media_id=v.id
           JOIN stream_variants s ON s.source_id=v.source_id AND s.binding_key=v.binding_key
         ), latest AS (SELECT entity_id, MAX(watched_at) AS watched_at FROM history GROUP BY entity_id)
         SELECT COUNT(*) FROM latest l
         WHERE NOT EXISTS (
           SELECT 1 FROM entity_watch_state w JOIN history h ON h.entity_id=w.entity_id
           WHERE w.entity_id=l.entity_id AND w.watched_at=l.watched_at AND h.watched_at=l.watched_at
             AND w.position_sec IS h.position_sec AND w.completed IS h.completed
             AND (w.duration_sec IS h.duration_sec OR (h.duration_sec IS NULL AND EXISTS (
               SELECT 1 FROM history p WHERE p.entity_id=w.entity_id AND p.duration_sec IS w.duration_sec)))
             AND (w.caption_mode IS h.caption_mode OR (h.caption_mode IS NULL AND EXISTS (
               SELECT 1 FROM history p WHERE p.entity_id=w.entity_id AND p.caption_mode IS w.caption_mode)))
         )"
    ).fetch_one(pool).await?;
    let episode_progress: i64 = sqlx::query_scalar(
        "WITH history AS (
           SELECT b.entity_id, e.episode_key, e.season, e.episode, h.* FROM watch_history h
           JOIN series_episodes e ON h.media_kind='episode' AND h.media_id=e.id
           JOIN series_bindings b ON b.source_id=e.source_id AND b.series_title=e.series_title
         ), latest AS (SELECT entity_id, episode_key, MAX(watched_at) AS watched_at FROM history GROUP BY entity_id,episode_key)
         SELECT COUNT(*) FROM latest l
         WHERE NOT EXISTS (
           SELECT 1 FROM episode_watch_state w JOIN history h ON h.entity_id=w.series_entity_id AND h.episode_key=w.episode_key
           WHERE w.series_entity_id=l.entity_id AND w.episode_key=l.episode_key
             AND w.watched_at=l.watched_at AND h.watched_at=l.watched_at
             AND w.position_sec IS h.position_sec AND w.duration_sec IS h.duration_sec
             AND w.completed IS h.completed AND w.caption_mode IS h.caption_mode
             AND w.season IS h.season AND w.episode IS h.episode
         )"
    ).fetch_one(pool).await?;
    if series_missing != 0 || movie_progress != 0 || episode_progress != 0 {
        return Err(ConversionError::Validation);
    }
    Ok(())
}

async fn convert_staged(path: &Path) -> Result<(), ConversionError> {
    let pool = open(path, false).await?;
    let preparation = async {
        validate_legacy(&pool).await?;
        let flags = legacy_flags(&pool)
            .await
            .map_err(|_| ConversionError::Validation)?;
        let mut baseline = preserved_rows(&pool)
            .await
            .map_err(|_| ConversionError::Validation)?;
        for (table, columns) in [
            ("llm_about", "kind,item_key,model,lang,text,created_at"),
            ("vod_items", "id,director,tmdb_cast,runtime,rating"),
            (
                "series_meta",
                "source_id,series_title,director,tmdb_cast,runtime,rating",
            ),
            ("transcripts", "id,audio_track,content_key,complete"),
            (
                "transcript_translations",
                "id,segment_id,target_lang,backend,text,created_at",
            ),
        ] {
            baseline.push(
                fingerprint(&pool, table, columns)
                    .await
                    .map_err(|_| ConversionError::Validation)?,
            );
        }
        let ledger = fingerprint(
            &pool,
            "_sqlx_migrations",
            "version,description,installed_on,success,checksum,execution_time",
        )
        .await
        .map_err(|_| ConversionError::Validation)?;
        let mut tx = pool.begin().await?;
        sqlx::raw_sql(
            "CREATE TABLE nova_legacy_migration_ledger_v1 AS SELECT * FROM _sqlx_migrations;
             CREATE TABLE nova_profile_conversion_v1 (
                lineage TEXT PRIMARY KEY, converter_version INTEGER NOT NULL);
             INSERT INTO nova_profile_conversion_v1 VALUES ('local-llm-about-21', 1);
             DELETE FROM _sqlx_migrations WHERE version IN (20,21);",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        baseline.push(self::qualification::PreservedRows {
            table: "nova_legacy_migration_ledger_v1",
            ..ledger
        });
        Ok::<_, ConversionError>((baseline, flags))
    }
    .await;
    pool.close().await;
    let (baseline, flags) = preparation?;
    let path = path.to_str().ok_or(ConversionError::Filesystem)?;
    let upgraded = crate::init_profile_db(path)
        .await
        .map_err(|_| ConversionError::Migration)?;
    let check = async {
        verify_preserved(&upgraded, &baseline).await.map_err(|_| ConversionError::Validation)?;
        verify_flags(&upgraded, &flags).await.map_err(|_| ConversionError::Validation)?;
        integrity(&upgraded).await?;
        catalog_valid(&upgraded).await?;
        // All canonical state, not just counts, must be stable on reopen.
        let mut reopened_baseline = baseline;
        for (table, columns) in [
            ("canonical_entities", "id,kind,title,year,is_favorite,is_in_watchlist,created_at,state"),
            ("entity_watch_state", "entity_id,position_sec,duration_sec,watched_at,completed,caption_mode"),
            ("episode_watch_state", "series_entity_id,episode_key,position_sec,duration_sec,watched_at,completed,caption_mode"),
        ] {
            reopened_baseline.push(fingerprint(&upgraded, table, columns).await.map_err(|_| ConversionError::Validation)?);
        }
        Ok::<_, ConversionError>(reopened_baseline)
    }.await;
    upgraded.close().await;
    let baseline = check?;
    let reopened = crate::init_profile_db(path)
        .await
        .map_err(|_| ConversionError::Migration)?;
    let result = async {
        verify_preserved(&reopened, &baseline)
            .await
            .map_err(|_| ConversionError::Validation)?;
        verify_flags(&reopened, &flags)
            .await
            .map_err(|_| ConversionError::Validation)?;
        integrity(&reopened).await?;
        catalog_valid(&reopened).await?;
        if lineage(&reopened)
            .await
            .map_err(|_| ConversionError::Validation)?
            .1
            != 0
        {
            return Err(ConversionError::Validation);
        }
        let checkpoint: (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(&reopened)
            .await?;
        if checkpoint.0 != 0 {
            return Err(ConversionError::Validation);
        }
        Ok(())
    }
    .await;
    reopened.close().await;
    result
}

async fn vacant_output(output: &Path) -> Result<(), ConversionError> {
    // A leftover hot journal/WAL can override a validated main database when
    // SQLite opens it. Check dangling symlinks too; never remove such files.
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut path = output.as_os_str().to_os_string();
        path.push(suffix);
        match tokio::fs::symlink_metadata(Path::new(&path)).await {
            Ok(_) => return Err(ConversionError::Filesystem),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ConversionError::Filesystem),
        }
    }
    Ok(())
}

pub async fn convert(source: &Path, output: &Path) -> Result<(), ConversionError> {
    vacant_output(output).await?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let dir = tempfile::Builder::new()
        .prefix(".nova-profile-convert-")
        .tempdir_in(parent)?;
    let copy = dir.path().join("profile.db");
    let original = open(source, true).await?;
    let snapshot = sqlx::query("VACUUM INTO ?")
        .bind(copy.to_str().ok_or(ConversionError::Filesystem)?)
        .execute(&original)
        .await;
    original.close().await;
    snapshot?;
    convert_staged(&copy).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o600)).await?;
    }
    tokio::fs::File::open(&copy).await?.sync_all().await?;
    vacant_output(output).await?;
    // Same filesystem: atomic no-clobber publication only after all checks pass.
    tokio::fs::hard_link(&copy, output).await?;
    Ok(())
}

/// Read-only eligibility inspection. An unsupported or unreadable database never
/// enables recovery; `convert` independently repeats all checks on its snapshot.
pub async fn is_supported(source: &Path) -> bool {
    let Ok(pool) = open(source, true).await else {
        return false;
    };
    let supported = validate_legacy(&pool).await.is_ok();
    pool.close().await;
    supported
}

/// Install a qualified copy as a new profile, preserving the original as backup.
/// The caller serializes profile opens/deletes and owns the profiles directory.
/// App registration is the commit point. An interrupted attempt leaves at most
/// a receipt-bearing staging directory (and a complete, unregistered published
/// copy), which the next recovery's cleanup removes; never a visible profile.
pub async fn recover_as_profile(
    app_pool: &SqlitePool,
    profiles_root: &Path,
    source_id: &str,
) -> Result<nova_core::Profile, ConversionError> {
    let parsed = uuid::Uuid::parse_str(source_id).map_err(|_| ConversionError::Lineage)?;
    if parsed.to_string() != source_id {
        return Err(ConversionError::Lineage);
    }
    let original = crate::ProfileRepository::new(app_pool)
        .get(source_id)
        .await
        .map_err(|_| ConversionError::Lineage)?;
    let source_dir = profiles_root.join(source_id);
    let source = source_dir.join("profile.db");
    // Profile paths originate in app metadata, never from an IPC filesystem path.
    // Refuse redirected directories/databases before reading a recovery source.
    for path in [&source_dir, &source] {
        if tokio::fs::symlink_metadata(path)
            .await?
            .file_type()
            .is_symlink()
        {
            return Err(ConversionError::Filesystem);
        }
    }
    let _lease = interruption::lock(profiles_root).await?;
    interruption::cleanup(profiles_root, app_pool).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let receipt = interruption::Receipt::new(id.clone(), source_id.to_owned());
    // Not removed on drop: if this future is cancelled or the process exits,
    // the staging receipt lets a later cleanup reconcile the attempt.
    let staging = tempfile::Builder::new()
        .prefix(&format!("{}{id}-", interruption::PREFIX))
        .tempdir_in(profiles_root)?
        .keep();
    let result = publish_recovered(
        app_pool,
        profiles_root,
        &staging,
        &receipt,
        &source,
        &id,
        &original,
    )
    .await;
    // Completed attempts remove their own staging; a failure here is retried
    // by the next cleanup, which recognises the receipt. If a failed attempt
    // could not withdraw its published target, keep staging as its proof.
    let withdrawn = result.is_ok()
        || tokio::fs::symlink_metadata(profiles_root.join(&id))
            .await
            .is_err();
    if withdrawn && interruption::remove_staging(&staging).await.is_ok() {
        let _ = interruption::sync_directory(profiles_root).await;
    }
    result
}

async fn publish_recovered(
    app_pool: &SqlitePool,
    profiles_root: &Path,
    staging: &Path,
    receipt: &interruption::Receipt,
    source: &Path,
    id: &str,
    original: &nova_core::Profile,
) -> Result<nova_core::Profile, ConversionError> {
    #[cfg(test)]
    interruption::checkpoint("unwritten").await;
    receipt.write(staging).await?;
    interruption::sync_directory(profiles_root).await?;
    #[cfg(test)]
    interruption::checkpoint("staging").await;
    let copy = staging.join("profile.db");
    convert(source, &copy).await?;
    #[cfg(test)]
    interruption::checkpoint("converted").await;
    // Assemble the complete profile directory privately, then publish it with
    // one rename so it never appears under its UUID without its receipt.
    let assembled = staging.join("publish");
    tokio::fs::create_dir(&assembled).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&assembled, std::fs::Permissions::from_mode(0o700)).await?;
    }
    #[cfg(test)]
    interruption::checkpoint("assembling").await;
    receipt.write(&assembled).await?;
    tokio::fs::hard_link(&copy, assembled.join("profile.db")).await?;
    interruption::sync_directory(&assembled).await?;
    interruption::sync_directory(staging).await?;
    let directory = profiles_root.join(id);
    // Never repurpose an existing entry; POSIX rename may replace an empty one.
    match tokio::fs::symlink_metadata(&directory).await {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(ConversionError::Filesystem),
    }
    tokio::fs::rename(&assembled, &directory).await?;
    let registered = async {
        // Flush the directory before app.db can point at it.
        interruption::sync_directory(profiles_root).await?;
        #[cfg(test)]
        interruption::checkpoint("published").await;
        crate::ProfileRepository::new(app_pool)
            .register(
                id,
                &format!("{} (Recovered)", original.name),
                &original.avatar_color,
            )
            .await
            .map_err(|_| ConversionError::Database)
    }
    .await;
    match registered {
        Err(error) => {
            // Only the directory this attempt just published; the staging
            // removal (or a later cleanup) disposes of it.
            let _ = tokio::fs::rename(&directory, staging.join("discarded")).await;
            Err(error)
        }
        Ok(profile) => {
            #[cfg(test)]
            interruption::checkpoint("registered").await;
            // Committed. Opening the profile releases the receipt again if this
            // fails or the process exits first.
            if interruption::release(&directory).await.is_err() {
                tracing::warn!("recovered profile receipt was not released");
            }
            Ok(profile)
        }
    }
}

/// Drop the recovery ownership receipt from a registered profile's directory
/// before it is opened, so later cleanup can never remove a profile in use.
pub async fn release_recovery_receipt(profile_dir: &Path) -> Result<(), ConversionError> {
    interruption::release(profile_dir).await
}

/// Remove only proven, unregistered interrupted copies. The lock makes this safe
/// against another process still converting a profile.
pub async fn cleanup_interrupted_profiles(
    app_pool: &SqlitePool,
    profiles_root: &Path,
) -> Result<(), ConversionError> {
    let _lease = interruption::lock(profiles_root).await?;
    interruption::cleanup(profiles_root, app_pool).await
}
