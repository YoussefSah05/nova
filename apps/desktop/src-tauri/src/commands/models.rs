//! IPC views and controls for shared local models. Discovery only inspects
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
use crate::model_runtime::RuntimeError;
use crate::resource_policy::WorkClass;
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
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct ExternalModelStatus {
    pub model_id: String,
    pub capability: String,
    pub availability: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct ExternalLibraryStatus {
    pub library_id: String,
    pub display_name: String,
    pub availability: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub observed_at: i64,
    pub models: Vec<ExternalModelStatus>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct ModelRuntimeStatus {
    pub loaded_models: usize,
    pub active_leases: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub estimated_resident_bytes: u64,
    pub unknown_resident_models: usize,
    pub busy_models: usize,
    pub transitioning_models: usize,
    pub active_playback_sessions: usize,
    pub queued_work: usize,
    pub active_work_class: Option<String>,
    /// In-process heavyweight backends resident (or loading) and their budget.
    /// Realtime/interactive work may exceed the budget while every resident
    /// model is leased, so `residentModels` can briefly exceed it.
    pub resident_models: usize,
    pub resident_model_budget: usize,
    pub queued_jobs: usize,
    pub active_jobs: usize,
    pub job_slots: usize,
    /// Model or job work held back by active playback.
    pub suspended_work: usize,
}

/// Request explicit unload. Active inference leases always win over eviction.
#[tauri::command]
pub async fn unload_model(model_id: String, state: State<'_, AppState>) -> Result<bool, ApiError> {
    state
        .model_runtime
        .unload(&model_id)
        .await
        .map_err(runtime_api_error)
}

pub(crate) fn runtime_api_error(error: RuntimeError) -> ApiError {
    match error {
        RuntimeError::Busy => ApiError::Conflict(error.to_string()),
        RuntimeError::InvalidModel => ApiError::NotFound(error.to_string()),
        _ => ApiError::Internal(error.to_string()),
    }
}

#[tauri::command]
pub async fn model_runtime_status(
    state: State<'_, AppState>,
) -> Result<ModelRuntimeStatus, ApiError> {
    let snapshot = state.model_runtime.snapshot().await;
    Ok(ModelRuntimeStatus {
        loaded_models: snapshot.loaded_models,
        active_leases: snapshot.active_leases,
        estimated_resident_bytes: snapshot.estimated_resident_bytes,
        unknown_resident_models: snapshot.unknown_resident_models,
        busy_models: snapshot.busy_models,
        transitioning_models: snapshot.transitioning_models,
        active_playback_sessions: snapshot.active_playback_sessions,
        queued_work: snapshot.queued_work,
        active_work_class: snapshot
            .active_work_class
            .map(WorkClass::as_str)
            .map(str::to_string),
        resident_models: snapshot.resident_models,
        resident_model_budget: snapshot.resident_model_budget,
        queued_jobs: snapshot.queued_jobs,
        active_jobs: snapshot.active_jobs,
        job_slots: snapshot.job_slots,
        suspended_work: snapshot.suspended_work,
    })
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

/// Effective model location and activity, without filesystem paths or access grants.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct LocalModelStatus {
    pub model_id: String,
    pub capability: String,
    pub availability: String,
    pub library: String,
    pub enabled: bool,
    /// idle | loading | prepared (Whisper adapter only) | loaded | inUse
    pub activity: String,
}

fn effective_model<'a>(
    internal: &'a RegisteredModel,
    external: &'a [nova_storage::ExternalRegisteredModel],
) -> &'a RegisteredModel {
    use nova_core::models::ModelAvailability;
    let usable = |model: &RegisteredModel| {
        model.availability == ModelAvailability::Available
            || (model.descriptor.capability == "translation"
                && model.availability == ModelAvailability::Unverified)
    };
    if usable(internal) {
        return internal;
    }
    external
        .iter()
        .filter(|m| m.model.descriptor.id == internal.descriptor.id)
        .find(|m| usable(&m.model))
        .map(|m| &m.model)
        .or_else(|| {
            external
                .iter()
                .find(|m| {
                    m.model.descriptor.id == internal.descriptor.id
                        && m.model.availability == ModelAvailability::Unavailable
                })
                .map(|m| &m.model)
        })
        .unwrap_or(internal)
}

#[tauri::command]
pub async fn list_model_controls(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<LocalModelStatus>, ApiError> {
    let internal = observe_internal_models(&app, &state.pool).await?;
    refresh_external(&state.pool).await?;
    let external = ModelRepository::new(&state.pool)
        .list_external_models()
        .await?;
    let mut statuses = Vec::with_capacity(internal.len());
    for internal in &internal {
        let model = effective_model(internal, &external);
        let model_id = model.descriptor.id.as_str();
        let (enabled, activity) = state
            .model_runtime
            .model_activity(model_id)
            .await
            .map_err(runtime_api_error)?;
        statuses.push(LocalModelStatus {
            model_id: model_id.to_string(),
            capability: model.descriptor.capability.clone(),
            availability: model.availability.as_str().to_string(),
            library: if model.library.id.as_str() == "internal" {
                "internal"
            } else {
                "external"
            }
            .into(),
            enabled,
            activity: activity.into(),
        });
    }
    Ok(statuses)
}

#[tauri::command]
pub async fn set_model_enabled(
    model_id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    state
        .model_runtime
        .set_enabled(&model_id, enabled)
        .await
        .map_err(runtime_api_error)
}

#[cfg(test)]
mod control_tests {
    use super::*;
    use nova_core::models::ModelAvailability;

    #[test]
    fn effective_status_matches_runtime_location_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let (_, models) = crate::model_registry::discover_internal(dir.path()).unwrap();
        let mut internal = models
            .into_iter()
            .find(|m| m.descriptor.capability == "translation")
            .unwrap();
        let mut external_model = internal.clone();
        external_model.library.id = ModelLibraryId::new("external-fixture").unwrap();
        let mut external = vec![nova_storage::ExternalRegisteredModel {
            model: external_model,
            display_name: "Fixture".into(),
        }];
        external[0].model.availability = ModelAvailability::Unavailable;
        assert_eq!(
            effective_model(&internal, &external).library.id.as_str(),
            "external-fixture"
        );
        external[0].model.availability = ModelAvailability::Unverified;
        assert_eq!(
            effective_model(&internal, &external).library.id.as_str(),
            "external-fixture"
        );
        internal.availability = ModelAvailability::Available;
        assert_eq!(
            effective_model(&internal, &external).library.id.as_str(),
            "internal"
        );
        internal.descriptor.capability = "transcription".into();
        external[0].model.descriptor.capability = "transcription".into();
        internal.availability = ModelAvailability::Missing;
        assert_eq!(
            effective_model(&internal, &external).library.id.as_str(),
            "internal"
        );
    }
}
