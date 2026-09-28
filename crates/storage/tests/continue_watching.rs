//! Per-section Continue Watching: series continue or move on, live lists
//! recent channels, each rail stays inside its source.

use nova_core::ContinueSection;
use nova_storage::{init_profile_db, SourceRepository, WatchHistoryRepository};
use sqlx::SqlitePool;

async fn source(pool: &SqlitePool, name: &str) -> i64 {
    SourceRepository::new(pool)
        .create("m3u", name, &format!("https://example.invalid/{name}"))
        .await
        .unwrap()
        .id
}

async fn episode(pool: &SqlitePool, source: i64, series: &str, season: i64, episode: i64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO series_episodes (source_id, series_title, season, episode, episode_title, logo_url, stream_url)
         VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(source)
    .bind(series)
    .bind(season)
    .bind(episode)
    .bind(format!("Episode {episode}"))
    .bind(format!("https://still/{series}/{season}/{episode}.jpg"))
    .bind(format!("https://example.invalid/{series}/{season}/{episode}"))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn watched(pool: &SqlitePool, kind: &str, id: i64, pos: f64, completed: bool, at: i64) {
    sqlx::query(
        "INSERT INTO watch_history (media_kind, media_id, position_sec, duration_sec, watched_at, completed)
         VALUES (?, ?, ?, 2400, ?, ?)",
    )
    .bind(kind)
    .bind(id)
    .bind(pos)
    .bind(at)
    .bind(completed as i64)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn series_rail_shows_one_card_per_show_with_resume_or_up_next() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let src = source(&pool, "a").await;
    let other = source(&pool, "b").await;

    // Dark: S1E1 finished earlier, S1E2 in progress (latest) → resume S1E2.
    let dark1 = episode(&pool, src, "Dark", 1, 1).await;
    let dark2 = episode(&pool, src, "Dark", 1, 2).await;
    episode(&pool, src, "Dark", 1, 3).await;
    watched(&pool, "episode", dark1, 2390.0, true, 100).await;
    watched(&pool, "episode", dark2, 600.0, false, 200).await;
    // Lost: S1E10 finished → up next is S2E1 (next season).
    let lost10 = episode(&pool, src, "Lost", 1, 10).await;
    let lost21 = episode(&pool, src, "Lost", 2, 1).await;
    watched(&pool, "episode", lost10, 2400.0, true, 300).await;
    // Fargo: the final episode finished → the show is done, no card.
    let fargo = episode(&pool, src, "Fargo", 1, 1).await;
    watched(&pool, "episode", fargo, 2400.0, true, 400).await;
    // Another source's show never appears here.
    let elsewhere = episode(&pool, other, "Dark", 1, 1).await;
    watched(&pool, "episode", elsewhere, 60.0, false, 500).await;
    sqlx::query(
        "INSERT INTO series_meta (source_id, series_title, canonical_title, backdrop_url, tmdb_status)
         VALUES (?, 'Lost', 'LOST', 'https://backdrop/lost.jpg', 'matched')",
    )
    .bind(src)
    .execute(&pool)
    .await
    .unwrap();

    let rail = WatchHistoryRepository::new(&pool)
        .continue_watching_section(ContinueSection::Series, src, 10)
        .await
        .unwrap();
    let cards: Vec<(&str, i64, bool)> = rail
        .iter()
        .map(|c| (c.title.as_str(), c.media_id, c.up_next))
        .collect();
    assert_eq!(cards, vec![("LOST", lost21, true), ("Dark", dark2, false)]);

    let lost = &rail[0];
    assert_eq!((lost.season, lost.episode), (Some(2), Some(1)));
    assert_eq!(lost.series_title.as_deref(), Some("Lost"));
    assert_eq!(lost.position_sec, 0.0, "up next starts from the beginning");
    assert_eq!(
        lost.poster_url.as_deref(),
        Some("https://still/Lost/2/1.jpg"),
        "the episode still comes first"
    );
    let dark = &rail[1];
    assert_eq!(dark.position_sec, 600.0);
    assert_eq!(dark.episode_title.as_deref(), Some("Episode 2"));
    assert_eq!(
        dark.numbering,
        Some(nova_core::EpisodeNumberingKind::SeasonEpisode)
    );

    let limited = WatchHistoryRepository::new(&pool)
        .continue_watching_section(ContinueSection::Series, src, 1)
        .await
        .unwrap();
    assert_eq!(limited.len(), 1);
    pool.close().await;
}

#[tokio::test]
async fn live_rail_lists_recent_channels_of_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let src = source(&pool, "a").await;
    let other = source(&pool, "b").await;
    let mut ids = Vec::new();
    for (s, name, logo) in [
        (src, "CNN", Some("https://l/cnn.png")),
        (src, "TF1", None),
        (other, "BBC", None),
    ] {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO channels (source_id, name, logo_url, stream_url) VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(s)
        .bind(name)
        .bind(logo)
        .bind(format!("https://example.invalid/{name}"))
        .fetch_one(&pool)
        .await
        .unwrap();
        ids.push(id);
    }
    watched(&pool, "live", ids[0], 0.0, false, 100).await;
    watched(&pool, "live", ids[1], 0.0, false, 200).await;
    watched(&pool, "live", ids[2], 0.0, false, 300).await;

    let rail = WatchHistoryRepository::new(&pool)
        .continue_watching_section(ContinueSection::Live, src, 10)
        .await
        .unwrap();
    let cards: Vec<(&str, &str, Option<&str>)> = rail
        .iter()
        .map(|c| {
            (
                c.media_kind.as_str(),
                c.title.as_str(),
                c.poster_url.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        cards,
        vec![
            ("live", "TF1", None),
            ("live", "CNN", Some("https://l/cnn.png"))
        ]
    );
    pool.close().await;
}
