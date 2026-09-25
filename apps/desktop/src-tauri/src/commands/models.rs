//! Read-only IPC view of the internal model registry. Discovery only inspects
//! local files and refreshes app-wide metadata; it never starts a download.

use nova_storage::RegisteredModel;
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, State};

use crate::ipc::ApiError;
use crate::model_registry::refresh_internal;
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
    let persisted = observe_internal_models(&app, &state.pool).await?;
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

pub(crate) async fn observe_internal_models(
    app: &AppHandle,
    pool: &SqlitePool,
) -> Result<Vec<RegisteredModel>, ApiError> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    refresh_internal(app_data_dir, pool).await
}

pub(crate) async fn observe_internal_model(
    app: &AppHandle,
    pool: &SqlitePool,
    model_id: &str,
) -> Result<RegisteredModel, ApiError> {
    observe_internal_models(app, pool)
        .await?
        .into_iter()
        .find(|model| model.descriptor.id.as_str() == model_id)
        .ok_or_else(|| ApiError::NotFound("model not registered".into()))
}
