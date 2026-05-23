use std::time::Duration;

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    SqlitePool,
};

use crate::error::StorageError;

pub async fn init_app_db(path: &str) -> Result<SqlitePool, StorageError> {
    let pool = open_pool(path).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

pub async fn init_profile_db(path: &str) -> Result<SqlitePool, StorageError> {
    let pool = open_pool(path).await?;
    sqlx::migrate!("./migrations/profile").run(&pool).await?;
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
