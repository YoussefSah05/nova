use nova_storage::{LibraryHistoryItem, LibraryHistoryRepository};
use tauri::State;

use super::require_profile_pool;
use crate::{ipc::ApiError, state::AppState};

#[tauri::command]
pub async fn list_library_history(
    profile_id: String,
    source_id: i64,
    kind: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<LibraryHistoryItem>, ApiError> {
    if kind
        .as_deref()
        .is_some_and(|k| !matches!(k, "vod" | "episode" | "live"))
    {
        return Err(ApiError::Conflict("unknown history media kind".into()));
    }
    let pool = require_profile_pool(&profile_id, &state).await?;
    LibraryHistoryRepository::new(&pool)
        .list(
            source_id,
            kind.as_deref(),
            limit.unwrap_or(51).clamp(1, 100),
            offset.unwrap_or(0).max(0),
        )
        .await
        .map_err(Into::into)
}
