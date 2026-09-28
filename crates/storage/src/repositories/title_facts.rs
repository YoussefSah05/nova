//! Stored TMDB facts per work (`tmdb_title_facts`) and per season
//! (`tmdb_season_episodes`). The payloads are opaque JSON produced by
//! `nova_enrich::tmdb_facts`; freshness is decided by the caller.

use sqlx::SqlitePool;

use crate::error::StorageError;
use crate::repositories::title_art::TitleArtMedia;

/// A stored JSON payload with what decides whether it is still current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFacts {
    pub json: String,
    pub language: String,
    pub facts_version: i64,
    pub fetched_at: i64,
}

pub struct TitleFactsRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> TitleFactsRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(
        &self,
        media: TitleArtMedia,
        tmdb_id: i64,
    ) -> Result<Option<StoredFacts>, StorageError> {
        let media = media.as_str();
        let row = sqlx::query!(
            r#"SELECT facts as "json!: String", language as "language!: String",
                      facts_version as "facts_version!: i64", fetched_at as "fetched_at!: i64"
               FROM tmdb_title_facts WHERE media_type = ? AND tmdb_id = ?"#,
            media,
            tmdb_id
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| StoredFacts {
            json: r.json,
            language: r.language,
            facts_version: r.facts_version,
            fetched_at: r.fetched_at,
        }))
    }

    pub async fn put(
        &self,
        media: TitleArtMedia,
        tmdb_id: i64,
        language: &str,
        json: &str,
        facts_version: i64,
    ) -> Result<(), StorageError> {
        let media = media.as_str();
        let now = nova_core::now_ts();
        sqlx::query!(
            "INSERT INTO tmdb_title_facts (media_type, tmdb_id, language, facts, facts_version, fetched_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(media_type, tmdb_id) DO UPDATE SET
                 language = excluded.language, facts = excluded.facts,
                 facts_version = excluded.facts_version, fetched_at = excluded.fetched_at",
            media,
            tmdb_id,
            language,
            json,
            facts_version,
            now,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn season(
        &self,
        tmdb_id: i64,
        season: i64,
    ) -> Result<Option<StoredFacts>, StorageError> {
        let row = sqlx::query!(
            r#"SELECT episodes as "json!: String", language as "language!: String",
                      facts_version as "facts_version!: i64", fetched_at as "fetched_at!: i64"
               FROM tmdb_season_episodes WHERE tmdb_id = ? AND season = ?"#,
            tmdb_id,
            season
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| StoredFacts {
            json: r.json,
            language: r.language,
            facts_version: r.facts_version,
            fetched_at: r.fetched_at,
        }))
    }

    pub async fn put_season(
        &self,
        tmdb_id: i64,
        season: i64,
        language: &str,
        json: &str,
        facts_version: i64,
    ) -> Result<(), StorageError> {
        let now = nova_core::now_ts();
        sqlx::query!(
            "INSERT INTO tmdb_season_episodes (tmdb_id, season, language, episodes, facts_version, fetched_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(tmdb_id, season) DO UPDATE SET
                 language = excluded.language, episodes = excluded.episodes,
                 facts_version = excluded.facts_version, fetched_at = excluded.fetched_at",
            tmdb_id,
            season,
            language,
            json,
            facts_version,
            now,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
