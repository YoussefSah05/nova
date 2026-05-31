//! Per-profile caption-style persistence (Phase 6.2).
//!
//! A single row (`id = 1`) in `caption_style`. [`get`](CaptionStyleRepository::get)
//! returns [`CaptionStyle::default`] when the row is absent, so the UI always has
//! a value without a first-run write.

use nova_core::{now_ts, CaptionStyle};
use sqlx::SqlitePool;

use crate::error::StorageError;

pub struct CaptionStyleRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> CaptionStyleRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// The saved caption style, or defaults when none has been saved yet.
    pub async fn get(&self) -> Result<CaptionStyle, StorageError> {
        let row = sqlx::query!(
            r#"SELECT
                 font_scale  as "font_scale!: f64",
                 font_family as "font_family!: String",
                 text_color  as "text_color!: String",
                 background   as "background!: String",
                 bg_opacity  as "bg_opacity!: f64"
               FROM caption_style WHERE id = 1"#,
        )
        .fetch_optional(self.pool)
        .await?;

        Ok(row
            .map(|r| CaptionStyle {
                font_scale: r.font_scale,
                font_family: r.font_family,
                text_color: r.text_color,
                background: r.background,
                bg_opacity: r.bg_opacity,
            })
            .unwrap_or_default())
    }

    /// Upsert the singleton caption-style row.
    pub async fn set(&self, style: &CaptionStyle) -> Result<(), StorageError> {
        let now = now_ts();
        sqlx::query!(
            r#"INSERT INTO caption_style
                 (id, font_scale, font_family, text_color, background, bg_opacity, updated_at)
               VALUES (1, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                 font_scale  = excluded.font_scale,
                 font_family = excluded.font_family,
                 text_color  = excluded.text_color,
                 background   = excluded.background,
                 bg_opacity  = excluded.bg_opacity,
                 updated_at  = excluded.updated_at"#,
            style.font_scale,
            style.font_family,
            style.text_color,
            style.background,
            style.bg_opacity,
            now,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
