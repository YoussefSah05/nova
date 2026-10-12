//! Read only provider names/evidence; benchmark in an ephemeral synthetic database.
use nova_core::{Channel, SourceEvidence};
use nova_storage::{ChannelRepository, SourceRepository};
use sqlx::sqlite::SqliteConnectOptions;
use std::{str::FromStr, time::Instant};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = std::env::var("NOVA_LIVE_VALIDATION_PROFILE")?;
    let options = SqliteConnectOptions::from_str(&format!("sqlite:{path}"))?.read_only(true);
    let original = sqlx::SqlitePool::connect_with(options).await?;
    let rows = sqlx::query!("SELECT COALESCE(raw_name, name) as \"raw!: String\", tvg_id, country, category, raw_group, region FROM channels ORDER BY id").fetch_all(&original).await?;
    let dir = tempfile::tempdir()?;
    let pool = nova_storage::init_profile_db(
        dir.path()
            .join("profile.db")
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("temporary path"))?,
    )
    .await?;
    let source = SourceRepository::new(&pool)
        .create("m3u", "Validation", "https://example.invalid/list")
        .await?
        .id;
    let entries: Vec<_> = rows
        .into_iter()
        .enumerate()
        .map(|(i, r)| Channel {
            id: 0,
            source_id: source,
            tvg_id: r.tvg_id,
            name: r.raw.clone(),
            country: r.country,
            category: r.category,
            raw_group: r.raw_group,
            logo_url: None,
            stream_url: format!("https://example.invalid/live/fixture/fixture/{i}.ts"),
            is_favorite: false,
            is_in_watchlist: false,
            last_played_at: None,
            added_at: None,
            dedup_group: None,
            channel_group: None,
            region: r.region,
            evidence: SourceEvidence {
                raw_name: Some(r.raw),
                ..Default::default()
            },
        })
        .collect();
    let start = Instant::now();
    ChannelRepository::new(&pool).insert_batch(&entries).await?;
    println!(
        "Compiled {} streams in {} ms",
        entries.len(),
        start.elapsed().as_millis()
    );
    let repo = ChannelRepository::new(&pool);
    let start = Instant::now();
    let bundles = repo
        .list_groups(source, None, None, None, false, false, 200, 0)
        .await?;
    println!(
        "First {} bundles: {} ms",
        bundles.len(),
        start.elapsed().as_millis()
    );
    let start = Instant::now();
    let result = repo
        .list_groups(
            source,
            None,
            None,
            Some("beIN sports"),
            false,
            false,
            200,
            0,
        )
        .await?;
    println!("beIN search: {} ms", start.elapsed().as_millis());
    for b in result {
        println!(
            "{}: {} channels, {} streams, editions {:?}",
            b.title, b.member_count, b.stream_count, b.editions
        );
    }
    let mut count = 0;
    let mut offset = 0;
    loop {
        let page = repo
            .live_channels(
                source, None, None, None, None, None, false, false, 200, offset,
            )
            .await?;
        count += page.iter().map(|c| c.variants.len()).sum::<usize>();
        if page.len() < 200 {
            break;
        }
        offset += 200;
    }
    anyhow::ensure!(count == entries.len(), "unreachable stream");
    println!("Every input stream reachable through paged logical channels: {count}");
    let rows = repo
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
        .await?;
    for c in rows {
        println!(
            "{} [{}]: {:?}",
            c.title,
            c.edition,
            c.variants
                .iter()
                .map(|v| v.quality.as_deref().unwrap_or("Unknown"))
                .collect::<Vec<_>>()
        );
    }
    pool.close().await;
    original.close().await;
    Ok(())
}
