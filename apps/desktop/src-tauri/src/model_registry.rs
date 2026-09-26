//! Offline discovery for the current internal model layouts. This metadata
//! registry does not install, load, unload, or authenticate model artifacts.

use std::path::{Path, PathBuf};

use nova_core::{
    models::{
        ModelAvailability, ModelContractError, ModelDescriptor, ModelId, ModelLibrary,
        ModelLibraryId, ModelLocation, RelativeModelPath,
    },
    now_ts,
};
use nova_storage::{ExternalLibraryRecord, ModelRepository, RegisteredModel};
use nova_transcribe::{default_model, is_installed as whisper_installed, model_paths};
use sqlx::SqlitePool;
use tokio::sync::Mutex;

use crate::ipc::ApiError;

// Status requests may arrive together while an installer changes local files.
// Serialize the observation and write so an older probe cannot overwrite a
// newer one after it has finished.
static REFRESH_LOCK: Mutex<()> = Mutex::const_new(());

pub fn internal_library() -> Result<ModelLibrary, ModelContractError> {
    Ok(ModelLibrary {
        id: ModelLibraryId::new("internal")?,
        relative_root: RelativeModelPath::new("models")?,
    })
}

fn registration(
    library: &ModelLibrary,
    id: &str,
    capability: &str,
    backend: &str,
    relative_path: &str,
    availability: ModelAvailability,
    observed_at: i64,
) -> Result<RegisteredModel, ModelContractError> {
    let id = ModelId::new(id)?;
    Ok(RegisteredModel {
        library: library.clone(),
        descriptor: ModelDescriptor {
            id: id.clone(),
            capability: capability.to_string(),
            backend: backend.to_string(),
            artifact_revision: None,
        },
        location: ModelLocation {
            model_id: id,
            library_id: library.id.clone(),
            relative_path: RelativeModelPath::new(relative_path)?,
        },
        availability,
        observed_at,
    })
}

fn file_has_data(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

fn directory_has_data(path: &Path) -> bool {
    path.read_dir()
        .is_ok_and(|mut entries| entries.next().is_some())
}

fn has_evidence(path: &Path) -> bool {
    path.try_exists().unwrap_or(true)
}

fn whisper_availability(dir: &Path) -> ModelAvailability {
    let model = default_model();
    let paths = model_paths(dir, &model);
    if !has_evidence(&paths.ggml) && !has_evidence(&paths.coreml_dir) {
        return ModelAvailability::Missing;
    }
    if whisper_installed(dir, &model)
        && file_has_data(&paths.ggml)
        && (!cfg!(target_os = "macos") || directory_has_data(&paths.coreml_dir))
    {
        ModelAvailability::Available
    } else {
        ModelAvailability::Unverified
    }
}

fn embedding_availability(dir: &Path) -> ModelAvailability {
    if nova_enrich::FastEmbedder::local_files_present(dir) {
        return ModelAvailability::Available;
    }
    let marker = dir.join(".installed");
    if has_evidence(&marker) || has_evidence(dir) {
        // FastEmbed's cache layout is opaque here. The old marker establishes
        // an install attempt, not artifact integrity or offline loadability.
        ModelAvailability::Unverified
    } else {
        ModelAvailability::Missing
    }
}

pub fn discover_internal(
    app_data_dir: &Path,
) -> Result<(ModelLibrary, Vec<RegisteredModel>), ModelContractError> {
    discover_library(app_data_dir, internal_library()?)
}

fn discover_library(
    base_dir: &Path,
    library: ModelLibrary,
) -> Result<(ModelLibrary, Vec<RegisteredModel>), ModelContractError> {
    let root = library.relative_root.resolve(base_dir);
    let observed_at = now_ts();
    let models = vec![
        registration(
            &library,
            default_model().id,
            "transcription",
            "whisper.cpp",
            "whisper",
            whisper_availability(&root.join("whisper")),
            observed_at,
        )?,
        registration(
            &library,
            nova_enrich::MODEL_ID,
            "embedding",
            "fastembed",
            "embeddings",
            embedding_availability(&root.join("embeddings")),
            observed_at,
        )?,
    ];
    Ok((library, models))
}

fn unavailable_models(library: &ModelLibrary) -> Result<Vec<RegisteredModel>, ModelContractError> {
    let observed_at = now_ts();
    Ok(vec![
        registration(
            library,
            default_model().id,
            "transcription",
            "whisper.cpp",
            "whisper",
            ModelAvailability::Unavailable,
            observed_at,
        )?,
        registration(
            library,
            nova_enrich::MODEL_ID,
            "embedding",
            "fastembed",
            "embeddings",
            ModelAvailability::Unavailable,
            observed_at,
        )?,
    ])
}

/// A disconnected root is distinct from an online library lacking artifacts.
/// No selected external path is persisted or sent to the webview.
pub(crate) fn observe_external_at(
    library: &ModelLibrary,
    selected_root: Option<&Path>,
) -> Result<(bool, Vec<RegisteredModel>), ModelContractError> {
    let Some(root) = selected_root.filter(|path| path.is_dir()) else {
        return Ok((false, unavailable_models(library)?));
    };
    let models_dir = library.relative_root.resolve(root);
    let whisper_dir = models_dir.join("whisper");
    let whisper_paths = model_paths(&whisper_dir, &default_model());
    let embedding_dir = models_dir.join("embeddings");
    let linked = [
        models_dir.as_path(),
        whisper_dir.as_path(),
        whisper_paths.ggml.as_path(),
        whisper_paths.coreml_dir.as_path(),
        embedding_dir.as_path(),
    ];
    if linked.iter().any(|path| is_symlink(path)) {
        return Ok((false, unavailable_models(library)?));
    }
    let (_, models) = discover_library(root, library.clone())?;
    Ok((true, models))
}

fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
}

fn probe_external(
    mut record: ExternalLibraryRecord,
) -> Result<(ExternalLibraryRecord, Vec<RegisteredModel>), ModelContractError> {
    #[cfg(target_os = "macos")]
    let resolved =
        crate::mac_model_library::with_bookmarked_folder(&record.bookmark_data, |root| {
            observe_external_at(&record.library, Some(root))
        });
    #[cfg(not(target_os = "macos"))]
    let resolved: Option<(
        Result<(bool, Vec<RegisteredModel>), ModelContractError>,
        Option<Vec<u8>>,
    )> = None;

    let (online, models) = match resolved {
        Some((observation, renewal)) => {
            if let Some(bookmark) = renewal {
                record.bookmark_data = bookmark;
            }
            observation?
        }
        None => observe_external_at(&record.library, None)?,
    };
    record.online = online;
    record.observed_at = now_ts();
    Ok((record, models))
}

pub async fn register_external(
    pool: &SqlitePool,
    record: ExternalLibraryRecord,
) -> Result<(), ApiError> {
    let _guard = REFRESH_LOCK.lock().await;
    let (record, models) = tokio::task::spawn_blocking(move || probe_external(record))
        .await
        .map_err(|_| ApiError::Internal("model-library probe failed".into()))?
        .map_err(|_| ApiError::Internal("invalid model-library contract".into()))?;
    ModelRepository::new(pool)
        .record_external(&record, &models)
        .await?;
    Ok(())
}

pub async fn refresh_external(pool: &SqlitePool) -> Result<(), ApiError> {
    let _guard = REFRESH_LOCK.lock().await;
    let repo = ModelRepository::new(pool);
    for record in repo.list_external_libraries().await? {
        let (record, models) = tokio::task::spawn_blocking(move || probe_external(record))
            .await
            .map_err(|_| ApiError::Internal("model-library probe failed".into()))?
            .map_err(|_| ApiError::Internal("invalid model-library contract".into()))?;
        repo.record_external(&record, &models).await?;
    }
    Ok(())
}

pub async fn refresh_internal(
    app_data_dir: PathBuf,
    pool: &SqlitePool,
) -> Result<Vec<RegisteredModel>, ApiError> {
    let _guard = REFRESH_LOCK.lock().await;
    let (library, models) = tokio::task::spawn_blocking(move || discover_internal(&app_data_dir))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let repo = ModelRepository::new(pool);
    repo.record_internal(&library, &models).await?;
    Ok(repo.list_internal().await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_storage::init_app_db;

    /// A structurally complete Whisper install (GGML weights + Core ML dir).
    fn install_whisper(whisper_dir: &Path) {
        std::fs::create_dir_all(whisper_dir).unwrap();
        let paths = model_paths(whisper_dir, &default_model());
        std::fs::write(&paths.ggml, b"fixture").unwrap();
        std::fs::create_dir_all(&paths.coreml_dir).unwrap();
        std::fs::write(paths.coreml_dir.join("fixture"), b"x").unwrap();
    }

    #[test]
    fn discovery_is_offline_and_preserves_existing_internal_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (library, missing) = discover_internal(dir.path()).unwrap();
        assert_eq!(missing.len(), 2);
        assert!(missing
            .iter()
            .all(|m| m.availability == ModelAvailability::Missing));
        assert!(
            !dir.path().join("models").exists(),
            "discovery created model files"
        );

        let whisper = missing
            .iter()
            .find(|m| m.descriptor.id.as_str() == default_model().id)
            .unwrap();
        assert_eq!(
            whisper.location.resolve(dir.path(), &library),
            Some(dir.path().join("models/whisper"))
        );
    }

    #[test]
    fn whisper_layout_and_embedding_marker_have_distinct_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let whisper_dir = dir.path().join("models/whisper");
        std::fs::create_dir_all(&whisper_dir).unwrap();
        let paths = model_paths(&whisper_dir, &default_model());
        std::fs::write(&paths.ggml, b"").unwrap();
        assert_eq!(
            whisper_availability(&whisper_dir),
            ModelAvailability::Unverified
        );
        std::fs::write(&paths.ggml, b"nonempty fixture").unwrap();
        if cfg!(target_os = "macos") {
            std::fs::create_dir_all(&paths.coreml_dir).unwrap();
            std::fs::write(paths.coreml_dir.join("fixture"), b"x").unwrap();
        }
        assert_eq!(
            whisper_availability(&whisper_dir),
            ModelAvailability::Available
        );

        let embedding_dir = dir.path().join("models/embeddings");
        std::fs::create_dir_all(&embedding_dir).unwrap();
        std::fs::write(embedding_dir.join(".installed"), b"1").unwrap();
        assert_eq!(
            embedding_availability(&embedding_dir),
            ModelAvailability::Unverified
        );
    }

    #[tokio::test]
    async fn refresh_persists_unavailable_and_restored_location_with_stable_identity() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("app.db");
        let pool = init_app_db(db_path.to_str().unwrap()).await.unwrap();
        let id = default_model().id;

        let missing = refresh_internal(dir.path().to_path_buf(), &pool)
            .await
            .unwrap();
        let first = missing
            .iter()
            .find(|model| model.descriptor.id.as_str() == id)
            .unwrap();
        assert_eq!(first.availability, ModelAvailability::Missing);
        assert_eq!(
            first.location.resolve(dir.path(), &first.library),
            Some(dir.path().join("models/whisper"))
        );

        let whisper_dir = dir.path().join("models/whisper");
        std::fs::create_dir_all(&whisper_dir).unwrap();
        install_whisper(&whisper_dir);
        let present = refresh_internal(dir.path().to_path_buf(), &pool)
            .await
            .unwrap();
        let second = present
            .iter()
            .find(|model| model.descriptor.id.as_str() == id)
            .unwrap();
        assert_eq!(second.availability, ModelAvailability::Available);
        assert_eq!(second.descriptor.id, first.descriptor.id);

        std::fs::remove_dir_all(&whisper_dir).unwrap();
        let unavailable = refresh_internal(dir.path().to_path_buf(), &pool)
            .await
            .unwrap();
        let third = unavailable
            .iter()
            .find(|model| model.descriptor.id.as_str() == id)
            .unwrap();
        assert_eq!(third.availability, ModelAvailability::Missing);
        assert_eq!(third.descriptor.id, first.descriptor.id);
        assert_eq!(third.location, first.location);
        pool.close().await;
    }

    #[test]
    fn removable_root_disconnects_and_reconnects_without_changing_model_id() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("removable-library");
        let library = ModelLibrary {
            id: ModelLibraryId::new("external-fixture").unwrap(),
            relative_root: RelativeModelPath::new("models").unwrap(),
        };
        std::fs::create_dir_all(root.join("models/whisper")).unwrap();
        let whisper_dir = root.join("models/whisper");
        install_whisper(&whisper_dir);
        let (connected, first) = observe_external_at(&library, Some(&root)).unwrap();
        assert!(connected);
        let first_model = first
            .iter()
            .find(|m| m.descriptor.id.as_str() == default_model().id)
            .unwrap();
        assert_eq!(first_model.availability, ModelAvailability::Available);

        std::fs::remove_dir_all(&root).unwrap();
        let (connected, offline) = observe_external_at(&library, Some(&root)).unwrap();
        assert!(!connected);
        let offline_model = offline
            .iter()
            .find(|m| m.descriptor.id.as_str() == default_model().id)
            .unwrap();
        assert_eq!(offline_model.availability, ModelAvailability::Unavailable);
        assert_eq!(offline_model.descriptor.id, first_model.descriptor.id);

        std::fs::create_dir_all(&whisper_dir).unwrap();
        install_whisper(&whisper_dir);
        let (connected, restored) = observe_external_at(&library, Some(&root)).unwrap();
        assert!(connected);
        let restored_model = restored
            .iter()
            .find(|m| m.descriptor.id.as_str() == default_model().id)
            .unwrap();
        assert_eq!(restored_model.availability, ModelAvailability::Available);
        assert_eq!(restored_model.descriptor.id, first_model.descriptor.id);
        assert_eq!(restored_model.location, first_model.location);
    }

    #[cfg(unix)]
    #[test]
    fn external_probe_does_not_follow_model_artifact_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("selected");
        let whisper_dir = root.join("models/whisper");
        std::fs::create_dir_all(&whisper_dir).unwrap();
        let outside = dir.path().join("outside-model");
        std::fs::write(&outside, b"fixture").unwrap();
        symlink(&outside, model_paths(&whisper_dir, &default_model()).ggml).unwrap();
        let library = ModelLibrary {
            id: ModelLibraryId::new("external-fixture").unwrap(),
            relative_root: RelativeModelPath::new("models").unwrap(),
        };
        let (online, models) = observe_external_at(&library, Some(&root)).unwrap();
        assert!(!online);
        assert!(models
            .iter()
            .all(|model| model.availability == ModelAvailability::Unavailable));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn native_bookmark_refresh_persists_disconnect_and_reconnect() {
        use objc2_foundation::{NSString, NSURLBookmarkCreationOptions, NSURL};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("removable-library");
        let whisper_dir = root.join("models/whisper");
        std::fs::create_dir_all(&whisper_dir).unwrap();
        install_whisper(&whisper_dir);
        let url =
            NSURL::fileURLWithPath_isDirectory(&NSString::from_str(root.to_str().unwrap()), true);
        let bookmark = url
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::empty(),
                None,
                None,
            )
            .unwrap();
        let mut bookmark_data = vec![0]; // Plain bookmark in the current non-sandboxed bundle.
        bookmark_data.extend_from_slice(&bookmark.to_vec());

        let pool = init_app_db(dir.path().join("app.db").to_str().unwrap())
            .await
            .unwrap();
        let library = ModelLibrary {
            id: ModelLibraryId::new("external-fixture").unwrap(),
            relative_root: RelativeModelPath::new("models").unwrap(),
        };
        register_external(
            &pool,
            ExternalLibraryRecord {
                library,
                display_name: "Fixture library".into(),
                bookmark_data,
                online: false,
                observed_at: now_ts(),
            },
        )
        .await
        .unwrap();
        let repo = ModelRepository::new(&pool);
        let first = repo.list_external_models().await.unwrap();
        let first_model = first
            .iter()
            .find(|m| m.model.descriptor.id.as_str() == default_model().id)
            .unwrap();
        assert_eq!(first_model.model.availability, ModelAvailability::Available);

        std::fs::remove_dir_all(&root).unwrap();
        refresh_external(&pool).await.unwrap();
        let offline = repo.list_external_models().await.unwrap();
        let offline_model = offline
            .iter()
            .find(|m| m.model.descriptor.id.as_str() == default_model().id)
            .unwrap();
        assert_eq!(
            offline_model.model.availability,
            ModelAvailability::Unavailable
        );
        assert_eq!(
            offline_model.model.descriptor.id,
            first_model.model.descriptor.id
        );

        std::fs::create_dir_all(&whisper_dir).unwrap();
        install_whisper(&whisper_dir);
        refresh_external(&pool).await.unwrap();
        let restored = repo.list_external_models().await.unwrap();
        let restored_model = restored
            .iter()
            .find(|m| m.model.descriptor.id.as_str() == default_model().id)
            .unwrap();
        assert_eq!(
            restored_model.model.availability,
            ModelAvailability::Available
        );
        assert_eq!(restored_model.model.location, first_model.model.location);
    }
}
