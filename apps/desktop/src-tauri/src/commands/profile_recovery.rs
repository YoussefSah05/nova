//! Explicit copy recovery; never replaces an installed profile database.
use nova_core::Profile;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::{ipc::ApiError, state::AppState};

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(
    test,
    ts(export, export_to = "../../../../apps/desktop/src/types/generated/")
)]
#[serde(rename_all = "camelCase")]
pub enum ProfileRecoveryStatus {
    Available,
    Unsupported,
}

fn profiles_root(app: &AppHandle) -> Result<std::path::PathBuf, ApiError> {
    Ok(crate::validation_data_dir()
        .unwrap_or(
            app.path()
                .app_data_dir()
                .map_err(|_| ApiError::Internal("Profile storage is unavailable.".into()))?,
        )
        .join("profiles"))
}

fn validate_id(id: &str) -> Result<(), ApiError> {
    if uuid::Uuid::parse_str(id)
        .map(|value| value.to_string() == id)
        .unwrap_or(false)
    {
        Ok(())
    } else {
        Err(ApiError::InvalidInput("Invalid profile ID.".into()))
    }
}

#[tauri::command]
pub async fn inspect_profile_recovery(
    profile_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<ProfileRecoveryStatus, ApiError> {
    validate_id(&profile_id)?;
    let _open = state.profile_open.lock().await;
    nova_storage::ProfileRepository::new(&state.pool)
        .get(&profile_id)
        .await
        .map_err(|_| ApiError::NotFound("Profile not found.".into()))?;
    if state.profile_pool(&profile_id).await.is_some() {
        return Ok(ProfileRecoveryStatus::Unsupported);
    }
    let directory = profiles_root(&app)?.join(&profile_id);
    let path = directory.join("profile.db");
    for entry in [&directory, &path] {
        if tokio::fs::symlink_metadata(entry)
            .await
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(true)
        {
            return Ok(ProfileRecoveryStatus::Unsupported);
        }
    }
    Ok(
        if nova_storage::profile_recovery::is_supported(&path).await {
            ProfileRecoveryStatus::Available
        } else {
            ProfileRecoveryStatus::Unsupported
        },
    )
}

#[tauri::command]
pub async fn recover_profile_copy(
    profile_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Profile, ApiError> {
    validate_id(&profile_id)?;
    let open_guard = state.profile_open.clone().lock_owned().await;
    if state.profile_pool(&profile_id).await.is_some() {
        return Err(ApiError::Conflict("This profile is already open.".into()));
    }
    let pool = state.pool.clone();
    let root = profiles_root(&app)?;
    let runtime = tokio::runtime::Handle::current();
    // Fingerprint/snapshot qualification is bounded by the profile-open lock and
    // runs off the async worker thread. No provider or AI workers are started.
    tokio::task::spawn_blocking(move || {
        let _open = open_guard;
        runtime.block_on(nova_storage::profile_recovery::recover_as_profile(
            &pool,
            &root,
            &profile_id,
        ))
    })
    .await
    .map_err(|_| {
        ApiError::Internal("Recovery was interrupted. Your original profile is unchanged.".into())
    })?
    .map_err(|_| {
        ApiError::Conflict(
            "Recovery could not be completed. Your original profile is unchanged.".into(),
        )
    })
}
