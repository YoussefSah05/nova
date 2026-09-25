use nova_storage::{init_profile_db, SourceRepository, TranscriptRepository};
use sqlx::{migrate::Migrator, SqlitePool};

async fn add_catalog(pool: &SqlitePool) -> i64 {
    let id = SourceRepository::new(pool)
        .create("m3u", "fixture", "https://example.invalid/list")
        .await
        .unwrap()
        .id;
    sqlx::query("INSERT INTO channels (id, source_id, name, stream_url) VALUES (1, ?, 'fixture', 'https://example.invalid/live')").bind(id).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO vod_items (id, source_id, name, stream_url) VALUES (1, ?, 'fixture', 'https://example.invalid/movie')").bind(id).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO series_episodes (id, source_id, series_title, season, episode, stream_url) VALUES (1, ?, 'fixture', 1, 1, 'https://example.invalid/episode')").bind(id).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO catalog_embeddings VALUES ('movie', 1, 'test', X'0000803F'), ('live', 1, 'test', X'0000803F')").execute(pool).await.unwrap();
    sqlx::query("INSERT INTO watch_history (media_kind, media_id, position_sec, watched_at) VALUES ('vod', 1, 42, 1), ('live', 1, 42, 1), ('episode', 1, 42, 1)").execute(pool).await.unwrap();
    id
}

#[tokio::test]
async fn deleted_source_does_not_leave_state_for_reused_media_ids() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let id = add_catalog(&pool).await;
    SourceRepository::new(&pool).delete(id).await.unwrap();
    for table in ["catalog_embeddings", "watch_history"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "orphan state in {table}");
    }
    pool.close().await;
}

#[tokio::test]
async fn upgrades_existing_transcripts_and_repairs_catalog_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.db");
    let pool = SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    let mut old = sqlx::migrate!("./migrations/profile");
    old.migrations =
        std::borrow::Cow::Owned(old.iter().filter(|m| m.version <= 9).cloned().collect());
    old.run(&pool).await.unwrap();
    sqlx::query("INSERT INTO transcripts (id, media_kind, media_id, language, model, vtt_path, json_path, created_at) VALUES (1, 'vod', 1, 'en', 'test', '', '', 1)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO transcript_segments (id, transcript_id, seq, start_sec, end_sec, text) VALUES (1, 1, 0, 0, 2, 'migration fixture')").execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO transcript_windows (transcript_id, start_sec, end_sec) VALUES (1, 0, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut before_fix: Migrator = sqlx::migrate!("./migrations/profile");
    before_fix.migrations = std::borrow::Cow::Owned(
        before_fix
            .iter()
            .filter(|m| m.version <= 19)
            .cloned()
            .collect(),
    );
    before_fix.run(&pool).await.unwrap();
    add_catalog(&pool).await;
    // Simulate an orphan created by the old source-deletion path.
    sqlx::query("INSERT INTO watch_history (media_kind, media_id, position_sec, watched_at) VALUES ('vod', 999, 42, 1)").execute(&pool).await.unwrap();
    pool.close().await;
    let upgraded = init_profile_db(path.to_str().unwrap()).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM transcript_segments")
            .fetch_one(&upgraded)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM transcript_windows")
            .fetch_one(&upgraded)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM transcript_fts WHERE text MATCH 'fixture'"
        )
        .fetch_one(&upgraded)
        .await
        .unwrap(),
        1
    );
    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&upgraded)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM catalog_embeddings")
            .fetch_one(&upgraded)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM watch_history")
            .fetch_one(&upgraded)
            .await
            .unwrap(),
        3
    );
    upgraded.close().await;
    // Reopen again: migration checksums and repairs must be repeatable.
    init_profile_db(path.to_str().unwrap())
        .await
        .unwrap()
        .close()
        .await;
}

#[tokio::test]
async fn reused_catalog_id_does_not_relabel_an_old_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let source = add_catalog(&pool).await;
    let repo = TranscriptRepository::new(&pool);
    let old = repo
        .get_or_create_transcript("old-content", "vod", 1, 0, "en", "test", "", "")
        .await
        .unwrap();
    SourceRepository::new(&pool).delete(source).await.unwrap();
    add_catalog(&pool).await;
    let new = repo
        .get_or_create_transcript("new-content", "vod", 1, 0, "en", "test", "", "")
        .await
        .unwrap();
    assert_ne!(old, new);
    let old_again = repo
        .get_or_create_transcript("old-content", "vod", 2, 0, "en", "test", "", "")
        .await
        .unwrap();
    assert_eq!(old, old_again, "stable cache survives catalog deletion");
    pool.close().await;
}
