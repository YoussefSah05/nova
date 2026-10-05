//! Opt-in upgrade qualification against a real profile database.
//!
//! The database named by `NOVA_UPGRADE_DB` is copied into a temporary
//! directory with SQLite's `VACUUM INTO`; only the copy is migrated. Output is
//! qualification metadata only — no titles, URLs or other profile content.
//!
//! ```text
//! NOVA_UPGRADE_DB="$HOME/Library/Application Support/app.nova.desktop/profiles/<id>/profile.db" \
//!   SQLX_OFFLINE=true cargo test -p nova-storage --test real_profile_upgrade -- --ignored --nocapture
//! ```

use nova_storage::init_profile_db;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

#[path = "../src/profile_recovery/qualification.rs"]
mod qualification;
use qualification::{lineage, PreservedRows};

async fn fingerprint(
    pool: &SqlitePool,
    table: &'static str,
    columns: &'static str,
) -> PreservedRows {
    qualification::fingerprint(pool, table, columns)
        .await
        .unwrap()
}

async fn preserved_rows(pool: &SqlitePool) -> Vec<PreservedRows> {
    qualification::preserved_rows(pool).await.unwrap()
}

async fn verify_preserved(pool: &SqlitePool, baseline: &[PreservedRows]) {
    qualification::verify_preserved(pool, baseline)
        .await
        .unwrap();
}

async fn count(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[tokio::test]
#[ignore = "requires NOVA_UPGRADE_DB pointing at a profile database"]
async fn real_profile_upgrade_preserves_movie_user_state() {
    let source = std::env::var("NOVA_UPGRADE_DB").expect("set NOVA_UPGRADE_DB");
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("profile.db");

    // Consistent read-only snapshot (includes WAL content); never writes the source.
    let original = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&source)
            .read_only(true),
    )
    .await
    .unwrap();
    sqlx::query("VACUUM INTO ?")
        .bind(copy.to_str().unwrap())
        .execute(&original)
        .await
        .unwrap();
    original.close().await;

    let pool = SqlitePool::connect_with(SqliteConnectOptions::new().filename(&copy))
        .await
        .unwrap();
    let (from, pending) = lineage(&pool)
        .await
        .unwrap_or_else(|reason| panic!("qualification refused: {reason}"));
    eprintln!(
        "source migration {from}; pending migrations {pending}; qualification {}",
        if pending == 0 {
            "current-schema reopen only"
        } else {
            "upgrade"
        }
    );
    let preserved = preserved_rows(&pool).await;
    let flags = qualification::legacy_flags(&pool).await.unwrap();
    let movies = count(&pool, "SELECT COUNT(*) FROM vod_items").await;
    let favorites = count(
        &pool,
        "SELECT COUNT(*) FROM vod_items WHERE is_favorite = 1",
    )
    .await;
    let watchlist = count(
        &pool,
        "SELECT COUNT(*) FROM vod_items WHERE is_in_watchlist = 1",
    )
    .await;
    let history = count(
        &pool,
        "SELECT COUNT(*) FROM watch_history wh JOIN vod_items v ON v.id = wh.media_id WHERE wh.media_kind = 'vod'",
    )
    .await;
    pool.close().await;

    let started = std::time::Instant::now();
    let upgraded = init_profile_db(copy.to_str().unwrap()).await.unwrap();
    let elapsed = started.elapsed();
    verify_preserved(&upgraded, &preserved).await;
    qualification::verify_flags(&upgraded, &flags)
        .await
        .unwrap_or_else(|reason| panic!("{reason}"));

    // Every legacy favourite / watchlisted row's entity carries the flag.
    let lost_favorites = count(
        &upgraded,
        "SELECT COUNT(*) FROM vod_items v
         JOIN stream_variants s ON s.source_id = v.source_id AND s.binding_key = v.binding_key
         JOIN canonical_entities e ON e.id = s.entity_id
         WHERE v.is_favorite = 1 AND e.is_favorite = 0",
    )
    .await;
    let lost_watchlist = count(
        &upgraded,
        "SELECT COUNT(*) FROM vod_items v
         JOIN stream_variants s ON s.source_id = v.source_id AND s.binding_key = v.binding_key
         JOIN canonical_entities e ON e.id = s.entity_id
         WHERE v.is_in_watchlist = 1 AND e.is_in_watchlist = 0",
    )
    .await;
    // Every movie with legacy history has entity resume state.
    let lost_resume = count(
        &upgraded,
        "SELECT COUNT(*) FROM watch_history wh
         JOIN vod_items v ON v.id = wh.media_id AND wh.media_kind = 'vod'
         JOIN stream_variants s ON s.source_id = v.source_id AND s.binding_key = v.binding_key
         WHERE NOT EXISTS (SELECT 1 FROM entity_watch_state w WHERE w.entity_id = s.entity_id)",
    )
    .await;
    let unmapped = count(
        &upgraded,
        "SELECT COUNT(*) FROM vod_items v WHERE NOT EXISTS (
             SELECT 1 FROM stream_variants s
             WHERE s.source_id = v.source_id AND s.binding_key = v.binding_key)",
    )
    .await;
    let entities = count(&upgraded, "SELECT COUNT(*) FROM canonical_entities").await;
    // Entity resolution (0023) runs on this first open: every merged entity
    // must redirect in one hop to an active survivor.
    let merged = count(
        &upgraded,
        "SELECT COUNT(*) FROM canonical_entities WHERE state = 'merged'",
    )
    .await;
    let dangling = count(
        &upgraded,
        "SELECT COUNT(*) FROM canonical_entities e
         WHERE e.state = 'merged' AND NOT EXISTS (
             SELECT 1 FROM entity_redirects r
             JOIN canonical_entities t ON t.id = r.new_id AND t.state = 'active'
             WHERE r.old_id = e.id)",
    )
    .await;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&upgraded)
        .await
        .unwrap();
    let fk = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&upgraded)
        .await
        .unwrap()
        .len();
    upgraded.close().await;

    let started = std::time::Instant::now();
    let reopened = init_profile_db(copy.to_str().unwrap()).await.unwrap();
    let reopen = started.elapsed();
    let entities_after_reopen = count(&reopened, "SELECT COUNT(*) FROM canonical_entities").await;
    verify_preserved(&reopened, &preserved).await;
    qualification::verify_flags(&reopened, &flags)
        .await
        .unwrap_or_else(|reason| panic!("{reason}"));
    reopened.close().await;

    eprintln!(
        "movies {movies}, favourites {favorites}, watchlist {watchlist}, resume rows {history}; \
         entities {entities} ({merged} merged); lost favourites {lost_favorites}, lost watchlist {lost_watchlist}, \
         lost resume {lost_resume}, unmapped rows {unmapped}; integrity {integrity}, fk issues {fk}; \
         initialization {elapsed:?}, reopen {reopen:?}"
    );
    assert_eq!((lost_favorites, lost_watchlist, lost_resume), (0, 0, 0));
    assert_eq!(unmapped, 0);
    assert_eq!(dangling, 0, "every merged entity redirects to a survivor");
    assert_eq!(integrity, "ok");
    assert_eq!(fk, 0);
    assert_eq!(entities, entities_after_reopen, "backfill is idempotent");
}

#[tokio::test]
async fn qualification_rejects_a_different_lineage_without_rewriting_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.db");
    let pool = init_profile_db(path.to_str().unwrap()).await.unwrap();
    assert_eq!(lineage(&pool).await.unwrap().1, 0);
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 20")
        .execute(&pool)
        .await
        .unwrap();
    let before = fingerprint(
        &pool,
        "_sqlx_migrations",
        "version,description,checksum,success",
    )
    .await;
    assert!(lineage(&pool)
        .await
        .unwrap_err()
        .contains("migration 20 has a different checksum"));
    verify_preserved(&pool, &[before]).await;
    pool.close().await;
}

#[tokio::test]
async fn preservation_detects_changed_content_even_when_counts_match() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE profile_settings (key TEXT, value TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO profile_settings VALUES ('viewing', ?)")
        .bind("same\0before")
        .execute(&pool)
        .await
        .unwrap();
    let before = fingerprint(&pool, "profile_settings", "key,value").await;
    sqlx::query("UPDATE profile_settings SET value=?")
        .bind("same\0after")
        .execute(&pool)
        .await
        .unwrap();
    let after = fingerprint(&pool, "profile_settings", "key,value").await;
    assert_eq!(before.rows, after.rows);
    assert!(
        before.digest != after.digest,
        "same count cannot conceal changed content"
    );
    pool.close().await;
}

#[tokio::test]
async fn preservation_allows_orphan_cleanup_but_detects_deleted_valid_history() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(
        "CREATE TABLE vod_items (id INTEGER PRIMARY KEY);
         CREATE TABLE channels (id INTEGER PRIMARY KEY);
         CREATE TABLE series_episodes (id INTEGER PRIMARY KEY);
         CREATE TABLE watch_history (id INTEGER, media_kind TEXT, media_id INTEGER);
         INSERT INTO vod_items VALUES (1);
         INSERT INTO watch_history VALUES (1, 'vod', 1), (2, 'vod', 2);",
    )
    .execute(&pool)
    .await
    .unwrap();
    let before = fingerprint(&pool, "watch_history", "id,media_kind,media_id").await;
    assert_eq!(before.rows, 1);
    sqlx::query("DELETE FROM watch_history WHERE id=2")
        .execute(&pool)
        .await
        .unwrap();
    verify_preserved(&pool, &[before]).await;
    let before = fingerprint(&pool, "watch_history", "id,media_kind,media_id").await;
    sqlx::query("DELETE FROM watch_history WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    let after = fingerprint(&pool, "watch_history", "id,media_kind,media_id").await;
    assert!(
        before.digest != after.digest,
        "valid history deletion detected"
    );
    pool.close().await;
}

#[tokio::test]
async fn preservation_distinguishes_adjacent_real_values() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE profile_settings (key TEXT, value REAL)")
        .execute(&pool)
        .await
        .unwrap();
    let first = 1.0_f64;
    sqlx::query("INSERT INTO profile_settings VALUES ('precision', ?)")
        .bind(first)
        .execute(&pool)
        .await
        .unwrap();
    let before = fingerprint(&pool, "profile_settings", "key,value").await;
    sqlx::query("UPDATE profile_settings SET value=?")
        .bind(f64::from_bits(first.to_bits() + 1))
        .execute(&pool)
        .await
        .unwrap();
    let after = fingerprint(&pool, "profile_settings", "key,value").await;
    assert!(
        before.digest != after.digest,
        "adjacent real values must not collide"
    );
    pool.close().await;
}
