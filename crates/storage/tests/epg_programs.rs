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

/// What a feed refresh does: replace the feed's programmes in one transaction.
async fn refresh(pool: &SqlitePool, feed: i64, programs: &[Program]) {
    let mut tx = pool.begin().await.unwrap();
    ProgramsRepository::clear_feed_tx(&mut tx, feed)
        .await
        .unwrap();
    ProgramsRepository::insert_batch_tx(&mut tx, feed, programs)
        .await
        .unwrap();
    let covered: Vec<String> = programs.iter().map(|p| p.channel_tvg_id.clone()).collect();
    ProgramsRepository::clear_unowned_tx(&mut tx, &covered)
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
