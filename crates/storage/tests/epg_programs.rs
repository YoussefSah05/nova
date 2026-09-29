//! EPG programmes belong to the feed that supplied them (M24): feeds covering
//! the same channel never overwrite each other, reads prefer the viewing
//! source's feeds, and deleting a source removes its programmes.

use nova_epg::Program;
use nova_storage::{init_profile_db, EpgSourcesRepository, ProgramsRepository, SourceRepository};
use sqlx::SqlitePool;

fn program(channel: &str, title: &str, start: i64, stop: i64) -> Program {
    Program {
        channel_tvg_id: channel.into(),
        title: title.into(),
        sub_title: None,
        description: None,
        start_utc: start,
        stop_utc: stop,
        category: None,
        icon_url: None,
    }
}

/// A source with one registered feed; returns `(source_id, feed_id)`.
async fn source_with_feed(pool: &SqlitePool, name: &str) -> (i64, i64) {
    let source = SourceRepository::new(pool)
        .create("xtream", name, &format!("http://{name}/get.php"))
        .await
        .unwrap()
        .id;
    let feeds = EpgSourcesRepository::new(pool);
    feeds
        .upsert(source, &format!("http://{name}/xmltv.php"), "xtream")
        .await
        .unwrap();
    let feed = feeds.list_for_source(source).await.unwrap()[0].id;
    (source, feed)
}

/// What a feed refresh does: stage the programmes, then publish them.
async fn refresh(pool: &SqlitePool, feed: i64, programs: &[Program]) {
    let repo = ProgramsRepository::new(pool);
    repo.clear_staged(feed).await.unwrap();
    repo.stage_batch(feed, programs).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    ProgramsRepository::publish_staged_tx(&mut tx, feed)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn feeds_covering_one_channel_never_overwrite_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let (a, feed_a) = source_with_feed(&pool, "a").await;
    let (b, feed_b) = source_with_feed(&pool, "b").await;
    refresh(&pool, feed_a, &[program("cnn.us", "A news", 0, 100)]).await;
    refresh(&pool, feed_b, &[program("cnn.us", "B news", 0, 100)]).await;
    // A refreshes again: B's programme for the same channel survives.
    refresh(&pool, feed_a, &[program("cnn.us", "A news 2", 0, 100)]).await;

    let programs = ProgramsRepository::new(&pool);
    let now = |source| {
        let programs = &programs;
        async move {
            programs
                .now_and_next("cnn.us", 50, source)
                .await
                .unwrap()
                .now
                .unwrap()
                .title
        }
    };
    assert_eq!(now(Some(a)).await, "A news 2");
    assert_eq!(now(Some(b)).await, "B news");
    let grid: Vec<String> = programs
        .range(&["cnn.us".into()], 0, 100, Some(b))
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect();
    assert_eq!(
        grid,
        ["B news"],
        "the viewing source's feed only, no duplicates"
    );

    // Deleting a source removes its feeds' programmes (and only theirs).
    SourceRepository::new(&pool).delete(a).await.unwrap();
    assert_eq!(now(None).await, "B news");
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM programs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 1);
}

#[tokio::test]
async fn programmes_written_before_feeds_owned_them_are_replaced_per_channel() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let (_, feed) = source_with_feed(&pool, "a").await;
    for (channel, title) in [("cnn.us", "old cnn"), ("bbc.uk", "old bbc")] {
        sqlx::query(
            "INSERT INTO programs (channel_tvg_id, title, start_utc, stop_utc) VALUES (?, ?, 0, 100)",
        )
        .bind(channel)
        .bind(title)
        .execute(&pool)
        .await
        .unwrap();
    }
    refresh(&pool, feed, &[program("cnn.us", "new cnn", 0, 100)]).await;
    let titles: Vec<String> = sqlx::query_scalar("SELECT title FROM programs ORDER BY title")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        titles,
        ["new cnn", "old bbc"],
        "a channel the feed doesn't cover keeps its legacy guide"
    );
}

#[tokio::test]
async fn a_channel_shows_one_feeds_guide_even_within_a_source() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let (a, provider) = source_with_feed(&pool, "a").await;
    let feeds = EpgSourcesRepository::new(&pool);
    feeds
        .upsert(a, "http://community/guide.xml", "community")
        .await
        .unwrap();
    let community = feeds.list_for_source(a).await.unwrap()[1].id;
    // The community feed covers the channel with shifted slots.
    refresh(
        &pool,
        community,
        &[
            program("cnn.us", "C 20:15", 75, 135),
            program("cnn.us", "C 21:15", 135, 200),
        ],
    )
    .await;
    refresh(
        &pool,
        provider,
        &[
            program("cnn.us", "P 20:00", 60, 120),
            program("cnn.us", "P 21:00", 120, 180),
        ],
    )
    .await;

    let programs = ProgramsRepository::new(&pool);
    for prefer in [Some(a), None] {
        let nn = programs.now_and_next("cnn.us", 70, prefer).await.unwrap();
        assert_eq!(
            (nn.now.unwrap().title, nn.next.unwrap().title),
            ("P 20:00".to_string(), "P 21:00".to_string()),
            "now and next come from the same feed ({prefer:?})"
        );
        let grid: Vec<String> = programs
            .range(&["cnn.us".into()], 0, 300, prefer)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.title)
            .collect();
        assert_eq!(
            grid,
            ["P 20:00", "P 21:00"],
            "no other feed's rows ({prefer:?})"
        );
    }
}

#[tokio::test]
async fn channels_read_their_own_sources_guide_and_feeds_report_status() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let (a, feed_a) = source_with_feed(&pool, "a").await;
    let (b, feed_b) = source_with_feed(&pool, "b").await;
    refresh(&pool, feed_a, &[program("cnn.us", "A news", 0, 100)]).await;
    refresh(
        &pool,
        feed_b,
        &[
            program("cnn.us", "B news", 0, 100),
            program("cnn.us", "B late", 100, 200),
        ],
    )
    .await;
    let channel = |source: i64, tvg: Option<&'static str>| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO channels (source_id, tvg_id, name, stream_url) VALUES (?, ?, 'CNN', 'http://s/1')")
                .bind(source)
                .bind(tvg)
                .execute(&pool)
                .await
                .unwrap()
                .last_insert_rowid()
        }
    };
    let on_a = channel(a, Some("cnn.us")).await;
    let on_b = channel(b, Some("cnn.us")).await;
    let untagged = channel(a, None).await;

    let guides = ProgramsRepository::new(&pool)
        .now_and_next_for_channels(&[on_b, on_a, untagged, 9_999], 50)
        .await
        .unwrap();
    let summary: Vec<(i64, Option<String>, Option<String>)> = guides
        .into_iter()
        .map(|g| {
            (
                g.channel_id,
                g.now.map(|p| p.title),
                g.next.map(|p| p.title),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (on_b, Some("B news".into()), Some("B late".into())),
            (on_a, Some("A news".into()), None),
            (untagged, None, None),
            (9_999, None, None),
        ],
        "each channel reads its own source's guide, in request order"
    );

    let status = EpgSourcesRepository::new(&pool)
        .feed_status(b)
        .await
        .unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(
        (status[0].kind.as_str(), status[0].programme_count),
        ("xtream", 2)
    );
    let json = serde_json::to_string(&status).unwrap();
    assert!(
        !json.contains("http"),
        "feed URLs never leave storage: {json}"
    );
}

#[tokio::test]
async fn the_guide_window_reads_each_channels_feed_and_the_overview_counts_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let (a, feed_a) = source_with_feed(&pool, "a").await;
    let (_b, feed_b) = source_with_feed(&pool, "b").await;
    refresh(
        &pool,
        feed_a,
        &[
            program("cnn.us", "Early", 0, 100),
            program("cnn.us", "Evening", 100, 200),
            program("cnn.us", "Night", 200, 300),
        ],
    )
    .await;
    // Another source's feed covers a channel source A lists too.
    refresh(&pool, feed_b, &[program("bbc.uk", "B only", 0, 400)]).await;
    let channel = |tvg: Option<&'static str>, category: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO channels (source_id, tvg_id, name, category, stream_url)
                 VALUES (?, ?, 'Ch', ?, 'http://s/1')",
            )
            .bind(a)
            .bind(tvg)
            .bind(category)
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid()
        }
    };
    let cnn = channel(Some("cnn.us"), "News").await;
    let bbc = channel(Some("bbc.uk"), "News").await;
    let bare = channel(None, "Sports").await;

    let guide = ProgramsRepository::new(&pool)
        .guide_for_channels(&[bbc, cnn, bare], 150, 250)
        .await
        .unwrap();
    let titles: Vec<(i64, Vec<String>)> = guide
        .into_iter()
        .map(|g| {
            (
                g.channel_id,
                g.programs.into_iter().map(|p| p.title).collect(),
            )
        })
        .collect();
    assert_eq!(
        titles,
        [
            (bbc, vec!["B only".to_string()]),
            (cnn, vec!["Evening".to_string(), "Night".to_string()]),
            (bare, vec![]),
        ],
        "overlapping programmes, in request order, any feed when the source has none"
    );

    let overview = nova_storage::ChannelRepository::new(&pool)
        .live_overview(a, 150)
        .await
        .unwrap();
    assert_eq!((overview.channels, overview.with_guide), (3, 2));
    let later = nova_storage::ChannelRepository::new(&pool)
        .live_overview(a, 350)
        .await
        .unwrap();
    assert_eq!(
        later.with_guide, 1,
        "a guide that has run out is not guide data"
    );
    assert_eq!(
        overview
            .categories
            .iter()
            .map(|c| (c.name.as_str(), c.count))
            .collect::<Vec<_>>(),
        [("News", 2), ("Sports", 1)]
    );
}

#[tokio::test]
async fn now_is_the_latest_starting_programme_on_air_even_when_programmes_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let (a, feed) = source_with_feed(&pool, "a").await;
    refresh(
        &pool,
        feed,
        &[
            program("cnn.us", "Filler", 0, 86_400),
            program("cnn.us", "A", 500, 2000),
            program("cnn.us", "B", 800, 1500),
            program("cnn.us", "C", 900, 1100),
            program("cnn.us", "Next", 1100, 1200),
            program("cnn.us", "Later", 1200, 1300),
        ],
    )
    .await;
    let id = sqlx::query(
        "INSERT INTO channels (source_id, tvg_id, name, stream_url) VALUES (?, 'cnn.us', 'CNN', 'http://s/1')",
    )
    .bind(a)
    .execute(&pool)
    .await
    .unwrap()
    .last_insert_rowid();
    let guide = ProgramsRepository::new(&pool)
        .now_and_next_for_channels(&[id, id], 1000)
        .await
        .unwrap();
    for g in &guide {
        assert_eq!(
            (
                g.now.as_ref().map(|p| p.title.as_str()),
                g.next.as_ref().map(|p| p.title.as_str())
            ),
            (Some("C"), Some("Next"))
        );
    }
}
