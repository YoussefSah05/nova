//! App-wide permission to use a model. Missing preferences preserve existing behavior.
use sqlx::SqlitePool;

use crate::error::StorageError;

pub struct ModelPreferencesRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ModelPreferencesRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn enabled(&self, model_id: &str) -> Result<bool, StorageError> {
        let key = format!("model.enabled:{model_id}");
        let row = sqlx::query!("SELECT value FROM app_settings WHERE key = ?", key)
            .fetch_optional(self.pool)
            .await?;
        Ok(row.is_none_or(|row| row.value != "false"))
    }

    pub async fn set_enabled(&self, model_id: &str, enabled: bool) -> Result<(), StorageError> {
        let key = format!("model.enabled:{model_id}");
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
    async fn default_and_persistent_model_scoping() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::init_app_db(dir.path().join("app.db").to_str().unwrap())
            .await
            .unwrap();
        let repo = ModelPreferencesRepository::new(&pool);
        assert!(repo.enabled("one").await.unwrap());
        repo.set_enabled("one", false).await.unwrap();
        assert!(!ModelPreferencesRepository::new(&pool)
            .enabled("one")
            .await
            .unwrap());
        assert!(repo.enabled("two").await.unwrap());
        repo.set_enabled("one", true).await.unwrap();
        assert!(repo.enabled("one").await.unwrap());
    }
}
