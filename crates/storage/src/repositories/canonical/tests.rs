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
        provider_tmdb_id: None,
        evidence: Default::default(),
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
    let movies_rail = WatchHistoryRepository::new(&db.pool)
        .continue_watching_section(nova_core::ContinueSection::Movies, db.source_id, 10)
        .await
        .unwrap();
    assert_eq!(movies_rail.len(), 1);
    assert_eq!(movies_rail[0].entity_id.as_deref(), Some(entity.as_str()));
    assert_eq!(movies_rail[0].media_id, new_row);
    assert_eq!(movies_rail[0].title, "Heat");
    assert!(WatchHistoryRepository::new(&db.pool)
        .continue_watching_section(nova_core::ContinueSection::Movies, db.source_id + 1, 10)
        .await
        .unwrap()
        .is_empty());
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
    assert!(WatchHistoryRepository::new(&db.pool)
        .continue_watching_section(nova_core::ContinueSection::Movies, db.source_id, 10)
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
    // Same title/year, different provider items: the compiler never merges.
    // Consolidation is the separate, evidence-checked resolution step.
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
    let started = std::time::Instant::now();
    let report = resolve(&db).await;
    let resolution = started.elapsed();
    let started = std::time::Instant::now();
    let mut tx = db.pool.begin().await.unwrap();
    crate::PublicationRepository::measure_tx(&mut tx, db.source_id)
        .await
        .unwrap();
    crate::PublicationRepository::candidate_report_tx(&mut tx, db.source_id)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    eprintln!("publish validation {:?}", started.elapsed());
    let started = std::time::Instant::now();
    let mut tx = db.pool.begin().await.unwrap();
    crate::CatalogSearchRepository::rebuild_movies_tx(&mut tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let rebuild = started.elapsed();
    let started = std::time::Instant::now();
    let hits = crate::CatalogSearchRepository::new(&db.pool)
        .movie_candidates(db.source_id, "\"film\"* \"2999\"*", 300)
        .await
        .unwrap();
    eprintln!(
        "search index rebuild {rebuild:?}, query {:?} ({} hits)",
        started.elapsed(),
        hits.len()
    );
    eprintln!("full refresh (delete+insert+compile) {refresh:?}, page {list:?} ({}), entity lookup {lookup:?}, reopen backfill {reopen:?}, resolution {resolution:?} ({report:?})", page.len());
}

// ── Entity resolution ───────────────────────────────────────────────────────

async fn second_source(db: &Db) -> i64 {
    SourceRepository::new(&db.pool)
        .create(
            "xtream",
            "Backup",
            "http://other/get.php?username=u&password=p",
        )
        .await
        .unwrap()
        .id
}

async fn import_to(db: &Db, source_id: i64, rows: &[VodItem], replace: bool) {
    let mut tx = db.pool.begin().await.unwrap();
    if replace {
        WatchHistoryRepository::delete_for_source_media_tx(&mut tx, source_id)
            .await
            .unwrap();
        VodRepository::delete_by_source_tx(&mut tx, source_id)
            .await
            .unwrap();
    }
    VodRepository::insert_batch_tx(&mut tx, rows).await.unwrap();
    CanonicalRepository::compile_movies_tx(&mut tx, source_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn resolve(db: &Db) -> ResolutionReport {
    let mut tx = db.pool.begin().await.unwrap();
    let report = CanonicalRepository::resolve_movies_tx(&mut tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    report
}

async fn movies_in(db: &Db, source_id: i64) -> Vec<CanonicalMediaSummary> {
    CanonicalRepository::new(&db.pool)
        .list_movies(
            source_id,
            100,
            0,
            &MovieFilters::default(),
            ContentSort::Default,
        )
        .await
        .unwrap()
}

/// Entities are created within the same second in tests; pin creation order
/// so the survivor (oldest) is predictable.
async fn set_created_at(db: &Db, entity: &EntityId, at: i64) {
    sqlx::query("UPDATE canonical_entities SET created_at = ? WHERE id = ?")
        .bind(at)
        .bind(entity.as_str())
        .execute(&db.pool)
        .await
        .unwrap();
}

async fn state_of(db: &Db, entity: &EntityId) -> String {
    sqlx::query_scalar("SELECT state FROM canonical_entities WHERE id = ?")
        .bind(entity.as_str())
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

fn tmdb(mut item: VodItem, id: i64) -> VodItem {
    item.provider_tmdb_id = Some(id);
    item
}

#[tokio::test]
async fn the_entity_title_does_not_depend_on_which_source_refreshed_last() {
    let db = db().await;
    let backup = second_source(&db).await;
    let caps = |source: i64, url: &str| {
        let mut m = movie(source, "HEAT", url, Some("xtream:vod:1"));
        m.clean_title = Some("HEAT".into());
        m
    };
    let mixed = |source: i64, url: &str| movie(source, "Heat", url, Some("xtream:vod:1"));
    import(&db, &[caps(db.source_id, "http://host/1.mkv")], false).await;
    import_to(&db, backup, &[mixed(backup, "http://other/1.mkv")], false).await;
    // Pin the ALL-CAPS copy's entity as the survivor (the hard case: its own
    // compile only ever saw "HEAT").
    let caps_entity = movies_in(&db, db.source_id).await[0].entity_id.clone();
    set_created_at(&db, &caps_entity, 0).await;
    resolve(&db).await;
    let title = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT title FROM canonical_entities WHERE state = 'active'",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap()
    };
    assert_eq!(
        title().await,
        vec!["Heat".to_string()],
        "one entity, mixed case preferred"
    );
    // Refresh each source in turn: the title stays put (no last-writer-wins).
    import(&db, &[caps(db.source_id, "http://host/1.mkv")], true).await;
    assert_eq!(title().await, vec!["Heat".to_string()]);
    import_to(&db, backup, &[mixed(backup, "http://other/1.mkv")], true).await;
    assert_eq!(title().await, vec!["Heat".to_string()]);
    // Both sources list it under the entity title, not their own copy's name.
    for source in [db.source_id, backup] {
        assert_eq!(movies_in(&db, source).await[0].title, "Heat");
    }
}

#[tokio::test]
async fn cross_source_copies_merge_and_user_state_is_combined() {
    let db = db().await;
    let backup = second_source(&db).await;
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
    import_to(
        &db,
        backup,
        &[movie(
            backup,
            "Heat FHD",
            "http://other/9.mkv",
            Some("xtream:vod:9"),
        )],
        false,
    )
    .await;
    let primary = movies(&db).await[0].entity_id.clone();
    let copy = movies_in(&db, backup).await[0].entity_id.clone();
    assert_ne!(primary, copy);
    set_created_at(&db, &primary, 100).await;
    set_created_at(&db, &copy, 200).await;

    let repo = CanonicalRepository::new(&db.pool);
    repo.set_favorite(&primary, true).await.unwrap();
    repo.record_progress(&primary, 100.0, Some(6000.0), false, None)
        .await
        .unwrap();
    repo.set_in_watchlist(&copy, true).await.unwrap();
    repo.record_progress(&copy, 2400.0, Some(6000.0), false, Some("source"))
        .await
        .unwrap();
    // Make the copy's watch unambiguously the latest.
    sqlx::query("UPDATE entity_watch_state SET watched_at = watched_at + 60 WHERE entity_id = ?")
        .bind(copy.as_str())
        .execute(&db.pool)
        .await
        .unwrap();

    let report = resolve(&db).await;
    assert_eq!(report.applied, 1);
    assert_eq!(state_of(&db, &copy).await, "merged");

    // One entity, listed in both sources, with combined flags and the latest resume.
    for source in [db.source_id, backup] {
        let list = movies_in(&db, source).await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].entity_id, primary);
        assert!(list[0].is_favorite && list[0].is_in_watchlist);
    }
    let detail = repo.movie_detail(&primary).await.unwrap().unwrap();
    assert_eq!(detail.variants.len(), 2);
    assert_eq!(detail.resume.unwrap().position_sec, 2400.0);

    // Historical ID resolves deterministically to the survivor everywhere.
    assert_eq!(
        repo.canonical_entity_id(&copy).await.unwrap(),
        Some(primary.clone())
    );
    let via_old = repo.movie_detail(&copy).await.unwrap().unwrap();
    assert_eq!(via_old.summary.entity_id, primary);
    assert_eq!(
        repo.resume(&copy).await.unwrap().unwrap().position_sec,
        2400.0
    );
    let target = repo.resolve_playback(&copy, None).await.unwrap().unwrap();
    assert_eq!(target.entity_id, primary);
    assert!(repo.set_favorite(&copy, false).await.unwrap());
    assert!(
        !repo
            .movie_detail(&primary)
            .await
            .unwrap()
            .unwrap()
            .summary
            .is_favorite
    );
    repo.record_progress(&copy, 3000.0, None, false, None)
        .await
        .unwrap();
    assert_eq!(
        repo.resume(&primary).await.unwrap().unwrap().position_sec,
        3000.0
    );

    // The legacy row mirror follows the survivor for both sources' rows.
    let wl: Vec<i64> = sqlx::query_scalar("SELECT is_in_watchlist FROM vod_items ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(wl, vec![1, 1]);

    // Lineage records the rule and both sides' prior state.
    let lineage = repo.lineage(&copy).await.unwrap();
    assert_eq!(lineage.len(), 1);
    let entry = &lineage[0];
    assert_eq!(
        (
            entry.operation.as_str(),
            entry.actor.as_str(),
            entry.rule.as_deref()
        ),
        ("merge", "resolver", Some("title_year"))
    );
    assert_eq!((&entry.entity_a, &entry.entity_b), (&primary, &copy));
    let prior: serde_json::Value = serde_json::from_str(&entry.prior_state_json).unwrap();
    assert_eq!(prior["survivor"]["isFavorite"], true);
    assert_eq!(prior["absorbed"]["isInWatchlist"], true);
    assert_eq!(prior["absorbed"]["watch"]["positionSec"], 2400.0);
    assert!(!entry.prior_state_json.contains("http"));
    assert!(!entry.evidence_json.contains("Heat"));

    // Re-resolving is a no-op.
    assert_eq!(resolve(&db).await.applied, 0);
}

#[tokio::test]
async fn hard_contradictions_and_missing_evidence_prevent_merges() {
    let db = db().await;
    let mut remake = movie(
        db.source_id,
        "Scarface",
        "http://host/2.mkv",
        Some("xtream:vod:2"),
    );
    remake.year = Some(1983);
    let mut original = movie(
        db.source_id,
        "Scarface",
        "http://host/1.mkv",
        Some("xtream:vod:1"),
    );
    original.year = Some(1932);
    let mut undated = movie(
        db.source_id,
        "Alien",
        "http://host/5.mkv",
        Some("xtream:vod:5"),
    );
    undated.year = None;
    import(
        &db,
        &[
            original,
            remake,
            tmdb(
                movie(
                    db.source_id,
                    "Heat",
                    "http://host/3.mkv",
                    Some("xtream:vod:3"),
                ),
                949,
            ),
            tmdb(
                movie(
                    db.source_id,
                    "Heat",
                    "http://host/4.mkv",
                    Some("xtream:vod:4"),
                ),
                1_234,
            ),
            undated,
            movie(
                db.source_id,
                "Alien",
                "http://host/6.mkv",
                Some("xtream:vod:6"),
            ),
        ],
        false,
    )
    .await;
    let report = resolve(&db).await;
    assert_eq!(report.applied, 0, "{report:?}");
    assert_eq!(
        report.stats.rejected, 2,
        "remake years + conflicting TMDB IDs"
    );
    assert_eq!(movies(&db).await.len(), 6);
}

#[tokio::test]
async fn a_relisted_item_rejoins_its_entity_with_favorites_and_completion() {
    let db = db().await;
    import(
        &db,
        &[movie(
            db.source_id,
            "Heat",
            "http://host/7.mkv",
            Some("xtream:vod:7"),
        )],
        false,
    )
    .await;
    let entity = movies(&db).await[0].entity_id.clone();
    set_created_at(&db, &entity, 100).await;
    let repo = CanonicalRepository::new(&db.pool);
    repo.set_favorite(&entity, true).await.unwrap();
    repo.set_in_watchlist(&entity, true).await.unwrap();
    repo.record_progress(&entity, 5990.0, Some(6000.0), true, None)
        .await
        .unwrap();

    // The provider renumbers the item: new stream ID, new URL. The compiler
    // mints a new entity; resolution folds it back into the original.
    import(
        &db,
        &[movie(
            db.source_id,
            "Heat",
            "http://host/70.mkv",
            Some("xtream:vod:70"),
        )],
        true,
    )
    .await;
    assert_eq!(resolve(&db).await.applied, 1);
    let list = movies(&db).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].entity_id, entity, "identity survives the relisting");
    assert!(list[0].is_favorite && list[0].is_in_watchlist);
    let resume = repo.resume(&entity).await.unwrap().unwrap();
    assert!(resume.completed);
    let target = repo.resolve_playback(&entity, None).await.unwrap().unwrap();
    assert_eq!(target.media_id, vod_id(&db, "http://host/70.mkv").await);
}

#[tokio::test]
async fn trusted_ids_outlive_their_rows_and_keep_blocking_false_merges() {
    let db = db().await;
    import(
        &db,
        &[tmdb(
            movie(
                db.source_id,
                "Heat",
                "http://host/1.mkv",
                Some("xtream:vod:1"),
            ),
            949,
        )],
        false,
    )
    .await;
    let original = movies(&db).await[0].entity_id.clone();
    resolve(&db).await;
    // The row disappears; a different film with the same title/year and a
    // conflicting TMDB ID arrives. The durable alias still separates them.
    import(
        &db,
        &[tmdb(
            movie(
                db.source_id,
                "Heat",
                "http://host/2.mkv",
                Some("xtream:vod:2"),
            ),
            1_234,
        )],
        true,
    )
    .await;
    let report = resolve(&db).await;
    assert_eq!(report.applied, 0);
    assert_eq!(state_of(&db, &original).await, "active");
    let aliases: Vec<String> = sqlx::query_scalar(
        "SELECT value FROM entity_aliases WHERE alias_kind = 'external' ORDER BY value",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(aliases, vec!["tmdb:movie:1234", "tmdb:movie:949"]);

    // A later copy carrying the original ID merges into the original.
    import(
        &db,
        &[tmdb(
            movie(
                db.source_id,
                "Heat (Director's Cut)",
                "http://host/3.mkv",
                Some("xtream:vod:3"),
            ),
            949,
        )],
        true,
    )
    .await;
    let report = resolve(&db).await;
    assert_eq!(report.applied, 1);
    let lineage = CanonicalRepository::new(&db.pool)
        .lineage(&original)
        .await
        .unwrap();
    assert_eq!(lineage.last().unwrap().rule.as_deref(), Some("trusted_id"));
}

#[tokio::test]
async fn one_card_per_entity_when_a_source_lists_duplicates() {
    let db = db().await;
    import(
        &db,
        &[
            movie(
                db.source_id,
                "Heat FHD",
                "http://host/1.mkv",
                Some("xtream:vod:1"),
            ),
            movie(
                db.source_id,
                "Heat 4K",
                "http://host/2.mkv",
                Some("xtream:vod:2"),
            ),
            movie(
                db.source_id,
                "Alien",
                "http://host/3.mkv",
                Some("xtream:vod:3"),
            ),
        ],
        false,
    )
    .await;
    assert_eq!(resolve(&db).await.applied, 1);
    let list = movies(&db).await;
    assert_eq!(list.len(), 2);
    let repo = CanonicalRepository::new(&db.pool);
    assert_eq!(
        repo.count_movies(db.source_id, &MovieFilters::default())
            .await
            .unwrap(),
        2
    );
    let heat = list.iter().find(|m| m.title.starts_with("Heat")).unwrap();
    let detail = repo.movie_detail(&heat.entity_id).await.unwrap().unwrap();
    assert_eq!(detail.variants.len(), 2);
    // Both variants stay individually playable.
    for variant in &detail.variants {
        let target = repo
            .resolve_playback(&heat.entity_id, Some(&variant.variant_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(target.variant_id, variant.variant_id);
    }
}

#[tokio::test]
async fn redirect_chains_compress_to_the_final_survivor() {
    let db = db().await;
    let backup = second_source(&db).await;
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
    let first: Vec<EntityId> = movies(&db).await.into_iter().map(|m| m.entity_id).collect();
    set_created_at(&db, &first[0], 200).await;
    set_created_at(&db, &first[1], 300).await;
    resolve(&db).await;
    let (a, b) = (first[0].clone(), first[1].clone());

    // An even older identity appears later (e.g. restored from another
    // source) and absorbs the previous survivor.
    import_to(
        &db,
        backup,
        &[movie(
            backup,
            "Heat",
            "http://other/1.mkv",
            Some("xtream:vod:1"),
        )],
        false,
    )
    .await;
    let z = movies_in(&db, backup).await[0].entity_id.clone();
    set_created_at(&db, &z, 50).await;
    assert_eq!(resolve(&db).await.applied, 1);

    let repo = CanonicalRepository::new(&db.pool);
    for old in [&a, &b] {
        assert_eq!(
            repo.canonical_entity_id(old).await.unwrap(),
            Some(z.clone())
        );
        let direct: String =
            sqlx::query_scalar("SELECT new_id FROM entity_redirects WHERE old_id = ?")
                .bind(old.as_str())
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(direct, z.as_str(), "chains are compressed to one hop");
    }
    assert_eq!(movies(&db).await.len(), 1);
    assert_eq!(movies(&db).await[0].entity_id, z);
}

#[tokio::test]
async fn corrupt_redirect_cycles_are_rejected_not_followed_forever() {
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
    let ids: Vec<EntityId> = movies(&db).await.into_iter().map(|m| m.entity_id).collect();
    let (a, b) = (ids[0].as_str(), ids[1].as_str());
    let op: i64 = sqlx::query_scalar(
        "INSERT INTO entity_lineage (operation, entity_a, entity_b, actor, evidence_json, prior_state_json, created_at)
         VALUES ('merge', ?, ?, 'resolver', '{}', '{}', 0) RETURNING id",
    )
    .bind(a)
    .bind(b)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    for (old, new) in [(a, b), (b, a)] {
        sqlx::query("UPDATE canonical_entities SET state = 'merged' WHERE id = ?")
            .bind(old)
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO entity_redirects (old_id, new_id, operation_id, created_at) VALUES (?, ?, ?, 0)")
            .bind(old)
            .bind(new)
            .bind(op)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    let repo = CanonicalRepository::new(&db.pool);
    assert!(matches!(
        repo.canonical_entity_id(&ids[0]).await,
        Err(StorageError::InvalidCanonicalRecord)
    ));
    assert_eq!(
        repo.canonical_entity_id(&EntityId::new("mov-unknown").unwrap())
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn cannot_link_blocks_resolution_and_follows_merges() {
    let db = db().await;
    let backup = second_source(&db).await;
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
    let ids: Vec<EntityId> = movies(&db).await.into_iter().map(|m| m.entity_id).collect();
    let repo = CanonicalRepository::new(&db.pool);
    assert!(repo.record_cannot_link(&ids[0], &ids[1]).await.unwrap());
    assert!(!repo.record_cannot_link(&ids[0], &ids[0]).await.unwrap());
    let report = resolve(&db).await;
    assert_eq!(report.applied, 0);
    assert_eq!(movies(&db).await.len(), 2);
    let lineage = repo.lineage(&ids[0]).await.unwrap();
    assert_eq!(
        (lineage[0].operation.as_str(), lineage[0].actor.as_str()),
        ("cannot_link", "user")
    );

    // A third copy matches both, but they are declared distinct: it is
    // ambiguous and merges with neither.
    import_to(
        &db,
        backup,
        &[movie(
            backup,
            "Heat",
            "http://other/1.mkv",
            Some("xtream:vod:1"),
        )],
        false,
    )
    .await;
    assert_eq!(resolve(&db).await.applied, 0);
}

#[tokio::test]
async fn lineage_is_append_only() {
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
                "Heat",
                "http://host/2.mkv",
                Some("xtream:vod:2"),
            ),
        ],
        false,
    )
    .await;
    resolve(&db).await;
    for sql in [
        "UPDATE entity_lineage SET rule = 'forged'",
        "DELETE FROM entity_lineage",
    ] {
        let err = sqlx::query(sql).execute(&db.pool).await.unwrap_err();
        assert!(err.to_string().contains("append-only"), "{err}");
    }
}

#[tokio::test]
async fn open_time_resolution_runs_once_per_ruleset() {
    let db = db().await;
    let repo = CanonicalRepository::new(&db.pool);
    // init_profile_db already resolved the (empty) profile.
    assert!(repo.ensure_movies_resolved().await.unwrap().is_none());
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
    // A profile last resolved by an older rule set (or before resolution
    // existed) is resolved on the next open.
    sqlx::query("DELETE FROM entity_resolution_state")
        .execute(&db.pool)
        .await
        .unwrap();
    let report = repo.ensure_movies_resolved().await.unwrap().unwrap();
    assert_eq!(report.applied, 1);
    assert!(repo.ensure_movies_resolved().await.unwrap().is_none());
}
