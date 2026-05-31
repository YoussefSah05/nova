//! Per-profile viewing/behaviour settings (cleanup phase).
//!
//! Stored as a single JSON blob under key `viewing` in `profile_settings`.
//! [`get`](ProfileSettingsRepository::get) returns [`ProfileSettings::default`]
//! when unset, so callers always have a value.

use nova_core::ProfileSettings;
use sqlx::SqlitePool;

use crate::error::StorageError;

const VIEWING_KEY: &str = "viewing";

pub struct ProfileSettingsRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ProfileSettingsRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// The saved settings, or defaults when none have been saved yet (or the
    /// stored blob fails to parse — treat as unset rather than erroring).
    pub async fn get(&self) -> Result<ProfileSettings, StorageError> {
        let row = sqlx::query!(
            "SELECT value FROM profile_settings WHERE key = ?",
            VIEWING_KEY
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row
            .and_then(|r| serde_json::from_str(&r.value).ok())
            .unwrap_or_default())
    }

    /// Upsert the settings blob.
    pub async fn set(&self, settings: &ProfileSettings) -> Result<(), StorageError> {
        let value = serde_json::to_string(settings)?;
        sqlx::query!(
            r#"INSERT INTO profile_settings (key, value) VALUES (?, ?)
               ON CONFLICT(key) DO UPDATE SET value = excluded.value"#,
            VIEWING_KEY,
            value,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_profile_db;

    #[tokio::test]
    async fn defaults_then_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.db");
        let pool = init_profile_db(path.to_str().unwrap()).await.unwrap();
        let repo = ProfileSettingsRepository::new(&pool);

        // Unset → defaults (autoplay on).
        let def = repo.get().await.unwrap();
        assert!(def.autoplay_next);
        assert_eq!(def.default_caption_mode, "off");

        let mut s = def.clone();
        s.autoplay_next = false;
        s.default_caption_mode = "source".to_string();
        s.preferred_audio_lang = Some("fr".to_string());
        repo.set(&s).await.unwrap();
        assert_eq!(repo.get().await.unwrap(), s);
    }
}
