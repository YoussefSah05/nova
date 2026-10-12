//! XMLTV channel icons as a channel-logo fallback.

use nova_core::ContentSort;
use nova_storage::{init_profile_db, ChannelRepository, EpgSourcesRepository, SourceRepository};
use sqlx::SqlitePool;

async fn insert_channels(pool: &SqlitePool, source_id: i64) {
    for (tvg, name, logo, group) in [
        ("BBC1", "BBC One", None, None),
        ("cnn.us", "CNN", Some("https://provider/cnn.png"), None),
        ("", "No Id", None, None),
        ("bein1", "beIN Sports 1", None, Some("bein sports")),
        (
            "bein2",
            "beIN Sports 2",
            Some("https://provider/bein2.png"),
            Some("bein sports"),
        ),
    ] {
        sqlx::query(
            "INSERT INTO channels (source_id, tvg_id, name, logo_url, stream_url, channel_group)
             VALUES (?, NULLIF(?, ''), ?, ?, ?, ?)",
        )
        .bind(source_id)
        .bind(tvg)
        .bind(name)
        .bind(logo)
        .bind(format!("https://example.invalid/{name}"))
        .bind(group)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn logos(pool: &SqlitePool, source_id: i64) -> Vec<(String, Option<String>)> {
    let mut out: Vec<_> = ChannelRepository::new(pool)
        .list(
            source_id,
            100,
            0,
            None,
            None,
            None,
            None,
            false,
            false,
            ContentSort::Default,
        )
        .await
        .unwrap()
        .into_iter()
        .map(|c| (c.name, c.logo_url))
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn epg_icons_fill_missing_logos_and_survive_a_channel_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let source = SourceRepository::new(&pool)
        .create("m3u", "fixture", "https://example.invalid/list")
        .await
        .unwrap()
        .id;
    insert_channels(&pool, source).await;
    let epg = EpgSourcesRepository::new(&pool);
    epg.upsert(source, "https://example.invalid/epg.xml", "community")
        .await
        .unwrap();
    let epg_id = epg.list_for_source(source).await.unwrap()[0].id;

    let icons = [
        nova_epg::ChannelIcon {
            channel_id: "bbc1.uk".into(),
            icon_url: "https://epg/bbc1.png".into(),
        },
        nova_epg::ChannelIcon {
            channel_id: "CNN.us".into(),
            icon_url: "https://epg/cnn.png".into(),
        },
        nova_epg::ChannelIcon {
            channel_id: "bein1".into(),
            icon_url: "https://epg/bein1.png".into(),
        },
    ];
    let matched =
        nova_epg::match_channel_icons(&epg.channel_tvg_ids(source).await.unwrap(), &icons);
    let mut tx = pool.begin().await.unwrap();
    EpgSourcesRepository::replace_channel_logos_tx(&mut tx, epg_id, source, &matched)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let expected = vec![
        (
            "BBC One".to_string(),
            Some("https://epg/bbc1.png".to_string()),
        ),
        // A provider logo is never replaced by the EPG one.
        (
            "CNN".to_string(),
            Some("https://provider/cnn.png".to_string()),
        ),
        ("No Id".to_string(), None),
        (
            "beIN Sports 1".to_string(),
            Some("https://epg/bein1.png".to_string()),
        ),
        (
            "beIN Sports 2".to_string(),
            Some("https://provider/bein2.png".to_string()),
        ),
    ];
    assert_eq!(logos(&pool, source).await, expected);

    // A bundle prefers a member's provider logo over another member's EPG logo.
    let groups = ChannelRepository::new(&pool)
        .list_groups(source, None, None, None, false, false, 100, 0)
        .await
        .unwrap();
    let bein = groups.iter().find(|g| g.group == "bein sports").unwrap();
    assert_eq!(bein.logo_url.as_deref(), Some("https://provider/bein2.png"));
    let bbc = groups.iter().find(|g| g.group == "bbc one").unwrap();
    assert_eq!(bbc.logo_url.as_deref(), Some("https://epg/bbc1.png"));

    // A catalog refresh rebuilds channel rows; the logos are keyed by tvg-id.
    sqlx::query("DELETE FROM channels WHERE source_id = ?")
        .bind(source)
        .execute(&pool)
        .await
        .unwrap();
    insert_channels(&pool, source).await;
    assert_eq!(logos(&pool, source).await, expected);

    // Re-running the feed replaces its own logos only.
    let mut tx = pool.begin().await.unwrap();
    EpgSourcesRepository::replace_channel_logos_tx(&mut tx, epg_id, source, &[])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        logos(&pool, source).await[0].1,
        None,
        "BBC One lost its EPG logo"
    );
    pool.close().await;
}

#[tokio::test]
async fn directory_logos_are_the_last_fallback() {
    use nova_storage::{ChannelLogoRepository, DirectoryLogoWrite};
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let source = SourceRepository::new(&pool)
        .create("m3u", "fixture", "https://example.invalid/list")
        .await
        .unwrap()
        .id;
    insert_channels(&pool, source).await;
    let epg = EpgSourcesRepository::new(&pool);
    epg.upsert(source, "https://example.invalid/epg.xml", "community")
        .await
        .unwrap();
    let epg_id = epg.list_for_source(source).await.unwrap()[0].id;
    let mut tx = pool.begin().await.unwrap();
    EpgSourcesRepository::replace_channel_logos_tx(
        &mut tx,
        epg_id,
        source,
        &[("bbc1".into(), "https://epg/bbc1.png".into())],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let logos_repo = ChannelLogoRepository::new(&pool);
    let mut candidates: Vec<String> = logos_repo
        .candidates(source)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    candidates.sort();
    assert_eq!(
        candidates,
        vec!["BBC One", "No Id", "beIN Sports 1"],
        "only channels without a provider logo"
    );

    let write = |key: &str, url: &str| DirectoryLogoWrite {
        channel_key: key.into(),
        directory_id: "X.us".into(),
        url: url.into(),
        method: "name",
        confidence_bp: 9000,
    };
    logos_repo
        .replace_directory_logos(
            source,
            &[
                write("bbc1", "https://dir/bbc1.png"),
                write("cnn.us", "https://dir/cnn.png"),
                write("name:no id", "https://dir/noid.png"),
                write("bein1", "https://dir/bein1.png"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        logos(&pool, source).await,
        vec![
            (
                "BBC One".to_string(),
                Some("https://epg/bbc1.png".to_string())
            ),
            (
                "CNN".to_string(),
                Some("https://provider/cnn.png".to_string())
            ),
            (
                "No Id".to_string(),
                Some("https://dir/noid.png".to_string())
            ),
            (
                "beIN Sports 1".to_string(),
                Some("https://dir/bein1.png".to_string())
            ),
            (
                "beIN Sports 2".to_string(),
                Some("https://provider/bein2.png".to_string())
            ),
        ]
    );
    let groups = ChannelRepository::new(&pool)
        .list_groups(source, None, None, None, false, false, 100, 0)
        .await
        .unwrap();
    let bein = groups.iter().find(|g| g.group == "bein sports").unwrap();
    assert_eq!(bein.logo_url.as_deref(), Some("https://provider/bein2.png"));

    logos_repo.clear_directory_logos().await.unwrap();
    assert_eq!(logos(&pool, source).await[2].1, None);
    pool.close().await;
}
