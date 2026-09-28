//! TMDB suggestions resolve to the library's own titles only.

use nova_storage::{init_profile_db, CanonicalRepository, SourceRepository, TitleFactsRepository};
use sqlx::SqlitePool;

async fn movie(
    pool: &SqlitePool,
    source: i64,
    entity: &str,
    title: &str,
    tmdb: Option<&str>,
    state: &str,
) {
    sqlx::query("INSERT INTO canonical_entities (id, kind, title, created_at, state) VALUES (?, 'movie', ?, 0, ?)")
        .bind(entity)
        .bind(title)
        .bind(state)
        .execute(pool)
        .await
        .unwrap();
    let vod: i64 = sqlx::query_scalar(
        "INSERT INTO vod_items (source_id, name, stream_url) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(source)
    .bind(title)
    .bind(format!("http://h/movie/u/p/{entity}.mkv"))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO stream_variants (id, entity_id, source_id, binding_key, vod_item_id, created_at)
         VALUES (?, ?, ?, ?, ?, 0)",
    )
    .bind(format!("v-{entity}"))
    .bind(entity)
    .bind(source)
    .bind(format!("k-{entity}"))
    .bind(vod)
    .execute(pool)
    .await
    .unwrap();
    if let Some(id) = tmdb {
        sqlx::query(
            "INSERT INTO entity_external_matches (entity_id, namespace, status, external_id, method, matcher_version, evidence_json, checked_at)
             VALUES (?, 'tmdb:movie', 'matched', ?, 'search', 3, '{}', 0)",
        )
        .bind(entity)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn suggestions_map_to_active_library_titles() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let src = SourceRepository::new(&pool)
        .create("m3u", "a", "https://example.invalid/a")
        .await
        .unwrap()
        .id;
    let other = SourceRepository::new(&pool)
        .create("m3u", "b", "https://example.invalid/b")
        .await
        .unwrap()
        .id;
    movie(
        &pool,
        src,
        "godfather",
        "The Godfather",
        Some("238"),
        "active",
    )
    .await;
    movie(
        &pool,
        src,
        "part-2",
        "The Godfather Part II",
        Some("240"),
        "active",
    )
    .await;
    movie(&pool, src, "merged", "Old duplicate", Some("240"), "merged").await;
    movie(&pool, other, "elsewhere", "Part III", Some("242"), "active").await;
    movie(&pool, src, "unmatched", "Home video", None, "active").await;

    let repo = TitleFactsRepository::new(&pool);
    let mut found = repo.library_movies(&[240, 242, 999]).await.unwrap();
    found.sort_by(|a, b| a.1.as_str().cmp(b.1.as_str()));
    let ids: Vec<(i64, &str)> = found.iter().map(|(t, e)| (*t, e.as_str())).collect();
    assert_eq!(
        ids,
        vec![(242, "elsewhere"), (240, "part-2")],
        "merged entities never suggested"
    );

    // Cards come only from the requested source.
    let entities: Vec<_> = found.into_iter().map(|(_, e)| e).collect();
    let cards = CanonicalRepository::new(&pool)
        .movies_by_ids(src, &entities)
        .await
        .unwrap();
    let titles: Vec<&str> = cards.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, vec!["The Godfather Part II"]);

    sqlx::query(
        "INSERT INTO series_meta (source_id, series_title, tmdb_id, tmdb_status) VALUES
           (?, 'Dark', 70523, 'matched'), (?, 'Lost', 4607, 'unmatched')",
    )
    .bind(src)
    .bind(src)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        repo.library_series(src, &[4607, 70523]).await.unwrap(),
        vec![(70523, "Dark".to_string())],
        "only confident matches"
    );
    assert!(repo
        .library_series(other, &[70523])
        .await
        .unwrap()
        .is_empty());
    assert!(repo.library_movies(&[]).await.unwrap().is_empty());
    pool.close().await;
}
