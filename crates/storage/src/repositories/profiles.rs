use nova_core::{now_ts, Profile};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StorageError;

pub struct ProfileRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ProfileRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, name: &str, avatar_color: &str) -> Result<Profile, StorageError> {
        self.register(&Uuid::new_v4().to_string(), name, avatar_color)
            .await
    }

    /// Register an already validated recovered profile. Callers generate the ID;
    /// the database must be published before this row becomes visible.
    pub async fn register(
        &self,
        id: &str,
        name: &str,
        avatar_color: &str,
    ) -> Result<Profile, StorageError> {
        let now = now_ts();
        sqlx::query!(
            "INSERT INTO profiles (id, name, avatar_color, created_at) VALUES (?, ?, ?, ?)",
            id,
            name,
            avatar_color,
            now,
        )
        .execute(self.pool)
        .await?;
        Ok(Profile {
            id: id.to_string(),
            name: name.to_string(),
            avatar_color: avatar_color.to_string(),
            created_at: now,
            last_used_at: None,
        })
    }

    pub async fn list(&self) -> Result<Vec<Profile>, StorageError> {
        let rows = sqlx::query!(
            "SELECT id, name, avatar_color, created_at, last_used_at FROM profiles ORDER BY created_at ASC"
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Profile {
                id: r.id,
                name: r.name,
                avatar_color: r.avatar_color,
                created_at: r.created_at,
                last_used_at: r.last_used_at,
            })
            .collect())
    }

    pub async fn get(&self, id: &str) -> Result<Profile, StorageError> {
        let row = sqlx::query!(
            "SELECT id, name, avatar_color, created_at, last_used_at FROM profiles WHERE id = ?",
            id
        )
        .fetch_optional(self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        Ok(Profile {
            id: row.id,
            name: row.name,
            avatar_color: row.avatar_color,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        })
    }

    /// Rename / recolour a profile, returning the updated row.
    pub async fn update(
        &self,
        id: &str,
        name: &str,
        avatar_color: &str,
    ) -> Result<Profile, StorageError> {
        sqlx::query!(
            "UPDATE profiles SET name = ?, avatar_color = ? WHERE id = ?",
            name,
            avatar_color,
            id,
        )
        .execute(self.pool)
        .await?;
        self.get(id).await
    }

    pub async fn update_last_used(&self, id: &str) -> Result<(), StorageError> {
        let now = now_ts();
        sqlx::query!("UPDATE profiles SET last_used_at = ? WHERE id = ?", now, id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), StorageError> {
        sqlx::query!("DELETE FROM profiles WHERE id = ?", id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}
