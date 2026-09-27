//! TMDB title artwork (`tmdb_title_art`): stored per TMDB work, resolved from
//! a movie entity's confident match or a series' match.

use nova_core::now_ts;
use sqlx::SqlitePool;

use crate::error::StorageError;

/// Which TMDB id namespace a work lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleArtMedia {
    Movie,
    Tv,
}

impl TitleArtMedia {
    fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "tv",
        }
    }
}

/// Stored art for one work.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredTitleArt {
    pub language: String,
    pub logo_url: Option<String>,
    pub logo_language: Option<String>,
    pub logo_width: Option<i64>,
    pub logo_height: Option<i64>,
    pub backdrop_url: Option<String>,
    pub artwork_version: i64,
}

pub struct TitleArtRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> TitleArtRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// The TMDB movie id of an entity's confident match.
    pub async fn movie_tmdb_id(&self, entity_id: &str) -> Result<Option<i64>, StorageError> {
        let id = sqlx::query_scalar!(
            r#"SELECT external_id as "external_id!: String" FROM entity_external_matches
               WHERE entity_id = ? AND namespace = 'tmdb:movie' AND status = 'matched'"#,
            entity_id
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(id.and_then(|s| s.parse::<i64>().ok()).filter(|id| *id > 0))
    }

    /// The TMDB TV id of a series' confident match in a source.
    pub async fn series_tmdb_id(
        &self,
        source_id: i64,
        series_title: &str,
    ) -> Result<Option<i64>, StorageError> {
        let id = sqlx::query_scalar!(
            r#"SELECT tmdb_id as "tmdb_id!: i64" FROM series_meta
               WHERE source_id = ? AND series_title = ?
                 AND tmdb_status = 'matched' AND tmdb_id IS NOT NULL"#,
            source_id,
            series_title
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(id.filter(|id| *id > 0))
    }

    pub async fn get(
        &self,
        media: TitleArtMedia,
        tmdb_id: i64,
    ) -> Result<Option<StoredTitleArt>, StorageError> {
        let media = media.as_str();
        let row = sqlx::query_as!(
            StoredTitleArt,
            r#"SELECT language as "language!: String", logo_url, logo_language,
                      logo_width, logo_height, backdrop_url,
                      artwork_version as "artwork_version!: i64"
               FROM tmdb_title_art WHERE media_type = ? AND tmdb_id = ?"#,
            media,
            tmdb_id
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    pub async fn put(
        &self,
        media: TitleArtMedia,
        tmdb_id: i64,
        art: &StoredTitleArt,
    ) -> Result<(), StorageError> {
        let media = media.as_str();
        let now = now_ts();
        sqlx::query!(
            "INSERT INTO tmdb_title_art
                 (media_type, tmdb_id, language, logo_url, logo_language, logo_width,
                  logo_height, backdrop_url, artwork_version, fetched_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(media_type, tmdb_id) DO UPDATE SET
                 language = excluded.language,
                 logo_url = excluded.logo_url,
                 logo_language = excluded.logo_language,
                 logo_width = excluded.logo_width,
                 logo_height = excluded.logo_height,
                 backdrop_url = excluded.backdrop_url,
                 artwork_version = excluded.artwork_version,
                 fetched_at = excluded.fetched_at",
            media,
            tmdb_id,
            art.language,
            art.logo_url,
            art.logo_language,
            art.logo_width,
            art.logo_height,
            art.backdrop_url,
            art.artwork_version,
            now,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
