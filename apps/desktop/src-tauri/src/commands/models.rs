//! Read-only IPC view of the internal model registry. Discovery only inspects
//! local files and refreshes app-wide metadata; it never starts a download.

use nova_core::{
    models::{ModelLibrary, ModelLibraryId, RelativeModelPath},
    now_ts,
};
use nova_storage::{ExternalLibraryRecord, ModelRepository, RegisteredModel};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, State};

use crate::ipc::ApiError;
use crate::model_registry::{refresh_external, refresh_internal, register_external};
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalModelStatus {
    pub model_id: String,
    pub capability: String,
    pub availability: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalLibraryStatus {
    pub library_id: String,
    pub display_name: String,
    pub availability: String,
    pub observed_at: i64,
    pub models: Vec<ExternalModelStatus>,
}

#[tauri::command]
pub async fn add_external_model_library(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, ApiError> {
    #[cfg(target_os = "macos")]
    {
        let Some(bookmark_data) = crate::mac_model_library::choose_folder(&app).await? else {
            return Ok(None);
        };
        let id = format!("external-{}", uuid::Uuid::new_v4());
        let library = ModelLibrary {
            id: ModelLibraryId::new(&id)
                .map_err(|_| ApiError::Internal("invalid library ID".into()))?,
            relative_root: RelativeModelPath::new("models")
                .map_err(|_| ApiError::Internal("invalid library root".into()))?,
        };
        register_external(
            &state.pool,
            ExternalLibraryRecord {
                library,
                display_name: "External model library".into(),
                bookmark_data,
                online: false,
                observed_at: now_ts(),
            },
        )
        .await?;
        Ok(Some(id))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, state);
        Err(ApiError::Internal(
            "external model folders require macOS folder authorization".into(),
        ))
    }
}

#[tauri::command]
pub async fn list_external_model_libraries(
    state: State<'_, AppState>,
) -> Result<Vec<ExternalLibraryStatus>, ApiError> {
    refresh_external(&state.pool).await?;
    let repo = ModelRepository::new(&state.pool);
    let libraries = repo.list_external_libraries().await?;
    let models = repo.list_external_models().await?;
    Ok(libraries
        .into_iter()
        .map(|library| ExternalLibraryStatus {
            library_id: library.library.id.as_str().to_string(),
            display_name: library.display_name,
            availability: if library.online {
                "available"
            } else {
                "unavailable"
            }
            .into(),
            observed_at: library.observed_at,
            models: models
                .iter()
                .filter(|model| model.model.library.id == library.library.id)
                .map(|model| ExternalModelStatus {
                    model_id: model.model.descriptor.id.as_str().to_string(),
                    capability: model.model.descriptor.capability.clone(),
                    availability: model.model.availability.as_str().to_string(),
                })
                .collect(),
        })
        .collect())
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
