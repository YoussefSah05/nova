//! Channel-logo fallbacks from the opt-in iptv-org directory
//! (`directory_channel_logos`). Matching happens in `nova_artwork::directory`;
//! this stores the result per source.

use nova_core::now_ts;
use sqlx::SqlitePool;

use crate::error::StorageError;

/// A channel without a provider logo, to match against the directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogoCandidate {
    pub tvg_id: Option<String>,
    pub name: String,
    pub country: Option<String>,
}

/// One matched logo (`channel_key` as `nova_artwork::directory::channel_key`).
#[derive(Debug, Clone)]
pub struct DirectoryLogoWrite {
    pub channel_key: String,
    pub directory_id: String,
    pub url: String,
    pub method: &'static str,
    pub confidence_bp: i64,
}

pub struct ChannelLogoRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ChannelLogoRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Distinct channels of a source that have no provider logo.
    pub async fn candidates(&self, source_id: i64) -> Result<Vec<LogoCandidate>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT DISTINCT NULLIF(tvg_id, '') as "tvg_id?: String",
                      name as "name!: String",
                      NULLIF(country, '') as "country?: String"
               FROM channels
               WHERE source_id = ? AND (logo_url IS NULL OR logo_url = '')"#,
            source_id
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| LogoCandidate {
                tvg_id: r.tvg_id,
                name: r.name,
                country: r.country,
            })
            .collect())
    }

    /// Replace a source's directory logos with a fresh match.
    pub async fn replace_directory_logos(
        &self,
        source_id: i64,
        logos: &[DirectoryLogoWrite],
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "DELETE FROM directory_channel_logos WHERE source_id = ?",
            source_id
        )
        .execute(&mut *tx)
        .await?;
        let now = now_ts();
        for l in logos {
            sqlx::query!(
                "INSERT OR REPLACE INTO directory_channel_logos
                     (source_id, channel_key, directory_id, url, method, confidence_bp, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                source_id,
                l.channel_key,
                l.directory_id,
                l.url,
                l.method,
                l.confidence_bp,
                now,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Remove every directory logo (the feature was turned off).
    pub async fn clear_directory_logos(&self) -> Result<(), StorageError> {
        sqlx::query!("DELETE FROM directory_channel_logos")
            .execute(self.pool)
            .await?;
        Ok(())
    }
}
