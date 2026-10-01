//! History reads durable state, including completed and temporarily absent media.
use nova_storage::{init_profile_db, LibraryHistoryRepository, SourceRepository};
use sqlx::SqlitePool;

async fn fixture(pool: &SqlitePool) -> (i64, i64, i64) {
    let a = SourceRepository::new(pool)
        .create("m3u", "A", "https://example.invalid/a")
        .await
        .unwrap()
        .id;
    let b = SourceRepository::new(pool)
        .create("m3u", "B", "https://example.invalid/b")
        .await
        .unwrap()
        .id;
    for (id, kind) in [("film", "movie"), ("show", "series"), ("other", "movie")] {
        sqlx::query(
            "INSERT INTO canonical_entities (id, kind, title, created_at) VALUES (?, ?, ?, 0)",
        )
        .bind(id)
        .bind(kind)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }
    for (entity, source) in [("film", a), ("other", b)] {
        sqlx::query("INSERT INTO stream_variants (id, entity_id, source_id, binding_key, created_at) VALUES (?, ?, ?, ?, 0)")
            .bind(entity).bind(entity).bind(source).bind(entity).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO entity_watch_state (entity_id, position_sec, duration_sec, watched_at, completed) VALUES (?, 900, 900, 300, 1)")
            .bind(entity).execute(pool).await.unwrap();
    }
    // Two provider bindings for a single show must not duplicate episodes.
    for title in ["Show", "Show HD"] {
        sqlx::query("INSERT INTO series_bindings (source_id, binding_key, entity_id, series_title, created_at) VALUES (?, ?, 'show', ?, 0)")
            .bind(a).bind(title).bind(title).execute(pool).await.unwrap();
    }
    for (key, number, at, completed) in [("s1e1", 1, 300, true), ("s1e2", 2, 200, false)] {
        sqlx::query("INSERT INTO episode_watch_state (series_entity_id, episode_key, season, episode, position_sec, duration_sec, watched_at, completed) VALUES ('show', ?, 1, ?, 600, 900, ?, ?)")
            .bind(key).bind(number).bind(at).bind(completed).execute(pool).await.unwrap();
    }
    let episode: i64 = sqlx::query_scalar("INSERT INTO series_episodes (source_id, series_title, season, episode, episode_key, episode_title, stream_url) VALUES (?, 'Show HD', 1, 2, 's1e2', 'Second', 'https://example.invalid/secret') RETURNING id")
        .bind(a).fetch_one(pool).await.unwrap();
    let live: i64 = sqlx::query_scalar("INSERT INTO channels (source_id, name, stream_url) VALUES (?, 'Channel', 'https://example.invalid/channel') RETURNING id")
        .bind(a).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO watch_history (media_kind, media_id, position_sec, watched_at) VALUES ('live', ?, 0, 100)")
        .bind(live).execute(pool).await.unwrap();
    (a, b, episode)
}

#[tokio::test]
async fn history_includes_completed_unavailable_and_each_episode_without_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let (a, b, episode) = fixture(&pool).await;
    let repo = LibraryHistoryRepository::new(&pool);
    let rows = repo.list(a, None, 50, 0).await.unwrap();
    assert_eq!(
        rows.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
        [
            "episode:show:s1e1",
            "movie:film",
            "episode:show:s1e2",
            "live:1"
        ]
    );
    assert!(rows[0].completed);
    assert_eq!(rows[0].media_id, None);
    assert!(rows[1].completed);
    assert_eq!(rows[1].media_id, None);
    assert_eq!(rows[2].media_id, Some(episode));
    assert_eq!(rows[2].position_sec, 600.0);
    assert_eq!(rows[2].episode_title.as_deref(), Some("Second"));
    assert!(rows[2].numbered);
    let json = serde_json::to_string(&rows).unwrap();
    assert!(!json.contains("streamUrl"));
    assert!(!json.contains("secret"));
    assert_eq!(repo.list(b, None, 50, 0).await.unwrap().len(), 1);
    assert!(repo.list(999, None, 50, 0).await.unwrap().is_empty());
    for (kind, count) in [("vod", 1), ("episode", 2), ("live", 1)] {
        assert_eq!(repo.list(a, Some(kind), 50, 0).await.unwrap().len(), count);
    }
    let page1 = repo.list(a, None, 2, 0).await.unwrap();
    let page2 = repo.list(a, None, 2, 2).await.unwrap();
    assert_eq!(
        page1
            .iter()
            .chain(&page2)
            .map(|r| &r.key)
            .collect::<Vec<_>>(),
        rows.iter().map(|r| &r.key).collect::<Vec<_>>()
    );
    assert!(repo.list(a, None, 2, 4).await.unwrap().is_empty());
    pool.close().await;
}

#[tokio::test]
async fn history_reattaches_to_current_episode_without_losing_saved_progress() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let (a, _, old) = fixture(&pool).await;
    sqlx::query("DELETE FROM series_episodes WHERE id = ?")
        .bind(old)
        .execute(&pool)
        .await
        .unwrap();
    let repo = LibraryHistoryRepository::new(&pool);
    let absent = repo.list(a, Some("episode"), 50, 0).await.unwrap();
    assert!(absent.iter().all(|r| r.media_id.is_none()));
    let new: i64 = sqlx::query_scalar("INSERT INTO series_episodes (source_id, series_title, season, episode, episode_key, stream_url) VALUES (?, 'Show', 1, 2, 's1e2', 'https://example.invalid/new') RETURNING id")
        .bind(a).fetch_one(&pool).await.unwrap();
    let restored = repo.list(a, Some("episode"), 50, 0).await.unwrap();
    assert_eq!(restored[1].key, absent[1].key);
    assert_eq!(restored[1].media_id, Some(new));
    assert_eq!(restored[1].position_sec, 600.0);
    assert_eq!(restored[1].watched_at, 200);
    pool.close().await;
}
