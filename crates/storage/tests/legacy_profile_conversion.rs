//! Offline converter qualification. Fixtures contain no real profile content.
use nova_storage::profile_recovery as legacy_profile;
// The shared verifier is exercised by real_profile_upgrade; this suite uses
// the converter's public checks and the normal migration runner.

use legacy_profile::{convert, historical_migrator, open, ConversionError};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::path::Path;

async fn fixture(path: &Path) -> SqlitePool {
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    historical_migrator().run(&pool).await.unwrap();
    sqlx::raw_sql(
        "INSERT INTO sources (id,kind,name,url) VALUES (1,'m3u','fixture','https://example.invalid/list');
         INSERT INTO vod_items (id,source_id,name,stream_url,is_favorite,is_in_watchlist,director,tmdb_cast,runtime,rating)
           VALUES (1,1,'Fixture Film','https://example.invalid/movie',1,1,'director','cast',90,8.5);
         INSERT INTO channels (id,source_id,name,stream_url,is_favorite,is_in_watchlist)
           VALUES (1,1,'Fixture Channel','https://example.invalid/live',1,1);
         INSERT INTO series_episodes (id,source_id,series_title,season,episode,stream_url,is_favorite,is_in_watchlist)
           VALUES (1,1,'Fixture Series',1,1,'https://example.invalid/episode',1,1);
         INSERT INTO series_meta (source_id,series_title,director,tmdb_cast,runtime,rating)
           VALUES (1,'Fixture Series','director','cast',45,9);
         INSERT INTO watch_history (media_kind,media_id,position_sec,duration_sec,watched_at,completed,caption_mode)
           VALUES ('vod',1,42,100,10,0,'source'), ('episode',1,24,200,20,0,'source'), ('live',1,4,NULL,30,0,'off'),
                  ('vod',999,1,100,1,0,'off');
         INSERT INTO caption_preferences (scope,scope_key,caption_mode,updated_at) VALUES ('vod','1','source',10);
         INSERT INTO profile_settings (key,value) VALUES ('fixture','saved');
         INSERT INTO transcripts (id,media_kind,media_id,content_key,language,model,vtt_path,json_path,created_at)
           VALUES (1,'vod',1,'fixture-content','en','fixture','','',1);
         INSERT INTO transcript_segments (id,transcript_id,seq,start_sec,end_sec,text) VALUES (1,1,0,0,2,'fixture dialogue');
         INSERT INTO transcript_windows (id,transcript_id,start_sec,end_sec) VALUES (1,1,0,2);
         INSERT INTO transcript_translations (segment_id,target_lang,backend,text,created_at) VALUES (1,'fr','fixture','bonjour',1);
         INSERT INTO llm_about VALUES ('movie','1','fixture','en','cached fixture',1);"
    ).execute(&pool).await.unwrap();
    pool.close().await;
    open(path, false).await.unwrap()
}

async fn file_digest(path: &Path) -> Vec<u8> {
    Sha256::digest(tokio::fs::read(path).await.unwrap()).to_vec()
}

#[tokio::test]
async fn converts_known_branch_preserves_state_and_reopens_without_changing_original() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let output = dir.path().join("output.db");
    fixture(&source).await.close().await;
    let before = file_digest(&source).await;
    convert(&source, &output).await.unwrap();
    assert!(
        before == file_digest(&source).await,
        "source bytes unchanged"
    );
    let pool = nova_storage::init_profile_db(output.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        // Converted to this build's head, whatever that is.
        sqlx::migrate!("./migrations/profile")
            .iter()
            .map(|m| m.version)
            .max()
            .unwrap()
    );
    for (sql, expected) in [
        ("SELECT COUNT(*) FROM nova_legacy_migration_ledger_v1", 21),
        ("SELECT COUNT(*) FROM canonical_entities WHERE is_favorite=1 AND is_in_watchlist=1", 2),
        ("SELECT COUNT(*) FROM entity_watch_state WHERE position_sec=42 AND caption_mode='source'", 1),
        ("SELECT COUNT(*) FROM episode_watch_state WHERE position_sec=24 AND caption_mode='source'", 1),
        ("SELECT COUNT(*) FROM transcript_fts WHERE text MATCH 'dialogue'", 1),
        ("SELECT COUNT(*) FROM transcript_translations WHERE text='bonjour'", 1),
        ("SELECT COUNT(*) FROM llm_about WHERE text='cached fixture'", 1),
        ("SELECT COUNT(*) FROM watch_history", 3),
        ("SELECT media_id FROM transcripts", -1),
    ] {
        assert_eq!(sqlx::query_scalar::<_, i64>(sql).fetch_one(&pool).await.unwrap(), expected);
    }
    let historical = historical_migrator();
    let archived: Vec<u8> =
        sqlx::query_scalar("SELECT checksum FROM nova_legacy_migration_ledger_v1 WHERE version=20")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        archived.as_slice()
            == historical
                .iter()
                .find(|m| m.version == 20)
                .unwrap()
                .checksum
                .as_ref()
    );
    pool.close().await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn rejects_unknown_dirty_incomplete_newer_and_changed_schema_without_publishing() {
    for alteration in [
        "UPDATE _sqlx_migrations SET checksum=X'00' WHERE version=6",
        "UPDATE _sqlx_migrations SET success=0 WHERE version=20",
        "DELETE FROM _sqlx_migrations WHERE version=10",
        "UPDATE _sqlx_migrations SET version=22 WHERE version=21",
        "ALTER TABLE vod_items ADD COLUMN unexpected TEXT",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        let output = dir.path().join("output.db");
        let pool = fixture(&source).await;
        sqlx::query(alteration).execute(&pool).await.unwrap();
        pool.close().await;
        let before = file_digest(&source).await;
        assert!(matches!(
            convert(&source, &output).await,
            Err(ConversionError::Lineage | ConversionError::Schema)
        ));
        assert!(!output.exists());
        assert!(
            before == file_digest(&source).await,
            "refused source bytes unchanged"
        );
        assert_eq!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".nova-profile-convert-"))
                .count(),
            0
        );
    }
}

#[tokio::test]
async fn refuses_existing_output_and_source_as_output() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let output = dir.path().join("output.db");
    fixture(&source).await.close().await;
    tokio::fs::write(&output, b"existing output").await.unwrap();
    assert!(matches!(
        convert(&source, &output).await,
        Err(ConversionError::Filesystem)
    ));
    assert_eq!(tokio::fs::read(&output).await.unwrap(), b"existing output");
    let before = file_digest(&source).await;
    assert!(matches!(
        convert(&source, &source).await,
        Err(ConversionError::Filesystem)
    ));
    assert!(before == file_digest(&source).await);
}

#[tokio::test]
async fn snapshot_includes_committed_wal_data() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let output = dir.path().join("output.db");
    let pool = fixture(&source).await;
    sqlx::query("PRAGMA wal_autocheckpoint=0")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE profile_settings SET value='committed in WAL'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(source.with_extension("db-wal").exists());
    convert(&source, &output).await.unwrap();
    pool.close().await;
    let converted = open(&output, true).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM profile_settings WHERE key='fixture'")
            .fetch_one(&converted)
            .await
            .unwrap(),
        "committed in WAL"
    );
    converted.close().await;
}

#[tokio::test]
#[ignore = "requires NOVA_LEGACY_UPGRADE_DB; only a temporary output is converted"]
async fn real_known_branch_converts_on_a_temporary_copy() {
    let source = std::env::var_os("NOVA_LEGACY_UPGRADE_DB").expect("set NOVA_LEGACY_UPGRADE_DB");
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("converted.db");
    let started = std::time::Instant::now();
    convert(Path::new(&source), &output).await.unwrap();
    let pool = open(&output, true).await.unwrap();
    for table in [
        "vod_items",
        "series_episodes",
        "channels",
        "watch_history",
        "transcript_segments",
        "llm_about",
        "canonical_entities",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        eprintln!("{table}: {count}");
    }
    eprintln!("known legacy schema 21 converted through current migrations; full conversion and checks: {:?}", started.elapsed());
    pool.close().await;
}

#[tokio::test]
async fn refuses_destination_sidecars_without_touching_them() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    fixture(&source).await.close().await;
    for suffix in ["-wal", "-shm", "-journal"] {
        let output = dir.path().join(format!("output{suffix}.db"));
        let sidecar = dir.path().join(format!("output{suffix}.db{suffix}"));
        tokio::fs::write(&sidecar, b"untouched sidecar")
            .await
            .unwrap();
        assert!(matches!(
            convert(&source, &output).await,
            Err(ConversionError::Filesystem)
        ));
        assert!(!output.exists());
        assert_eq!(
            tokio::fs::read(&sidecar).await.unwrap(),
            b"untouched sidecar"
        );
    }
    #[cfg(unix)]
    {
        let output = dir.path().join("dangling.db");
        let sidecar = dir.path().join("dangling.db-wal");
        std::os::unix::fs::symlink(dir.path().join("missing"), &sidecar).unwrap();
        assert!(matches!(
            convert(&source, &output).await,
            Err(ConversionError::Filesystem)
        ));
        assert!(std::fs::symlink_metadata(sidecar)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[tokio::test]
async fn semantic_validation_rejects_lost_series_state_and_reset_progress() {
    for corruption in [
        "DELETE FROM series_bindings",
        "UPDATE canonical_entities SET is_favorite=0 WHERE kind='series'",
        "UPDATE canonical_entities SET is_in_watchlist=0 WHERE kind='series'",
        "DELETE FROM episode_watch_state",
        "UPDATE episode_watch_state SET position_sec=0",
        "UPDATE entity_watch_state SET position_sec=0",
        "UPDATE entity_watch_state SET duration_sec=0",
        "UPDATE entity_watch_state SET completed=1",
        "UPDATE entity_watch_state SET caption_mode='off'",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        let output = dir.path().join("output.db");
        fixture(&source).await.close().await;
        convert(&source, &output).await.unwrap();
        let pool = open(&output, false).await.unwrap();
        sqlx::query(corruption).execute(&pool).await.unwrap();
        assert!(matches!(
            legacy_profile::catalog_valid(&pool).await,
            Err(ConversionError::Validation)
        ));
        pool.close().await;
    }
}

#[tokio::test]
async fn duplicate_bindings_keep_latest_movie_and_episode_history() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let output = dir.path().join("output.db");
    let pool = fixture(&source).await;
    sqlx::raw_sql(
        "INSERT INTO vod_items (id,source_id,name,stream_url,is_favorite,is_in_watchlist)
           SELECT 2,source_id,name,stream_url,is_favorite,is_in_watchlist FROM vod_items WHERE id=1;
         INSERT INTO series_episodes (id,source_id,series_title,season,episode,stream_url,is_favorite,is_in_watchlist)
           SELECT 2,source_id,series_title,season,episode,stream_url,is_favorite,is_in_watchlist FROM series_episodes WHERE id=1;
         INSERT INTO watch_history (media_kind,media_id,position_sec,duration_sec,watched_at,completed,caption_mode)
           VALUES ('vod',2,78,120,100,1,'off'), ('episode',2,98,200,100,0,'off');"
    ).execute(&pool).await.unwrap();
    pool.close().await;
    convert(&source, &output).await.unwrap();
    let pool = open(&output, true).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, f64>("SELECT position_sec FROM entity_watch_state")
            .fetch_one(&pool)
            .await
            .unwrap(),
        78.0
    );
    assert_eq!(
        sqlx::query_scalar::<_, f64>("SELECT position_sec FROM episode_watch_state")
            .fetch_one(&pool)
            .await
            .unwrap(),
        98.0
    );
    pool.close().await;
}

#[tokio::test]
async fn recovery_registers_a_separate_profile_only_after_validation() {
    let dir = tempfile::tempdir().unwrap();
    let app = nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
        .await
        .unwrap();
    let repo = nova_storage::ProfileRepository::new(&app);
    let original = repo.create("Fixture", "#e0a33e").await.unwrap();
    let profiles = dir.path().join("profiles");
    let original_dir = profiles.join(&original.id);
    tokio::fs::create_dir_all(&original_dir).await.unwrap();
    let source = original_dir.join("profile.db");
    fixture(&source).await.close().await;
    let before = file_digest(&source).await;
    assert!(legacy_profile::is_supported(&source).await);
    let copy = legacy_profile::recover_as_profile(&app, &profiles, &original.id)
        .await
        .unwrap();
    assert_ne!(copy.id, original.id);
    assert_eq!(copy.name, "Fixture (Recovered)");
    assert_eq!(
        copy.last_used_at, None,
        "recovery does not select or activate"
    );
    assert_eq!(repo.list().await.unwrap().len(), 2);
    assert_eq!(repo.get(&original.id).await.unwrap().last_used_at, None);
    assert!(
        before == file_digest(&source).await,
        "original is retained byte-for-byte"
    );
    nova_storage::init_profile_db(profiles.join(&copy.id).join("profile.db").to_str().unwrap())
        .await
        .unwrap()
        .close()
        .await;
    app.close().await;
}

#[tokio::test]
async fn failed_registration_removes_only_the_new_copy() {
    let dir = tempfile::tempdir().unwrap();
    let app = nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
        .await
        .unwrap();
    let original = nova_storage::ProfileRepository::new(&app)
        .create("Fixture", "#e0a33e")
        .await
        .unwrap();
    let profiles = dir.path().join("profiles");
    let original_dir = profiles.join(&original.id);
    tokio::fs::create_dir_all(&original_dir).await.unwrap();
    let source = original_dir.join("profile.db");
    fixture(&source).await.close().await;
    let before = file_digest(&source).await;
    sqlx::query("CREATE TRIGGER refuse_recovered_registration BEFORE INSERT ON profiles BEGIN SELECT RAISE(ABORT, 'fixture'); END")
        .execute(&app).await.unwrap();
    assert!(
        legacy_profile::recover_as_profile(&app, &profiles, &original.id)
            .await
            .is_err()
    );
    assert!(before == file_digest(&source).await);
    assert_eq!(
        nova_storage::ProfileRepository::new(&app)
            .list()
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        std::fs::read_dir(&profiles)
            .unwrap()
            .flatten()
            .filter(|e| e.file_type().unwrap().is_dir())
            .count(),
        1
    );
    app.close().await;
}

#[tokio::test]
async fn invalid_or_unsupported_recovery_never_registers_a_profile() {
    let dir = tempfile::tempdir().unwrap();
    let app = nova_storage::init_app_db(dir.path().join("app.db").to_str().unwrap())
        .await
        .unwrap();
    let original = nova_storage::ProfileRepository::new(&app)
        .create("Fixture", "#e0a33e")
        .await
        .unwrap();
    let profiles = dir.path().join("profiles");
    let original_dir = profiles.join(&original.id);
    tokio::fs::create_dir_all(&original_dir).await.unwrap();
    let source = original_dir.join("profile.db");
    let pool = fixture(&source).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum=X'00' WHERE version=6")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert!(!legacy_profile::is_supported(&source).await);
    for id in ["../../elsewhere", "not-a-profile", &original.id] {
        assert!(legacy_profile::recover_as_profile(&app, &profiles, id)
            .await
            .is_err());
    }
    assert_eq!(
        nova_storage::ProfileRepository::new(&app)
            .list()
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        std::fs::read_dir(&profiles)
            .unwrap()
            .flatten()
            .filter(|e| e.file_type().unwrap().is_dir())
            .count(),
        1
    );
    app.close().await;
}

#[tokio::test]
async fn merged_duplicates_may_gain_flags_from_their_own_title() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let output = dir.path().join("output.db");
    let pool = fixture(&source).await;
    // A second copy of the same film and episode, without the user's flags:
    // canonical merging legitimately mirrors the title's flags onto it.
    sqlx::raw_sql(
        "INSERT INTO vod_items (id,source_id,name,stream_url,is_favorite,is_in_watchlist)
           SELECT 2,source_id,name,stream_url,0,0 FROM vod_items WHERE id=1;
         INSERT INTO series_episodes (id,source_id,series_title,season,episode,stream_url,is_favorite,is_in_watchlist)
           SELECT 2,source_id,series_title,season,episode,stream_url,0,0 FROM series_episodes WHERE id=1;",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    convert(&source, &output).await.unwrap();
    let pool = open(&output, true).await.unwrap();
    let flags: Vec<(i64, i64)> =
        sqlx::query_as("SELECT is_favorite, is_in_watchlist FROM vod_items ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        flags.iter().all(|f| *f == (1, 1)),
        "the copy carries its title's flags"
    );
    pool.close().await;
}

#[tokio::test]
async fn a_flag_on_a_later_duplicate_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let output = dir.path().join("output.db");
    let pool = fixture(&source).await;
    // The user flagged the later copy, not the lowest-id one.
    sqlx::raw_sql(
        "INSERT INTO vod_items (id,source_id,name,stream_url,is_favorite,is_in_watchlist)
           SELECT 2,source_id,name,stream_url,1,1 FROM vod_items WHERE id=1;
         UPDATE vod_items SET is_favorite = 0, is_in_watchlist = 0 WHERE id = 1;",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    convert(&source, &output).await.unwrap();
    let pool = open(&output, true).await.unwrap();
    let flags: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT is_favorite, is_in_watchlist FROM canonical_entities WHERE kind = 'movie'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(flags, [(1, 1)], "the film keeps the user's flags");
    pool.close().await;
}
