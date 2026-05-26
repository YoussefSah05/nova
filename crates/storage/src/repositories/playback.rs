//! Read helpers for the player. Today: resolve a media id to its upstream URL
//! so the HLS proxy can fetch from it without the frontend leaking the raw
//! stream URL into its DOM (also lets us swap providers later without touching
//! the frontend).

use sqlx::SqlitePool;

use crate::error::StorageError;

pub struct PlaybackRepository<'a> {
    pool: &'a SqlitePool,
}

/// Output of [`PlaybackRepository::resolve_stream_url`]. `source_lang` is
/// optional today (no detection yet); the field exists so the player can
/// pass it through to Phase 6 transcription without a follow-up query.
#[derive(Debug, Clone)]
pub struct ResolvedStreamUrl {
    pub upstream_url: String,
    pub source_lang: Option<String>,
}

impl<'a> PlaybackRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Look up the upstream stream URL for `(media_kind, media_id)`.
    /// Returns `StorageError::NotFound` if the id doesn't exist in the
    /// table that matches `media_kind`.
    pub async fn resolve_stream_url(
        &self,
        media_kind: &str,
        media_id: i64,
    ) -> Result<ResolvedStreamUrl, StorageError> {
        match media_kind {
            "live" => {
                let row = sqlx::query!(
                    r#"SELECT stream_url as "stream_url!: String"
                       FROM channels WHERE id = ? LIMIT 1"#,
                    media_id,
                )
                .fetch_optional(self.pool)
                .await?
                .ok_or(StorageError::NotFound)?;
                Ok(ResolvedStreamUrl {
                    upstream_url: row.stream_url,
                    source_lang: None,
                })
            }
            "vod" => {
                let row = sqlx::query!(
                    r#"SELECT
                        stream_url        as "stream_url!: String",
                        detected_language as "detected_language?: String"
                       FROM vod_items WHERE id = ? LIMIT 1"#,
                    media_id,
                )
                .fetch_optional(self.pool)
                .await?
                .ok_or(StorageError::NotFound)?;
                Ok(ResolvedStreamUrl {
                    upstream_url: row.stream_url,
                    source_lang: row.detected_language,
                })
            }
            "episode" => {
                let row = sqlx::query!(
                    r#"SELECT stream_url as "stream_url!: String"
                       FROM series_episodes WHERE id = ? LIMIT 1"#,
                    media_id,
                )
                .fetch_optional(self.pool)
                .await?
                .ok_or(StorageError::NotFound)?;
                Ok(ResolvedStreamUrl {
                    upstream_url: row.stream_url,
                    source_lang: None,
                })
            }
            _ => Err(StorageError::NotFound),
        }
    }
}
