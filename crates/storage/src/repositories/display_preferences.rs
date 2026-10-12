//! App-wide display preferences, shared by every profile on this Mac.
use sqlx::SqlitePool;

use crate::error::StorageError;

const HIGH_REFRESH_RATE: &str = "display.highRefreshRate";

pub struct DisplayPreferencesRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> DisplayPreferencesRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Render the interface at the display's full refresh rate (120 Hz on
    /// ProMotion) instead of WebKit's default 60. On unless turned off.
    pub async fn high_refresh_rate(&self) -> Result<bool, StorageError> {
        let key = HIGH_REFRESH_RATE;
        let row = sqlx::query!("SELECT value FROM app_settings WHERE key = ?", key)
            .fetch_optional(self.pool)
            .await?;
        Ok(row.is_none_or(|row| row.value != "false"))
    }

    pub async fn set_high_refresh_rate(&self, enabled: bool) -> Result<(), StorageError> {
        let key = HIGH_REFRESH_RATE;
        let value = if enabled { "true" } else { "false" };
        sqlx::query!(
            "INSERT INTO app_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            key, value
        ).execute(self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn high_refresh_rate_defaults_on_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::init_app_db(dir.path().join("app.db").to_str().unwrap())
            .await
            .unwrap();
        let repo = DisplayPreferencesRepository::new(&pool);
        assert!(repo.high_refresh_rate().await.unwrap());
        repo.set_high_refresh_rate(false).await.unwrap();
        assert!(!repo.high_refresh_rate().await.unwrap());
        repo.set_high_refresh_rate(true).await.unwrap();
        assert!(repo.high_refresh_rate().await.unwrap());
    }
}
