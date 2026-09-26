use super::*;
use crate::{init_profile_db, SourceRepository, VodRepository, WatchHistoryRepository};
use nova_core::VodItem;

struct Db {
    _dir: tempfile::TempDir,
    pool: SqlitePool,
    source_id: i64,
}

async fn db() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let source_id = SourceRepository::new(&pool)
        .create(
            "xtream",
            "Provider",
            "http://host/get.php?username=u&password=p",
        )
        .await
        .unwrap()
        .id;
    Db {
        _dir: dir,
        pool,
        source_id,
    }
}

fn movie(source_id: i64, name: &str, url: &str, upstream: Option<&str>) -> VodItem {
    VodItem {
        id: 0,
        source_id,
        name: name.into(),
        clean_title: Some(name.into()),
        year: Some(1999),
        logo_url: None,
        stream_url: url.into(),
        category: Some("Drama".into()),
        is_favorite: false,
        is_in_watchlist: false,
        added_at: None,
        dedup_group: None,
        canonical_title: None,
        poster_url: None,
        backdrop_url: None,
        overview: None,
        upstream_id: upstream.map(str::to_string),
    }
}

/// Mirrors `run_import`: optional replace of the source's rows, insert, then
/// compile, all in one transaction.
async fn import(db: &Db, rows: &[VodItem], replace: bool) -> CompileStats {
    let mut tx = db.pool.begin().await.unwrap();
    if replace {
        WatchHistoryRepository::delete_for_source_media_tx(&mut tx, db.source_id)
            .await
            .unwrap();
        VodRepository::delete_by_source_tx(&mut tx, db.source_id)
            .await
            .unwrap();
    }
    VodRepository::insert_batch_tx(&mut tx, rows).await.unwrap();
    let stats = CanonicalRepository::compile_movies_tx(&mut tx, db.source_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    stats
}

async fn movies(db: &Db) -> Vec<CanonicalMediaSummary> {
    CanonicalRepository::new(&db.pool)
        .list_movies(
            db.source_id,
            100,
            0,
            &MovieFilters::default(),
            ContentSort::Default,
        )
        .await
        .unwrap()
}

async fn vod_id(db: &Db, url: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM vod_items WHERE stream_url = ?")
        .bind(url)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn import_creates_one_entity_and_variant_per_source_record() {
    let db = db().await;
    let stats = import(
        &db,
        &[
            movie(
                db.source_id,
                "Heat",
                "http://host/movie/u/p/1.mkv",
                Some("xtream:vod:1"),
            ),
            movie(
                db.source_id,
                "Alien",
                "http://host/movie/u/p/2.mkv",
                Some("xtream:vod:2"),
            ),
        ],
        false,
    )
    .await;
    assert_eq!(stats.created, 2);
    let list = movies(&db).await;
    assert_eq!(list.len(), 2);
    assert!(list
        .iter()
        .all(|m| m.entity_id.as_str().starts_with("mov-")));
    // Recompiling is a no-op.
    let mut tx = db.pool.begin().await.unwrap();
    let again = CanonicalRepository::compile_movies_tx(&mut tx, db.source_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(again, CompileStats::default());
    // No credential-bearing locator is stored in canonical tables.
    let keys: Vec<String> = sqlx::query_scalar("SELECT binding_key FROM stream_variants")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert!(keys
        .iter()
        .all(|k| k.starts_with("u1-") && !k.contains("host")));
}

#[tokio::test]
async fn user_state_survives_refresh_and_credential_rotation() {
    let db = db().await;
    let url = "http://host/movie/u/p/7.mkv";
    import(
        &db,
        &[movie(db.source_id, "Heat", url, Some("xtream:vod:7"))],
        false,
    )
    .await;
    let entity = movies(&db).await[0].entity_id.clone();
    let repo = CanonicalRepository::new(&db.pool);
    assert!(repo.set_favorite(&entity, true).await.unwrap());
    assert!(repo.set_in_watchlist(&entity, true).await.unwrap());
    repo.record_progress(&entity, 1234.5, Some(6000.0), false, Some("source"))
        .await
        .unwrap();
    let old_row = vod_id(&db, url).await;

    // Refresh with rotated credentials: new row ID, new URL, same upstream ID.
    // (SQLite may reuse a deleted rowid; a new title listed first makes the
    // movie's row ID actually change, as it does in real provider refreshes.)
    let rotated = "http://host/movie/u/NEW/7.mkv";
    let stats = import(
        &db,
        &[
            movie(
                db.source_id,
                "New arrival",
                "http://host/movie/u/NEW/8.mkv",
                Some("xtream:vod:8"),
            ),
            movie(db.source_id, "Heat", rotated, Some("xtream:vod:7")),
        ],
        true,
    )
    .await;
    assert_eq!((stats.rebound, stats.created), (1, 1));
    let new_row = vod_id(&db, rotated).await;
    assert_ne!(old_row, new_row);

    let list = movies(&db).await;
    assert_eq!(list.len(), 2);
    let heat = list.iter().find(|m| m.title == "Heat").unwrap();
    assert_eq!(heat.entity_id, entity, "identity survives refresh");
    assert!(heat.is_favorite && heat.is_in_watchlist);
    let arrival = list.iter().find(|m| m.title == "New arrival").unwrap();
    assert!(
        !arrival.is_favorite,
        "state never leaks onto a reused row ID"
    );
    let resume = repo.resume(&entity).await.unwrap().unwrap();
    assert_eq!(resume.position_sec, 1234.5);
    assert!(!resume.completed);
    // Legacy mirror follows the entity after the refresh.
    let (fav, wl): (i64, i64) =
        sqlx::query_as("SELECT is_favorite, is_in_watchlist FROM vod_items WHERE id = ?")
            .bind(new_row)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((fav, wl), (1, 1));
    // Playback resolves to the new row; Continue Watching follows the entity.
    let target = repo.resolve_playback(&entity, None).await.unwrap().unwrap();
    assert_eq!(
        (target.media_kind.as_str(), target.media_id),
        ("vod", new_row)
    );
    let continue_watching = WatchHistoryRepository::new(&db.pool)
        .list_continue_watching(10)
        .await
        .unwrap();
    assert_eq!(continue_watching.len(), 1);
    assert_eq!(continue_watching[0].media_id, new_row);
}

#[tokio::test]
async fn completion_is_preserved_and_hidden_from_continue_watching() {
    let db = db().await;
    import(
        &db,
        &[movie(
            db.source_id,
            "Heat",
            "http://host/a.mkv",
            Some("xtream:vod:1"),
        )],
        false,
    )
    .await;
    let entity = movies(&db).await[0].entity_id.clone();
    let repo = CanonicalRepository::new(&db.pool);
    repo.record_progress(&entity, 5990.0, Some(6000.0), true, None)
        .await
        .unwrap();
    import(
        &db,
        &[movie(
            db.source_id,
            "Heat",
            "http://host/a.mkv",
            Some("xtream:vod:1"),
        )],
        true,
    )
    .await;
    assert!(repo.resume(&entity).await.unwrap().unwrap().completed);
    assert!(WatchHistoryRepository::new(&db.pool)
        .list_continue_watching(10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn removed_upstream_item_keeps_its_entity_and_state_until_it_returns() {
    let db = db().await;
    let heat = movie(
        db.source_id,
        "Heat",
        "http://host/1.mkv",
        Some("xtream:vod:1"),
    );
    let alien = movie(
        db.source_id,
        "Alien",
        "http://host/2.mkv",
        Some("xtream:vod:2"),
    );
    import(&db, &[heat.clone(), alien.clone()], false).await;
    let heat_entity = movies(&db)
        .await
        .into_iter()
        .find(|m| m.title == "Heat")
        .unwrap()
        .entity_id;
    let repo = CanonicalRepository::new(&db.pool);
    repo.set_favorite(&heat_entity, true).await.unwrap();

    // Provider temporarily drops Heat.
    import(&db, std::slice::from_ref(&alien), true).await;
    assert_eq!(movies(&db).await.len(), 1);
    let detail = repo.movie_detail(&heat_entity).await.unwrap().unwrap();
    assert!(
        detail.summary.is_favorite,
        "state retained while unavailable"
    );
    assert_eq!(detail.summary.title, "Heat");
    assert!(detail.variants.iter().all(|v| !v.available));
    assert!(repo
        .resolve_playback(&heat_entity, None)
        .await
        .unwrap()
        .is_none());

    // It comes back: same entity, still a favourite.
    import(&db, &[heat, alien], true).await;
    let list = movies(&db).await;
    let back = list.iter().find(|m| m.title == "Heat").unwrap();
    assert_eq!(back.entity_id, heat_entity);
    assert!(back.is_favorite);
}

#[tokio::test]
async fn locator_bindings_upgrade_to_stable_ids_without_losing_identity() {
    let db = db().await;
    let url = "http://host/movie/u/p/9.mkv";
    // A record compiled before a stable ID was known (e.g. upgraded install).
    import(&db, &[movie(db.source_id, "Heat", url, None)], false).await;
    let entity = movies(&db).await[0].entity_id.clone();
    CanonicalRepository::new(&db.pool)
        .set_in_watchlist(&entity, true)
        .await
        .unwrap();

    // Next refresh carries the Xtream ID: the variant is upgraded in place.
    let stats = import(
        &db,
        &[movie(db.source_id, "Heat", url, Some("xtream:vod:9"))],
        true,
    )
    .await;
    assert_eq!((stats.upgraded, stats.created), (1, 0));
    // Afterwards credentials may rotate freely.
    let rotated = "http://host/movie/u/rotated/9.mkv";
    import(
        &db,
        &[movie(db.source_id, "Heat", rotated, Some("xtream:vod:9"))],
        true,
    )
    .await;
    let list = movies(&db).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].entity_id, entity);
    assert!(list[0].is_in_watchlist);
}

#[tokio::test]
async fn duplicate_records_of_one_provider_item_share_an_entity() {
    let db = db().await;
    let url = "http://host/dup.mkv";
    let stats = import(
        &db,
        &[
            movie(db.source_id, "Heat", url, None),
            movie(db.source_id, "Heat copy", url, None),
        ],
        false,
    )
    .await;
    assert_eq!(stats.created, 1);
    assert_eq!(movies(&db).await.len(), 1);
    let repo = CanonicalRepository::new(&db.pool);
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM vod_items ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    let first = repo.entity_for_vod(ids[0]).await.unwrap().unwrap();
    let second = repo.entity_for_vod(ids[1]).await.unwrap().unwrap();
    assert_eq!(
        first, second,
        "an unbound duplicate still resolves its entity"
    );
    // The legacy mirror covers the unbound duplicate as well.
    repo.set_favorite(&first, true).await.unwrap();
    let flags: Vec<i64> = sqlx::query_scalar("SELECT is_favorite FROM vod_items ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(flags, [1, 1]);
    import(
        &db,
        &[
            movie(db.source_id, "Heat", url, None),
            movie(db.source_id, "Heat copy", url, None),
        ],
        true,
    )
    .await;
    let flags: Vec<i64> = sqlx::query_scalar("SELECT is_favorite FROM vod_items ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        flags,
        [1, 1],
        "mirror re-derived for every duplicate after refresh"
    );
}

#[tokio::test]
async fn many_variants_rebind_to_their_own_records_in_one_compile() {
    let db = db().await;
    let rows: Vec<_> = (0..6)
        .map(|i| {
            movie(
                db.source_id,
                &format!("Film {i}"),
                &format!("http://host/{i}.mkv"),
                Some(&format!("xtream:vod:{i}")),
            )
        })
        .collect();
    import(&db, &rows, false).await;
    let before: std::collections::HashMap<String, EntityId> = movies(&db)
        .await
        .into_iter()
        .map(|m| (m.title, m.entity_id))
        .collect();
    // Refresh in reverse order with a duplicate row of every item: all six
    // variants are unbound and re-bound by the same UPDATE statement.
    let mut refreshed = Vec::new();
    for row in rows.iter().rev() {
        refreshed.push(row.clone());
        refreshed.push(VodItem {
            name: format!("{} copy", row.name),
            ..row.clone()
        });
    }
    let stats = import(&db, &refreshed, true).await;
    assert_eq!((stats.rebound, stats.created), (6, 0));
    let bound: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT vod_item_id) FROM stream_variants")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(bound, 6);
    for summary in movies(&db).await {
        let title = summary.title.trim_end_matches(" copy").to_string();
        assert_eq!(before[&title], summary.entity_id, "{title} kept its entity");
    }
}

#[tokio::test]
async fn distinct_titles_are_never_merged_by_the_compiler() {
    let db = db().await;
    // Same title/year, different provider items: resolution is out of scope,
    // so the compiler must not guess (false merges are worse than dupes).
    import(
        &db,
        &[
            movie(
                db.source_id,
                "Heat",
                "http://host/1.mkv",
                Some("xtream:vod:1"),
            ),
            movie(
                db.source_id,
                "Heat",
                "http://host/2.mkv",
                Some("xtream:vod:2"),
            ),
        ],
        false,
    )
    .await;
    let list = movies(&db).await;
    assert_eq!(list.len(), 2);
    assert_ne!(list[0].entity_id, list[1].entity_id);
}

#[tokio::test]
async fn deleting_a_source_keeps_entities_and_their_user_state() {
    let db = db().await;
    import(
        &db,
        &[movie(
            db.source_id,
            "Heat",
            "http://host/1.mkv",
            Some("xtream:vod:1"),
        )],
        false,
    )
    .await;
    let entity = movies(&db).await[0].entity_id.clone();
    let repo = CanonicalRepository::new(&db.pool);
    repo.set_favorite(&entity, true).await.unwrap();
    repo.record_progress(&entity, 60.0, None, false, None)
        .await
        .unwrap();
    SourceRepository::new(&db.pool)
        .delete(db.source_id)
        .await
        .unwrap();
    let variants: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stream_variants")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(variants, 0);
    let (fav,): (i64,) = sqlx::query_as("SELECT is_favorite FROM canonical_entities WHERE id = ?")
        .bind(entity.as_str())
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(fav, 1);
    assert_eq!(
        repo.resume(&entity).await.unwrap().unwrap().position_sec,
        60.0
    );
}

#[tokio::test]
async fn browse_filters_sort_and_count_use_entity_state() {
    let db = db().await;
    let mut rows = Vec::new();
    for i in 0..5 {
        let mut m = movie(
            db.source_id,
            &format!("Film {i}"),
            &format!("http://host/{i}.mkv"),
            Some(&format!("xtream:vod:{i}")),
        );
        m.category = Some(if i % 2 == 0 { "Drama" } else { "Action" }.into());
        rows.push(m);
    }
    import(&db, &rows, false).await;
    let repo = CanonicalRepository::new(&db.pool);
    let all = movies(&db).await;
    assert_eq!(
        all.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(),
        ["Film 0", "Film 1", "Film 2", "Film 3", "Film 4"]
    );
    repo.set_favorite(&all[3].entity_id, true).await.unwrap();
    let favorites = MovieFilters {
        favorites_only: true,
        ..MovieFilters::default()
    };
    let fav = repo
        .list_movies(db.source_id, 10, 0, &favorites, ContentSort::Default)
        .await
        .unwrap();
    assert_eq!(fav.len(), 1);
    assert_eq!(fav[0].title, "Film 3");
    assert_eq!(
        repo.count_movies(db.source_id, &favorites).await.unwrap(),
        1
    );
    let drama = MovieFilters {
        category: Some("Drama"),
        query: Some("film"),
        ..MovieFilters::default()
    };
    assert_eq!(repo.count_movies(db.source_id, &drama).await.unwrap(), 3);
    let page = repo
        .list_movies(
            db.source_id,
            2,
            2,
            &MovieFilters::default(),
            ContentSort::Default,
        )
        .await
        .unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].title, "Film 2");
    let recent = repo
        .list_movies(
            db.source_id,
            1,
            0,
            &MovieFilters::default(),
            ContentSort::RecentlyAdded,
        )
        .await
        .unwrap();
    assert_eq!(recent[0].title, "Film 4", "same added_at: newest row first");
}

#[tokio::test]
async fn playback_honours_a_requested_variant_of_the_same_entity_only() {
    let db = db().await;
    import(
        &db,
        &[
            movie(
                db.source_id,
                "Heat",
                "http://host/1.mkv",
                Some("xtream:vod:1"),
            ),
            movie(
                db.source_id,
                "Alien",
                "http://host/2.mkv",
                Some("xtream:vod:2"),
            ),
        ],
        false,
    )
    .await;
    let list = movies(&db).await;
    let repo = CanonicalRepository::new(&db.pool);
    let heat = repo
        .movie_detail(&list[0].entity_id)
        .await
        .unwrap()
        .unwrap();
    let alien = repo
        .movie_detail(&list[1].entity_id)
        .await
        .unwrap()
        .unwrap();
    let own = &heat.variants[0].variant_id;
    let foreign = &alien.variants[0].variant_id;
    let target = repo
        .resolve_playback(&list[0].entity_id, Some(own))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&target.variant_id, own);
    // A variant of another entity is never played for this entity.
    let target = repo
        .resolve_playback(&list[0].entity_id, Some(foreign))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&target.variant_id, own);
    let unknown = EntityId::new("mov-missing").unwrap();
    assert!(repo.movie_detail(&unknown).await.unwrap().is_none());
    assert!(!repo.set_favorite(&unknown, true).await.unwrap());
}

#[tokio::test]
async fn legacy_row_writes_land_on_the_entity_and_survive_refresh() {
    let db = db().await;
    let url = "http://host/movie/u/p/3.mkv";
    import(
        &db,
        &[movie(db.source_id, "Ran", url, Some("xtream:vod:3"))],
        false,
    )
    .await;
    let row = vod_id(&db, url).await;
    // Unmigrated surfaces (search results, player progress) address the row.
    VodRepository::new(&db.pool)
        .set_favorite(row, true)
        .await
        .unwrap();
    VodRepository::new(&db.pool)
        .set_in_watchlist(row, true)
        .await
        .unwrap();
    let history = WatchHistoryRepository::new(&db.pool);
    history
        .record_progress("vod", row, 77.0, Some(100.0), false, Some("off"))
        .await
        .unwrap();
    let entity = movies(&db).await[0].entity_id.clone();
    let repo = CanonicalRepository::new(&db.pool);
    assert_eq!(
        repo.resume(&entity).await.unwrap().unwrap().position_sec,
        77.0
    );

    let rotated = "http://host/movie/u/new/3.mkv";
    import(
        &db,
        &[
            movie(
                db.source_id,
                "Other",
                "http://host/movie/u/new/4.mkv",
                Some("xtream:vod:4"),
            ),
            movie(db.source_id, "Ran", rotated, Some("xtream:vod:3")),
        ],
        true,
    )
    .await;
    let summary = movies(&db)
        .await
        .into_iter()
        .find(|m| m.entity_id == entity)
        .unwrap();
    assert!(summary.is_favorite && summary.is_in_watchlist);
    // The legacy resume read for the new row resolves through the entity.
    let new_row = vod_id(&db, rotated).await;
    assert_ne!(new_row, row);
    let resume = history
        .resume_position("vod", new_row)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resume.position_sec, 77.0);
    // Legacy unfavourite on the new row updates the entity too.
    VodRepository::new(&db.pool)
        .set_favorite(new_row, false)
        .await
        .unwrap();
    assert!(
        !repo
            .movie_detail(&entity)
            .await
            .unwrap()
            .unwrap()
            .summary
            .is_favorite
    );
}

#[tokio::test]
#[ignore = "scale timing probe; run manually"]
async fn scale_probe_compile_30k() {
    let db = db().await;
    let rows: Vec<_> = (0..30_000)
        .map(|i| {
            movie(
                db.source_id,
                &format!("Film {i}"),
                &format!("http://host/{i}.mkv"),
                Some(&format!("xtream:vod:{i}")),
            )
        })
        .collect();
    let started = std::time::Instant::now();
    let mut tx = db.pool.begin().await.unwrap();
    VodRepository::insert_batch_tx(&mut tx, &rows)
        .await
        .unwrap();
    let insert_only = started.elapsed();
    let started = std::time::Instant::now();
    CanonicalRepository::compile_movies_tx(&mut tx, db.source_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let first = started.elapsed();
    eprintln!("insert only {insert_only:?}, first compile {first:?}");
    let started = std::time::Instant::now();
    import(&db, &rows, true).await;
    let refresh = started.elapsed();
    let repo = CanonicalRepository::new(&db.pool);
    let started = std::time::Instant::now();
    let page = repo
        .list_movies(
            db.source_id,
            200,
            10_000,
            &MovieFilters::default(),
            ContentSort::RecentlyAdded,
        )
        .await
        .unwrap();
    let list = started.elapsed();
    let row = vod_id(&db, "http://host/29999.mkv").await;
    let started = std::time::Instant::now();
    for _ in 0..100 {
        repo.entity_for_vod(row).await.unwrap().unwrap();
    }
    let lookup = started.elapsed() / 100;
    let started = std::time::Instant::now();
    repo.backfill_movies().await.unwrap();
    let reopen = started.elapsed();
    eprintln!("full refresh (delete+insert+compile) {refresh:?}, page {list:?} ({}), entity lookup {lookup:?}, reopen backfill {reopen:?}", page.len());
}
