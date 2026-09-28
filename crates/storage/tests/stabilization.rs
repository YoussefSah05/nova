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

#[tokio::test]
async fn upgrade_backfills_canonical_movies_preserving_user_state() {
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
    let mut old: Migrator = sqlx::migrate!("./migrations/profile");
    old.migrations =
        std::borrow::Cow::Owned(old.iter().filter(|m| m.version <= 20).cloned().collect());
    old.run(&pool).await.unwrap();
    let source = SourceRepository::new(&pool)
        .create("m3u", "legacy", "https://example.invalid/list")
        .await
        .unwrap()
        .id;
    // Representative pre-0021 catalog: favourite, watchlisted, resumed,
    // completed and untouched movies, plus a duplicate locator.
    for (id, name, url, fav, wl) in [
        (10, "Heat", "https://example.invalid/heat", 1, 0),
        (11, "Alien", "https://example.invalid/alien", 0, 1),
        (12, "Ran", "https://example.invalid/ran", 0, 0),
        (13, "Up", "https://example.invalid/up", 0, 0),
        (14, "Heat (copy)", "https://example.invalid/heat", 0, 0),
    ] {
        sqlx::query("INSERT INTO vod_items (id, source_id, name, stream_url, is_favorite, is_in_watchlist) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(id).bind(source).bind(name).bind(url).bind(fav).bind(wl)
            .execute(&pool).await.unwrap();
    }
    // Heat's two duplicate rows both have history: the newest must win.
    sqlx::query("INSERT INTO watch_history (media_kind, media_id, position_sec, duration_sec, watched_at, completed) VALUES ('vod', 12, 300, 6000, 5, 0), ('vod', 13, 5400, 5400, 6, 1), ('vod', 10, 100, 6000, 2, 0), ('vod', 14, 900, 6000, 9, 0)")
        .execute(&pool).await.unwrap();
    pool.close().await;

    for _ in 0..2 {
        // Upgrade, then reopen: the data step must be idempotent.
        let upgraded = init_profile_db(path.to_str().unwrap()).await.unwrap();
        let repo = nova_storage::CanonicalRepository::new(&upgraded);
        let movies = repo
            .list_movies(
                source,
                50,
                0,
                &nova_storage::MovieFilters::default(),
                nova_core::ContentSort::Default,
            )
            .await
            .unwrap();
        let titles: Vec<_> = movies.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Heat", "Alien", "Ran", "Up"],
            "duplicate locator collapses"
        );
        let by = |t: &str| movies.iter().find(|m| m.title == t).unwrap();
        assert!(by("Heat").is_favorite && !by("Heat").is_in_watchlist);
        assert!(by("Alien").is_in_watchlist && !by("Alien").is_favorite);
        let ran = repo.resume(&by("Ran").entity_id).await.unwrap().unwrap();
        assert_eq!((ran.position_sec, ran.completed), (300.0, false));
        let up = repo.resume(&by("Up").entity_id).await.unwrap().unwrap();
        assert!(up.completed, "completion preserved");
        let counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM canonical_entities), (SELECT COUNT(*) FROM stream_variants), (SELECT COUNT(*) FROM entity_watch_state)",
        )
        .fetch_one(&upgraded)
        .await
        .unwrap();
        assert_eq!(counts, (4, 4, 3));
        let heat = repo.resume(&by("Heat").entity_id).await.unwrap().unwrap();
        assert_eq!(
            heat.position_sec, 900.0,
            "most recent duplicate history wins"
        );
        // Legacy rows and history are untouched (additive migration).
        let legacy: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM watch_history")
            .fetch_one(&upgraded)
            .await
            .unwrap();
        assert_eq!(legacy, 4);
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&upgraded)
            .await
            .unwrap();
        assert_eq!(integrity, "ok");
        let fk: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(&upgraded)
            .await
            .unwrap();
        assert!(fk.is_empty());
        upgraded.close().await;
    }
}

/// Concurrent SQL migrations are not safe on SQLite (sqlx's migrator takes no
/// lock); the desktop serializes profile opens. The canonical data step must
/// still tolerate concurrent runners: it takes the write lock up front.
#[tokio::test]
async fn concurrent_backfills_of_a_legacy_profile_compile_exactly_once() {
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePool::connect_with(options.clone()).await.unwrap();
    // Schema at head, but rows written the pre-0021 way (no binding keys).
    sqlx::migrate!("./migrations/profile")
        .run(&pool)
        .await
        .unwrap();
    let source = SourceRepository::new(&pool)
        .create("m3u", "legacy", "https://example.invalid/list")
        .await
        .unwrap()
        .id;
    for i in 0..500 {
        sqlx::query(
            "INSERT INTO vod_items (source_id, name, stream_url, is_favorite) VALUES (?, ?, ?, ?)",
        )
        .bind(source)
        .bind(format!("Film {i}"))
        .bind(format!("https://example.invalid/{i}"))
        .bind(i64::from(i % 7 == 0))
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let runners: Vec<_> = (0..4)
        .map(|_| {
            let options = options.clone();
            tokio::spawn(async move {
                let pool = SqlitePool::connect_with(options).await.unwrap();
                let result = nova_storage::CanonicalRepository::new(&pool)
                    .backfill_movies()
                    .await;
                pool.close().await;
                result
            })
        })
        .collect();
    let mut created = 0;
    for runner in runners {
        created += runner
            .await
            .unwrap()
            .expect("every concurrent backfill succeeds")
            .created;
    }
    assert_eq!(created, 500, "each entity created by exactly one runner");
    let upgraded = init_profile_db(path.to_str().unwrap()).await.unwrap();
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM canonical_entities), (SELECT COUNT(*) FROM stream_variants), (SELECT COUNT(*) FROM canonical_entities WHERE is_favorite = 1)",
    )
    .fetch_one(&upgraded)
    .await
    .unwrap();
    assert_eq!(counts, (500, 500, 72));
    upgraded.close().await;
}

/// 0036 rebuilds `canonical_entities` (to admit series) by parking and
/// reloading every child table; existing movie identity state must come
/// through untouched, and existing series gain entities seeded with their
/// flags, keyed by provider series ID where the catalog sentinel carries one.
#[tokio::test]
async fn upgrade_to_canonical_series_keeps_movie_identity_and_seeds_series() {
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
    let mut old: Migrator = sqlx::migrate!("./migrations/profile");
    old.migrations =
        std::borrow::Cow::Owned(old.iter().filter(|m| m.version <= 35).cloned().collect());
    old.run(&pool).await.unwrap();
    let source = SourceRepository::new(&pool)
        .create("xtream", "legacy", "http://example.invalid/get.php")
        .await
        .unwrap()
        .id;
    // Movie identity state in every child table of `canonical_entities`.
    for sql in [
        "INSERT INTO vod_items (id, source_id, name, stream_url, binding_key, locator_key) VALUES (10, 1, 'Heat', 'http://example.invalid/m/1', 'k1', 'k1')",
        "INSERT INTO canonical_entities (id, kind, title, is_favorite, is_in_watchlist, created_at, state) VALUES ('mov-a', 'movie', 'Heat', 1, 1, 1, 'active'), ('mov-b', 'movie', 'Heat', 0, 0, 2, 'merged')",
        "INSERT INTO stream_variants VALUES ('var-1', 'mov-a', 1, 'k1', 10, 1)",
        "INSERT INTO entity_watch_state VALUES ('mov-a', 120.0, 6000.0, 5, 0, 'source')",
        "INSERT INTO entity_lineage (operation, entity_a, entity_b, actor, evidence_json, prior_state_json, created_at) VALUES ('merge', 'mov-a', 'mov-b', 'resolver', '{}', '{}', 3)",
        "INSERT INTO entity_redirects VALUES ('mov-b', 'mov-a', 1, 3)",
        "INSERT INTO entity_aliases VALUES ('mov-a', 'external', 'tmdb:movie:949', 1)",
        "INSERT INTO entity_external_matches (entity_id, namespace, status, external_id, method, matcher_version, evidence_json, canonical_title, checked_at) VALUES ('mov-a', 'tmdb:movie', 'matched', '949', 'trusted_id', 2, '{}', 'Heat', 4)",
        // Series: an unopened Xtream sentinel (favourite), an opened series
        // whose sentinel was replaced by episodes (watchlisted), and one
        // untouched series.
        "INSERT INTO series_episodes (source_id, series_title, season, episode, stream_url, is_favorite, is_in_watchlist) VALUES (1, 'Dark', 0, 0, 'xtream://series/42', 1, 0)",
        "INSERT INTO series_episodes (source_id, series_title, season, episode, stream_url, is_favorite, is_in_watchlist) VALUES (1, 'Lost', 1, 1, 'http://example.invalid/series/u/p/7.mkv', 0, 1), (1, 'Lost', 1, 2, 'http://example.invalid/series/u/p/8.mkv', 0, 1)",
        "INSERT INTO series_episodes (source_id, series_title, season, episode, stream_url) VALUES (1, 'Ozark', 0, 0, 'xtream://series/9')",
        // Series TMDB decisions made before they were entity-owned.
        "INSERT INTO series_meta (source_id, series_title, canonical_title, tmdb_id, tmdb_genres, tmdb_status, match_method, confidence_bp, matcher_version, evidence_json, checked_at) VALUES (1, 'Dark', 'Dark', 70523, 'Drama, Mystery', 'matched', 'trusted_id', 10000, 3, '{}', 7)",
        "INSERT INTO series_meta (source_id, series_title, tmdb_status, match_method, matcher_version, evidence_json, checked_at) VALUES (1, 'Lost', 'unmatched', 'search', 3, '{}', 7)",
        // Episode progress: row-keyed history (Lost S1E1) and history parked
        // by a refresh (Dark, rows not re-fetched yet).
        "INSERT INTO watch_history (media_kind, media_id, position_sec, duration_sec, watched_at, completed) SELECT 'episode', id, 321, 2400, 9, 0 FROM series_episodes WHERE stream_url LIKE '%/7.mkv'",
        "INSERT INTO pending_episode_history (source_id, episode_key, series_title, position_sec, watched_at, completed) VALUES (1, 'u1-parked', 'Dark', 55, 8, 0)",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    assert_eq!(source, 1);
    pool.close().await;

    for _ in 0..2 {
        // Upgrade, then reopen: the data step must be idempotent.
        let upgraded = init_profile_db(path.to_str().unwrap()).await.unwrap();
        let movie: (String, i64, i64, String) = sqlx::query_as(
            "SELECT kind, is_favorite, is_in_watchlist, state FROM canonical_entities WHERE id = 'mov-a'",
        )
        .fetch_one(&upgraded)
        .await
        .unwrap();
        assert_eq!(movie, ("movie".into(), 1, 1, "active".into()));
        let children: (i64, i64, i64, i64, i64, i64, f64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM stream_variants), (SELECT COUNT(*) FROM entity_lineage),
                    (SELECT COUNT(*) FROM entity_redirects), (SELECT COUNT(*) FROM entity_aliases),
                    (SELECT COUNT(*) FROM entity_external_matches WHERE namespace = 'tmdb:movie'), (SELECT COUNT(*) FROM entity_watch_state),
                    (SELECT position_sec FROM entity_watch_state WHERE entity_id = 'mov-a')",
        )
        .fetch_one(&upgraded)
        .await
        .unwrap();
        assert_eq!(
            children,
            (1, 1, 1, 1, 1, 1, 120.0),
            "movie identity state kept"
        );
        let lineage_guard = sqlx::query("DELETE FROM entity_lineage")
            .execute(&upgraded)
            .await;
        assert!(lineage_guard.is_err(), "lineage is still append-only");

        let series: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT title, is_favorite, is_in_watchlist FROM canonical_entities
             WHERE kind = 'series' ORDER BY title",
        )
        .fetch_all(&upgraded)
        .await
        .unwrap();
        assert_eq!(
            series,
            [
                ("Dark".into(), 1, 0),
                ("Lost".into(), 0, 1),
                ("Ozark".into(), 0, 0)
            ],
            "one entity per series, seeded from the row flags"
        );
        let keys: Vec<(String, String)> = sqlx::query_as(
            "SELECT series_title, binding_key FROM series_bindings ORDER BY series_title",
        )
        .fetch_all(&upgraded)
        .await
        .unwrap();
        let key = |title: &str| keys.iter().find(|(t, _)| t == title).unwrap().1.clone();
        assert_eq!(
            key("Dark"),
            nova_core::catalog::media::series_binding_key(Some("42"), "Dark")
        );
        assert_eq!(
            key("Lost"),
            nova_core::catalog::media::series_binding_key(None, "Lost"),
            "an opened series has no sentinel yet: bound by title"
        );
        // Legacy series matches now belong to the bound entities.
        let matches: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT e.title, m.status, m.external_id, m.primary_genre
             FROM entity_external_matches m JOIN canonical_entities e ON e.id = m.entity_id
             WHERE m.namespace = 'tmdb:tv' ORDER BY e.title",
        )
        .fetch_all(&upgraded)
        .await
        .unwrap();
        assert_eq!(
            matches,
            [
                (
                    "Dark".into(),
                    "matched".into(),
                    Some("70523".into()),
                    Some("Drama".into())
                ),
                ("Lost".into(), "unresolved".into(), None, None)
            ]
        );
        // Episode progress is owned by the series entities.
        let episodes: Vec<(String, Option<i64>, Option<i64>, f64)> = sqlx::query_as(
            "SELECT e.title, s.season, s.episode, s.position_sec
             FROM episode_watch_state s JOIN canonical_entities e ON e.id = s.series_entity_id
             ORDER BY e.title",
        )
        .fetch_all(&upgraded)
        .await
        .unwrap();
        assert_eq!(episodes, [("Lost".into(), Some(1), Some(1), 321.0)]);
        // Parked history isn't guessed onto an entity by title (another
        // series may hold it now): its sentinel-only series is queued for the
        // background re-fetch, which re-attaches it through its episode row.
        let history = nova_storage::WatchHistoryRepository::new(&upgraded);
        assert_eq!(
            history.series_with_pending_episodes(1, 10).await.unwrap(),
            vec!["Dark".to_string()]
        );
        let unkeyed: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM series_episodes WHERE episode_key IS NULL")
                .fetch_one(&upgraded)
                .await
                .unwrap();
        assert_eq!(unkeyed, 0);
        // Carried-over decisions are provisional: queued for re-confirmation.
        let queued = nova_storage::SeriesRepository::new(&upgraded)
            .list_for_tmdb(1, 3, i64::MIN, "en-US")
            .await
            .unwrap();
        let mut titles: Vec<_> = queued.iter().map(|q| q.series_title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, ["Dark", "Lost", "Ozark"]);
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&upgraded)
            .await
            .unwrap();
        assert_eq!(integrity, "ok");
        let fk: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(&upgraded)
            .await
            .unwrap();
        assert!(fk.is_empty());
        upgraded.close().await;
    }
}
