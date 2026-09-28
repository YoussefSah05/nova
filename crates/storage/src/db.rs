use std::time::Duration;

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    SqlitePool,
};

use crate::error::StorageError;

pub async fn init_app_db(path: &str) -> Result<SqlitePool, StorageError> {
    let pool = open_pool(path).await?;
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.set_ignore_missing(true);
    migrator.run(&pool).await?;
    Ok(pool)
}

pub async fn init_profile_db(path: &str) -> Result<SqlitePool, StorageError> {
    let pool = open_pool(path).await?;
    // `ignore_missing`: tolerate a DB that has *newer* migrations applied than
    // this build knows about (e.g. after running a feature branch then an older
    // one, or an app downgrade). Without it sqlx errors with VersionMissing and
    // the profile won't open. Pending older migrations still apply normally.
    let mut migrator = sqlx::migrate!("./migrations/profile");
    migrator.set_ignore_missing(true);
    migrator.run(&pool).await?;
    // Data step of the additive canonical-catalog migration (0021): bind
    // pre-existing movie rows to canonical entities, seeding user state.
    let canonical = crate::repositories::CanonicalRepository::new(&pool);
    canonical.backfill_movies().await?;
    // Canonical series (0036): bind pre-existing series to entities, seeding
    // their favorite/watchlist from the per-row flags.
    canonical.backfill_series().await?;
    // Entity resolution (0023): resolve once for profiles that predate it or
    // were last resolved by an older rule set; a no-op otherwise.
    canonical.ensure_movies_resolved().await?;
    // Canonical search projection (0030): built once, rebuilt on version bumps.
    crate::repositories::CatalogSearchRepository::new(&pool)
        .ensure_movies()
        .await?;
    Ok(pool)
}

async fn open_pool(path: &str) -> Result<SqlitePool, StorageError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;
    Ok(pool)
}
