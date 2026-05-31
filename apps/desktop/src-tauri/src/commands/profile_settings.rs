//! Per-profile viewing/behaviour settings commands (cleanup phase).
//!
//! `get_profile_settings` returns the saved settings (or defaults);
//! `set_profile_settings` upserts them. Stored in the profile DB.

use nova_core::ProfileSettings;
use nova_storage::ProfileSettingsRepository;
use tauri::State;

use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

#[tauri::command]
pub async fn get_profile_settings(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<ProfileSettings, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ProfileSettingsRepository::new(&pool)
        .get()
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_profile_settings(
    profile_id: String,
    settings: ProfileSettings,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ProfileSettingsRepository::new(&pool)
        .set(&settings)
        .await
        .map_err(Into::into)
}
