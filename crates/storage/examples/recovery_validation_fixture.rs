//! Build a synthetic data root for native profile-recovery validation.
//! Contains no real profile content. Never point it at an installed data root.
//!
//! Usage: recovery_validation_fixture /absolute/empty/dir [movie-count]
use nova_storage::profile_recovery::historical_migrator;
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: recovery_validation_fixture /absolute/empty/dir [movie-count]";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!(USAGE))?);
    let movies: u32 = match args.next() {
        Some(value) => value.to_str().unwrap_or_default().parse()?,
        None => 30_000,
    };
    anyhow::ensure!(root.is_absolute(), "{USAGE}");
    // Refuse anything that already has content: this tool only creates.
    if root.exists() {
        anyhow::ensure!(
            std::fs::read_dir(&root)?.next().is_none(),
            "refusing a non-empty directory"
        );
    } else {
        std::fs::create_dir_all(&root)?;
    }
    let app = nova_storage::init_app_db(root.join("app.db").to_str().unwrap()).await?;
    let repo = nova_storage::ProfileRepository::new(&app);
    let profiles = root.join("profiles");

    let legacy = repo.create("Validation legacy", "#e0a33e").await?;
    let path = profile_path(&profiles, &legacy.id)?;
    let pool = historical(&path).await?;
    seed(&pool, movies).await?;
    pool.close().await;

    let unknown = repo.create("Validation unknown lineage", "#7a8cff").await?;
    let path = profile_path(&profiles, &unknown.id)?;
    let pool = historical(&path).await?;
    seed(&pool, 10).await?;
    // Mimics the real unknown schema-6 checksum: not any known migration SQL.
    sqlx::raw_sql("UPDATE _sqlx_migrations SET checksum = zeroblob(48) WHERE version = 6")
        .execute(&pool)
        .await?;
    pool.close().await;

    let healthy = repo.create("Validation healthy", "#4fbf8f").await?;
    nova_storage::init_profile_db(profile_path(&profiles, &healthy.id)?.to_str().unwrap())
        .await?
        .close()
        .await;
    app.close().await;

    println!("Synthetic data root ready ({movies} legacy movies):");
    println!("  recoverable legacy:  {}", legacy.id);
    println!("  unknown lineage:     {}", unknown.id);
    println!("  healthy current:     {}", healthy.id);
    Ok(())
}

fn profile_path(profiles: &Path, id: &str) -> anyhow::Result<PathBuf> {
    let directory = profiles.join(id);
    std::fs::create_dir_all(&directory)?;
    Ok(directory.join("profile.db"))
}

async fn historical(path: &Path) -> anyhow::Result<SqlitePool> {
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true),
    )
    .await?;
    historical_migrator().run(&pool).await?;
    Ok(pool)
}

/// Historical-schema SQL (not checkable by the current compile-time schema),
/// mirroring the conversion test fixture with a scalable movie catalog.
async fn seed(pool: &SqlitePool, movies: u32) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO sources (id,kind,name,url) VALUES (1,'m3u','Synthetic','https://example.invalid/list');
         INSERT INTO channels (id,source_id,name,stream_url,is_favorite,is_in_watchlist)
           VALUES (1,1,'Synthetic Channel','https://example.invalid/live',1,0);
         INSERT INTO series_episodes (id,source_id,series_title,season,episode,stream_url,is_favorite,is_in_watchlist)
           VALUES (1,1,'Synthetic Series',1,1,'https://example.invalid/episode/1',1,1),
                  (2,1,'Synthetic Series',1,2,'https://example.invalid/episode/2',1,1);
         INSERT INTO series_meta (source_id,series_title,director,tmdb_cast,runtime,rating)
           VALUES (1,'Synthetic Series','director','cast',45,9);
         INSERT INTO profile_settings (key,value) VALUES ('fixture','saved');",
    )
    .execute(pool)
    .await?;
    // Integer-only interpolation; titles are unique so no duplicate merging.
    sqlx::raw_sql(&format!(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i < {movies})
         INSERT INTO vod_items (id,source_id,name,stream_url,is_favorite,is_in_watchlist)
           SELECT i,1,'Synthetic Film '||i,'https://example.invalid/movie/'||i,i%50=0,i%70=0 FROM n;
         INSERT INTO watch_history (media_kind,media_id,position_sec,duration_sec,watched_at,completed,caption_mode)
           SELECT 'vod',id,id%90,100,id,0,'off' FROM vod_items WHERE id%500=1;
         INSERT INTO watch_history (media_kind,media_id,position_sec,duration_sec,watched_at,completed,caption_mode)
           VALUES ('episode',1,24,200,20,0,'source');
         INSERT INTO transcripts (id,media_kind,media_id,content_key,language,model,vtt_path,json_path,created_at)
           VALUES (1,'vod',1,'synthetic-content','en','fixture','','',1);
         INSERT INTO transcript_segments (id,transcript_id,seq,start_sec,end_sec,text)
           VALUES (1,1,0,0,2,'synthetic dialogue');
         INSERT INTO llm_about VALUES ('movie','1','fixture','en','synthetic about',1);"
    ))
    .execute(pool)
    .await?;
    Ok(())
}
