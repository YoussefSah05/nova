//! Opt-in timing of the reads the Movies section issues when it opens, on a
//! synthetic catalogue the size of a large real library. Prints timings only.
//!
//!   SQLX_OFFLINE=true cargo test --release -p nova-storage --test movies_browse_scale \
//!     -- --ignored --nocapture
use nova_core::{ContentSort, VodItem};
use nova_storage::repositories::canonical::MovieFilters;
use nova_storage::{init_profile_db, CanonicalRepository, SourceRepository, VodRepository};
use sqlx::SqlitePool;
use std::time::{Duration, Instant};

const FILMS: i64 = 28_600;
const GENRES: [&str; 12] = [
    "Action",
    "Drama",
    "Comedy",
    "Horror",
    "Thriller",
    "Animation",
    "Documentary",
    "Romance",
    "Science Fiction",
    "Crime",
    "Family",
    "Adventure",
];

fn film(source_id: i64, i: i64, version: i64) -> VodItem {
    VodItem {
        id: 0,
        source_id,
        name: format!("Synthetic Film {i}{}", if version > 0 { " 4K" } else { "" }),
        clean_title: Some(format!("Synthetic Film {i}")),
        year: Some(1930 + i % 95),
        logo_url: None,
        stream_url: format!("http://host/movie/u/p/{i}-{version}.mkv"),
        category: Some(GENRES[(i % 12) as usize].into()),
        is_favorite: i % 40 == 0,
        is_in_watchlist: i % 60 == 0,
        added_at: Some(1_700_000_000 + i),
        dedup_group: None,
        canonical_title: None,
        poster_url: Some(format!("https://image.example.invalid/p/{i}.jpg")),
        backdrop_url: (i % 3 == 0).then(|| format!("https://image.example.invalid/b/{i}.jpg")),
        overview: Some("A synthetic overview long enough to resemble real catalogue text.".into()),
        upstream_id: Some(format!("xtream:vod:{i}-{version}")),
        provider_tmdb_id: None,
        evidence: Default::default(),
    }
}

async fn seed() -> (tempfile::TempDir, SqlitePool, i64) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let source = SourceRepository::new(&pool)
        .create(
            "xtream",
            "Synthetic",
            "http://host/get.php?username=u&password=p",
        )
        .await
        .unwrap()
        .id;
    // ~8% of films have a second version, so entities carry several variants.
    let rows: Vec<VodItem> = (1..=FILMS)
        .flat_map(|i| {
            let mut v = vec![film(source, i, 0)];
            if i % 12 == 0 {
                v.push(film(source, i, 1));
            }
            v
        })
        .collect();
    let mut tx = pool.begin().await.unwrap();
    VodRepository::insert_batch_tx(&mut tx, &rows)
        .await
        .unwrap();
    CanonicalRepository::compile_movies_tx(&mut tx, source)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    // Evidence columns the browse reads use (normally written by import/enrichment).
    sqlx::raw_sql(
        "UPDATE vod_items SET
           genre = category,
           poster_url = 'https://image.example.invalid/p/' || id || '.jpg',
           quality_hints = CASE WHEN name LIKE '% 4K' THEN '2160p,hdr' ELSE '1080p' END,
           language_hints = 'en',
           provider_facts = json_object('durationSec', 5400 + id % 3600, 'rating', (id % 90) / 10.0);
         INSERT INTO tmdb_title_facts (media_type, tmdb_id, language, facts, facts_version, fetched_at)
           SELECT 'movie', rowid, 'en',
                  json_object('runtimeMin', 90 + rowid % 60, 'rating', (rowid % 80) / 10.0,
                              'collection', CASE WHEN rowid % 25 = 0
                                THEN json_object('id', rowid / 100, 'name', 'Saga ' || (rowid / 100))
                              END),
                  1, 0
           FROM canonical_entities WHERE kind = 'movie' AND rowid % 10 < 6;
         INSERT INTO entity_external_matches
           (entity_id, namespace, status, external_id, method, confidence_bp, matcher_version,
            evidence_json, canonical_title, genres, checked_at)
           SELECT id, 'tmdb:movie', 'matched', CAST(rowid AS TEXT), 'search', 9000, 1, '{}',
                  title, 'Drama, Crime', 0
           FROM canonical_entities WHERE kind = 'movie' AND rowid % 10 < 6;
         UPDATE canonical_entities SET is_in_watchlist = (rowid % 60 = 0),
                                       is_favorite = (rowid % 40 = 0)
           WHERE kind = 'movie';",
    )
    .execute(&pool)
    .await
    .unwrap();
    (dir, pool, source)
}

async fn timed<T, F: std::future::Future<Output = T>>(label: &str, f: F) -> (T, Duration) {
    let start = Instant::now();
    let out = f.await;
    let took = start.elapsed();
    println!("{label:<34} {:>8.1} ms", took.as_secs_f64() * 1000.0);
    (out, took)
}

#[tokio::test]
#[ignore = "opt-in scale timing; run with --release -- --ignored --nocapture"]
async fn movies_open_reads_at_library_scale() {
    let (dir, pool, source) = seed().await;
    let repo = CanonicalRepository::new(&pool);
    let all = MovieFilters::default();
    let watchlist = MovieFilters {
        watchlist_only: true,
        ..Default::default()
    };
    println!("--- each read alone (warm) ---");
    for _ in 0..2 {
        let _ = repo.count_movies(source, &all).await.unwrap();
    }
    let (n, _) = timed("count_movies", repo.count_movies(source, &all)).await;
    timed(
        "list_movies recent (grid page)",
        repo.list_movies(source, 60, 0, &all, ContentSort::RecentlyAdded),
    )
    .await
    .0
    .unwrap();
    timed(
        "list_movies watchlist (spotlight)",
        repo.list_movies(source, 20, 0, &watchlist, ContentSort::RecentlyAdded),
    )
    .await
    .0
    .unwrap();
    timed("movie_genres", repo.movie_genres(source))
        .await
        .0
        .unwrap();
    timed("movie_decades", repo.movie_decades(source))
        .await
        .0
        .unwrap();
    timed("library_collections", repo.library_collections(source))
        .await
        .0
        .unwrap();
    println!("--- as the screen issues them (concurrent, pool of 4) ---");
    let ((a, b, grid, spot, recent, genres, decades, collections), _) =
        timed("all Movies-open reads", async {
            tokio::join!(
                repo.count_movies(source, &all),
                repo.count_movies(source, &all),
                repo.list_movies(source, 60, 0, &all, ContentSort::RecentlyAdded),
                repo.list_movies(source, 20, 0, &watchlist, ContentSort::RecentlyAdded),
                repo.list_movies(source, 20, 0, &all, ContentSort::RecentlyAdded),
                repo.movie_genres(source),
                repo.movie_decades(source),
                repo.library_collections(source),
            )
        })
        .await;
    // Every concurrent read succeeds and returns real content.
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(grid.unwrap().len(), 60);
    assert!(!spot.unwrap().is_empty() && !recent.unwrap().is_empty());
    assert_eq!(genres.unwrap().len(), GENRES.len());
    assert!(!decades.unwrap().is_empty() && !collections.unwrap().is_empty());
    // Separately named second versions compile as their own entities.
    assert!(n.unwrap() >= FILMS);
    let path = dir.path().join("profile.db");
    println!(
        "database size: {} MB",
        std::fs::metadata(&path).unwrap().len() / 1_000_000
    );
    // Baseline: identical pool without the memory map (SQLite's default).
    let baseline = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .pragma("mmap_size", "0"),
        )
        .await
        .unwrap();
    let repo = CanonicalRepository::new(&baseline);
    let _ = repo.count_movies(source, &all).await.unwrap();
    timed("baseline without mmap (concurrent)", async {
        let (a, ..) = tokio::join!(
            repo.count_movies(source, &all),
            repo.count_movies(source, &all),
            repo.list_movies(source, 60, 0, &all, ContentSort::RecentlyAdded),
            repo.list_movies(source, 20, 0, &watchlist, ContentSort::RecentlyAdded),
            repo.list_movies(source, 20, 0, &all, ContentSort::RecentlyAdded),
            repo.movie_genres(source),
            repo.movie_decades(source),
            repo.library_collections(source),
        );
        a.unwrap()
    })
    .await;
    baseline.close().await;
}
