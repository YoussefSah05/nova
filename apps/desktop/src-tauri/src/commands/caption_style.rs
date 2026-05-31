//! Phase 6.2 — per-profile caption-style commands.
//!
//! The player's caption appearance (font, size, colour, backdrop) is stored in
//! the profile DB so it follows the profile. `get_caption_style` returns the
//! saved style (or defaults); `set_caption_style` upserts it.

use nova_core::CaptionStyle;
use nova_storage::CaptionStyleRepository;
use tauri::State;

use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

#[tauri::command]
pub async fn get_caption_style(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<CaptionStyle, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    CaptionStyleRepository::new(&pool)
        .get()
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_caption_style(
    profile_id: String,
    style: CaptionStyle,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    CaptionStyleRepository::new(&pool)
        .set(&style)
        .await
        .map_err(Into::into)
}
