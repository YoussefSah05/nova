//! Display preferences (app-wide): the interface refresh rate.

use nova_storage::repositories::display_preferences::DisplayPreferencesRepository;
use tauri::{AppHandle, State};

use crate::ipc::ApiError;
use crate::state::AppState;

#[tauri::command]
pub async fn get_high_refresh_rate(state: State<'_, AppState>) -> Result<bool, ApiError> {
    Ok(DisplayPreferencesRepository::new(&state.pool)
        .high_refresh_rate()
        .await?)
}

/// Persist the choice and apply it to the open window at once.
#[tauri::command]
pub async fn set_high_refresh_rate(
    enabled: bool,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), ApiError> {
    DisplayPreferencesRepository::new(&state.pool)
        .set_high_refresh_rate(enabled)
        .await?;
    apply_to_main_window(&app, enabled);
    Ok(())
}

pub(crate) fn apply_to_main_window(app: &AppHandle, enabled: bool) {
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("main") {
            crate::display_refresh::apply(&window, enabled);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, enabled);
}
