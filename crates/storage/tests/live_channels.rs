//! Source-scoped grouping, preservation, selection and publication boundaries.
use nova_core::{Channel, SourceEvidence};
use nova_storage::{init_profile_db, ChannelRepository, SourceRepository};
use sqlx::SqlitePool;

async fn fixture() -> (tempfile::TempDir, SqlitePool, i64) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let source = SourceRepository::new(&pool)
        .create("m3u", "Fixture", "https://example.invalid/list")
        .await
        .unwrap()
        .id;
    (dir, pool, source)
}
fn channel(source: i64, name: &str, index: usize) -> Channel {
    Channel {
        id: 0,
        source_id: source,
        tvg_id: Some(format!("guide-{index}")),
        name: name.into(),
        country: None,
        category: Some("Sports".into()),
        raw_group: None,
        logo_url: None,
        stream_url: format!("https://provider.invalid/live/user/secret/{}.ts", index + 1),
        is_favorite: false,
        is_in_watchlist: false,
        last_played_at: None,
        added_at: None,
        dedup_group: None,
        channel_group: None,
        region: None,
        evidence: SourceEvidence {
            raw_name: Some(name.into()),
            ..Default::default()
        },
    }
}
async fn seed(pool: &SqlitePool, source: i64) {
    let names = [
        "FR - BEINSPORT 1 SD",
        "FR - BEINSPORT 1 HD",
        "FR - BeIN SPORTS 1 FHD",
        "AR - BEINSPORTS 1 HD",
        "AR - BEINSPORTS 1 EN HD",
        "FR - BEINSPORT 2 HD",
        "FR - BEINSPORT MAX 4 HD",
        "FR - BEINSPORT 10 HD",
        "FR - France 24 HD",
    ];
    ChannelRepository::new(pool)
        .insert_batch(
            &names
                .iter()
                .enumerate()
                .map(|(i, s)| channel(source, s, i))
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
}
#[tokio::test]
async fn bundles_search_full_counts_and_quality_selection_are_source_scoped() {
    let (_dir, pool, source) = fixture().await;
    seed(&pool, source).await;
    let repo = ChannelRepository::new(&pool);
    let groups = repo
        .list_groups(
            source,
            None,
            None,
            Some("beIN sports 1 FR HD"),
            false,
            false,
            200,
            0,
        )
        .await
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].group, "bein sports");
    assert_eq!((groups[0].member_count, groups[0].stream_count), (6, 8));
    assert_eq!(groups[0].editions, ["AR", "EN", "FR"]);
    let rows = repo
        .live_channels(
            source,
            Some("bein sports"),
            None,
            None,
            None,
            Some("1 FR"),
            false,
            false,
            200,
            0,
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "channel 1 must not match channel 10");
    let c = &rows[0];
    assert_eq!(c.variants.len(), 3);
    let first = repo.resolve_live(source, &c.key, None, &[]).await.unwrap();
    assert_eq!(c.variants[0].quality.as_deref(), Some("FHD"));
    repo.record_live_outcome(first.channel_id, false)
        .await
        .unwrap();
    let fallback = repo.resolve_live(source, &c.key, None, &[]).await.unwrap();
    assert_ne!(first.channel_id, fallback.channel_id);
    assert_eq!(fallback.channel.variants[0].quality.as_deref(), Some("HD"));
    repo.record_live_outcome(fallback.channel_id, true)
        .await
        .unwrap();
    assert_eq!(
        repo.resolve_live(source, &c.key, None, &[])
            .await
            .unwrap()
            .channel_id,
        fallback.channel_id
    );
    let other_source = SourceRepository::new(&pool)
        .create("m3u", "Other", "https://example.invalid/other")
        .await
        .unwrap()
        .id;
    assert!(repo
        .resolve_live(other_source, &c.key, None, &[])
        .await
        .is_err());
    assert!(repo
        .resolve_live(source, &c.key, Some(&groups[0].group), &[])
        .await
        .is_err());
    let json = serde_json::to_string(&fallback).unwrap();
    assert!(
        !json.contains("secret")
            && !json.contains("provider.invalid")
            && !json.contains("streamUrl")
    );
}
#[tokio::test]
async fn recompile_and_refresh_preserve_preferences_and_rollback() {
    let (dir, pool, source) = fixture().await;
    seed(&pool, source).await;
    let repo = ChannelRepository::new(&pool);
    let c = repo
        .live_channels(
            source,
            Some("bein sports"),
            None,
            None,
            None,
            Some("FR 1"),
            false,
            false,
            200,
            0,
        )
        .await
        .unwrap()
        .remove(0);
    let hd = c
        .variants
        .iter()
        .find(|v| v.quality.as_deref() == Some("HD"))
        .unwrap();
    repo.record_live_outcome(hd.channel_id, true).await.unwrap();
    sqlx::query("UPDATE channels SET is_favorite=1 WHERE id=?")
        .bind(hd.channel_id)
        .execute(&pool)
        .await
        .unwrap();
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM channels ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    ChannelRepository::compile_live_tx(&mut tx, Some(source), true)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        ids,
        sqlx::query_scalar::<_, i64>("SELECT id FROM channels ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap()
    );
    assert!(
        repo.live_channel_for_row(hd.channel_id)
            .await
            .unwrap()
            .unwrap()
            .is_favorite
    );
    // A failed publication leaves the usable generation intact.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM channels WHERE source_id=?")
        .bind(source)
        .execute(&mut *tx)
        .await
        .unwrap();
    ChannelRepository::compile_live_tx(&mut tx, Some(source), true)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        repo.resolve_live(source, &c.key, None, &[])
            .await
            .unwrap()
            .variant_key,
        hd.variant_key
    );
    // Refresh rebinds durable preferences even when row IDs change.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM channels WHERE source_id=?")
        .bind(source)
        .execute(&mut *tx)
        .await
        .unwrap();
    let entries = [
        channel(source, "FR - BEINSPORT 1 HD", 1),
        channel(source, "FR - BeIN SPORTS 1 FHD", 2),
    ];
    ChannelRepository::insert_batch_tx(&mut tx, &entries)
        .await
        .unwrap();
    ChannelRepository::compile_live_tx(&mut tx, Some(source), true)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        repo.resolve_live(source, &c.key, None, &[])
            .await
            .unwrap()
            .variant_key,
        hd.variant_key
    );
    pool.close().await;
    let reopened = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        ChannelRepository::new(&reopened)
            .resolve_live(source, &c.key, None, &[])
            .await
            .unwrap()
            .variant_key,
        hd.variant_key
    );
}

/// Regression: the page of keys was a plain CTE that SQLite re-ran for every
/// channel row (quadratic: ~75 s per page on a real 8.7k-channel playlist).
/// The bound is far above the linear cost and far below the quadratic one.
#[tokio::test]
async fn live_pages_stay_fast_on_large_playlists() {
    let (_dir, pool, source) = fixture().await;
    let names: Vec<Channel> = (0..4_000)
        .map(|i| channel(source, &format!("FR - Channel {} HD", i % 1_500), i))
        .collect();
    let repo = ChannelRepository::new(&pool);
    repo.insert_batch(&names).await.unwrap();
    let start = std::time::Instant::now();
    let page = repo
        .live_channels(source, None, None, None, None, None, false, false, 200, 400)
        .await
        .unwrap();
    let key = page[0].key.clone();
    repo.resolve_live(source, &key, None, &[]).await.unwrap();
    let elapsed = start.elapsed();
    assert_eq!(page.len(), 200);
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "live page + resolve took {elapsed:?}"
    );
    // Pages are disjoint and follow edition, then natural title order.
    let next = repo
        .live_channels(source, None, None, None, None, None, false, false, 200, 600)
        .await
        .unwrap();
    assert!(page.iter().all(|a| next.iter().all(|b| a.key != b.key)));
}
