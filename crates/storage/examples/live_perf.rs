//! Times the Live screen's queries on a *copy* of a profile database.
//!
//! `NOVA_LIVE_PERF_DB=/path/to/copy/profile.db cargo run --release -p nova-storage --example live_perf`
//! The copy is modified (the live projection is recompiled); never point this at a live profile.
use nova_storage::{ChannelRepository, ProgramsRepository};
use std::time::Instant;

macro_rules! timed {
    ($label:expr, $e:expr) => {{
        let start = Instant::now();
        let out = $e;
        println!(
            "{:<34} {:>8.1} ms",
            $label,
            start.elapsed().as_secs_f64() * 1000.0
        );
        out
    }};
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = std::env::var("NOVA_LIVE_PERF_DB")?;
    let pool = nova_storage::init_profile_db(&path).await?;
    let source = sqlx::query_scalar!(
        r#"SELECT source_id as "id!: i64" FROM channels GROUP BY source_id ORDER BY COUNT(*) DESC LIMIT 1"#
    )
    .fetch_one(&pool)
    .await?;
    let repo = ChannelRepository::new(&pool);
    sqlx::query!("UPDATE channels SET live_ruleset = NULL")
        .execute(&pool)
        .await?;
    timed!("first open: ensure_live", repo.ensure_live().await?);
    timed!("ensure_live (already current)", repo.ensure_live().await?);
    let now = nova_core::now_ts();
    timed!("live_overview", repo.live_overview(source, now).await?);
    let groups = timed!(
        "wall: live_groups page 1",
        repo.live_groups(source, None, None, None, false, false, 200, 0)
            .await?
    );
    timed!(
        "wall: live_groups page 2",
        repo.live_groups(source, None, None, None, false, false, 200, 200)
            .await?
    );
    timed!(
        "wall: live_groups search",
        repo.live_groups(source, None, None, Some("bein"), false, false, 200, 0)
            .await?
    );
    let page = timed!(
        "guide: live_channels page 1",
        repo.live_channels(source, None, None, None, None, None, false, false, 200, 0)
            .await?
    );
    timed!(
        "guide: live_channels page 5",
        repo.live_channels(source, None, None, None, None, None, false, false, 200, 800)
            .await?
    );
    timed!(
        "guide: live_channels search",
        repo.live_channels(
            source,
            None,
            None,
            None,
            None,
            Some("sport"),
            false,
            false,
            200,
            0
        )
        .await?
    );
    let family = groups
        .iter()
        .max_by_key(|g| g.stream_count)
        .map(|g| g.group.clone())
        .unwrap_or_default();
    timed!(
        format!("bundle: live_channels '{family}'"),
        repo.live_channels(
            source,
            Some(&family),
            None,
            None,
            None,
            None,
            false,
            false,
            200,
            0
        )
        .await?
    );
    let ids: Vec<i64> = page.iter().take(20).map(|c| c.representative_id).collect();
    timed!(
        "guide: 20 channels of programmes",
        ProgramsRepository::new(&pool)
            .guide_for_channels(&ids, now - 3600, now + 4 * 3600)
            .await?
    );
    if let Some(first) = page.first() {
        timed!(
            "player: live_channel_for_row",
            repo.live_channel_for_row(first.representative_id).await?
        );
        timed!(
            "player: resolve_live",
            repo.resolve_live(source, &first.key, None, &[]).await?
        );
    }
    pool.close().await;
    Ok(())
}
