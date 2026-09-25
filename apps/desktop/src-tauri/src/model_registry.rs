//! Offline discovery for the current internal model layouts. This metadata
//! registry does not install, load, unload, or authenticate model artifacts.

use std::path::Path;

use nova_core::{
    models::{
        ModelAvailability, ModelContractError, ModelDescriptor, ModelId, ModelLibrary,
        ModelLibraryId, ModelLocation, RelativeModelPath,
    },
    now_ts,
};
use nova_storage::RegisteredModel;
use nova_transcribe::{default_model, is_installed as whisper_installed, model_paths};
use nova_translate::model as nllb;

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

fn nllb_availability(dir: &Path) -> ModelAvailability {
    let paths = nllb::required_artifact_paths(dir);
    if !paths.iter().any(|path| has_evidence(path)) {
        return ModelAvailability::Missing;
    }
    if !nllb::is_installed(dir) || !paths.iter().all(|path| file_has_data(path)) {
        return ModelAvailability::Unverified;
    }
    // The existing installer checks only existence. A broken config must not
    // be presented as structurally available by this registry.
    let config_ok = std::fs::read(&paths[1])
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|json| json.is_object());
    if config_ok {
        ModelAvailability::Available
    } else {
        ModelAvailability::Unverified
    }
}

fn embedding_availability(dir: &Path) -> ModelAvailability {
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
    let library = internal_library()?;
    let root = library.relative_root.resolve(app_data_dir);
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
            nllb::MODEL_ID,
            "translation",
            "ctranslate2",
            "nllb",
            nllb_availability(&root.join("nllb")),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_is_offline_and_preserves_existing_internal_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (library, missing) = discover_internal(dir.path()).unwrap();
        assert_eq!(missing.len(), 3);
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
    fn partial_and_corrupt_layouts_are_unverified() {
        let dir = tempfile::tempdir().unwrap();
        let nllb_dir = dir.path().join("models/nllb");
        std::fs::create_dir_all(&nllb_dir).unwrap();
        for path in nllb::required_artifact_paths(&nllb_dir) {
            std::fs::write(path, b"x").unwrap();
        }
        let (_, models) = discover_internal(dir.path()).unwrap();
        let nllb = models
            .iter()
            .find(|m| m.descriptor.id.as_str() == nllb::MODEL_ID)
            .unwrap();
        assert_eq!(nllb.availability, ModelAvailability::Unverified);
        std::fs::write(nllb_dir.join("config.json"), b"{}").unwrap();
        let (_, models) = discover_internal(dir.path()).unwrap();
        let nllb = models
            .iter()
            .find(|m| m.descriptor.id.as_str() == nllb::MODEL_ID)
            .unwrap();
        assert_eq!(nllb.availability, ModelAvailability::Available);
        std::fs::write(nllb_dir.join("model.bin"), b"").unwrap();
        let (_, models) = discover_internal(dir.path()).unwrap();
        let nllb = models
            .iter()
            .find(|m| m.descriptor.id.as_str() == nllb::MODEL_ID)
            .unwrap();
        assert_eq!(nllb.availability, ModelAvailability::Unverified);
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
}
