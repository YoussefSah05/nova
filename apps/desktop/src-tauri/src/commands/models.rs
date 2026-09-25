//! Read-only IPC view of the internal model registry. Discovery only inspects
//! local files and refreshes app-wide metadata; it never starts a download.

use nova_storage::ModelRepository;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::ipc::ApiError;
use crate::model_registry::discover_internal;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalModelStatus {
    pub model_id: String,
    pub capability: String,
    pub backend: String,
    pub availability: String,
    pub artifact_revision: Option<String>,
    pub observed_at: i64,
}

#[tauri::command]
pub async fn list_internal_models(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<InternalModelStatus>, ApiError> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (library, models) = tokio::task::spawn_blocking(move || discover_internal(&app_data_dir))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let repo = ModelRepository::new(&state.pool);
    repo.record_internal(&library, &models).await?;
    let persisted = repo.list_internal().await?;
    Ok(persisted
        .into_iter()
        .map(|model| InternalModelStatus {
            model_id: model.descriptor.id.as_str().to_string(),
            capability: model.descriptor.capability,
            backend: model.descriptor.backend,
            availability: model.availability.as_str().to_string(),
            artifact_revision: model.descriptor.artifact_revision,
            observed_at: model.observed_at,
        })
        .collect())
}
