//! Whisper model management: descriptors, on-disk layout, and first-run
//! download of the prebuilt GGML weights + Core ML encoder.
//!
//! We ship no model in the bundle. On first use, [`ensure_model`] streams the
//! GGML weights and (on macOS) the prebuilt Core ML encoder from the
//! `ggerganov/whisper.cpp` Hugging Face repo and lays them out so `whisper-cli`
//! finds the Core ML encoder automatically.
//!
//! **Naming:** whisper.cpp derives the Core ML encoder path from the GGML file
//! by stripping a `-qX_X` quantization suffix and appending `-encoder.mlmodelc`.
//! So `ggml-large-v3-turbo-q5_0.bin` pairs with `ggml-large-v3-turbo-encoder.mlmodelc`
//! — which is exactly the prebuilt artifact name on Hugging Face.

use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio::io::AsyncWriteExt;

const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("network error fetching {url}: {source}")]
    Fetch {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("download of {url} returned HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("failed to unzip Core ML model: {0}")]
    Unzip(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A selectable Whisper model. Only the default is wired in Phase 6; the struct
/// leaves room for the `large-v3` full upgrade noted in `whisper-config`.
#[derive(Debug, Clone)]
pub struct WhisperModel {
    /// Stable identifier persisted into the `transcripts.model` column.
    pub id: &'static str,
    /// GGML weights filename (also the download basename on Hugging Face).
    pub ggml_file: &'static str,
    /// Core ML encoder directory name (`.mlmodelc`), downloaded as `<name>.zip`.
    pub coreml_dir: &'static str,
}

/// The default model per `docs/reference/whisper.md`: near-`large-v3`
/// quality at ~1/3 the size. Used regardless of RAM tier in Phase 6.
pub fn default_model() -> WhisperModel {
    WhisperModel {
        id: "large-v3-turbo-q5_0",
        ggml_file: "ggml-large-v3-turbo-q5_0.bin",
        coreml_dir: "ggml-large-v3-turbo-encoder.mlmodelc",
    }
}

/// Resolved on-disk locations for a model inside `models_dir`.
#[derive(Debug, Clone)]
pub struct ModelPaths {
    pub ggml: PathBuf,
    pub coreml_dir: PathBuf,
}

pub fn model_paths(models_dir: &Path, model: &WhisperModel) -> ModelPaths {
    ModelPaths {
        ggml: models_dir.join(model.ggml_file),
        coreml_dir: models_dir.join(model.coreml_dir),
    }
}

/// Core ML is only consulted on Apple platforms; elsewhere the GGML weights
/// alone are sufficient (CPU inference).
fn coreml_required() -> bool {
    cfg!(target_os = "macos")
}

/// True when every artifact the current platform needs is already present.
pub fn is_installed(models_dir: &Path, model: &WhisperModel) -> bool {
    let paths = model_paths(models_dir, model);
    if !paths.ggml.is_file() {
        return false;
    }
    if coreml_required() && !paths.coreml_dir.is_dir() {
        return false;
    }
    true
}

/// Progress for the first-run model download, surfaced to the UI.
#[derive(Debug, Clone)]
pub struct ModelDownloadProgress {
    /// Which artifact is downloading, e.g. the GGML weights or Core ML encoder.
    pub artifact: &'static str,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

/// Ensure the model is present under `models_dir`, downloading any missing
/// artifact. Idempotent — a no-op when [`is_installed`] is already true.
///
/// `on_progress` is invoked periodically during downloads. The first
/// `whisper-cli` run after this compiles the `.mlmodelc` (~30–120s); that
/// happens inside whisper.cpp, not here.
pub async fn ensure_model(
    models_dir: &Path,
    model: &WhisperModel,
    mut on_progress: impl FnMut(ModelDownloadProgress),
) -> Result<ModelPaths, ModelError> {
    tokio::fs::create_dir_all(models_dir).await?;
    let paths = model_paths(models_dir, model);

    if !paths.ggml.is_file() {
        let url = format!("{HF_BASE}/{}", model.ggml_file);
        download_to_file(&url, &paths.ggml, "weights", &mut on_progress).await?;
    }

    if coreml_required() && !paths.coreml_dir.is_dir() {
        let zip_name = format!("{}.zip", model.coreml_dir);
        let url = format!("{HF_BASE}/{zip_name}");
        let zip_path = models_dir.join(&zip_name);
        download_to_file(&url, &zip_path, "coreml", &mut on_progress).await?;
        unzip_into(&zip_path, models_dir).await?;
        let _ = tokio::fs::remove_file(&zip_path).await;
    }

    Ok(paths)
}

/// Stream `url` to `dest`, writing to a `.part` sidecar first and renaming on
/// success so an interrupted download is never mistaken for a complete one.
async fn download_to_file(
    url: &str,
    dest: &Path,
    artifact: &'static str,
    on_progress: &mut impl FnMut(ModelDownloadProgress),
) -> Result<(), ModelError> {
    let resp = reqwest::get(url).await.map_err(|e| ModelError::Fetch {
        url: url.to_string(),
        source: e,
    })?;
    if !resp.status().is_success() {
        return Err(ModelError::Status {
            url: url.to_string(),
            status: resp.status().as_u16(),
        });
    }
    let total_bytes = resp.content_length();

    let part = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await?;
    let mut downloaded_bytes = 0u64;
    let mut resp = resp;
    loop {
        let chunk = resp.chunk().await.map_err(|e| ModelError::Fetch {
            url: url.to_string(),
            source: e,
        })?;
        let Some(chunk) = chunk else { break };
        file.write_all(&chunk).await?;
        downloaded_bytes += chunk.len() as u64;
        on_progress(ModelDownloadProgress {
            artifact,
            downloaded_bytes,
            total_bytes,
        });
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}

/// Extract `zip_path` into `dest_dir`, preserving the archive's directory
/// structure. Runs on a blocking thread — the `zip` crate is synchronous.
async fn unzip_into(zip_path: &Path, dest_dir: &Path) -> Result<(), ModelError> {
    let zip_path = zip_path.to_path_buf();
    let dest_dir = dest_dir.to_path_buf();
    tokio::task::spawn_blocking(move || extract_zip(&zip_path, &dest_dir))
        .await
        .map_err(|e| ModelError::Unzip(format!("extraction task panicked: {e}")))?
}

fn extract_zip(zip_path: &Path, dest_dir: &Path) -> Result<(), ModelError> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| ModelError::Unzip(e.to_string()))?;
    archive
        .extract(dest_dir)
        .map_err(|e| ModelError::Unzip(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coreml_encoder_name_matches_quant_stripped_ggml() {
        // whisper.cpp strips the -q5_0 suffix to find the Core ML encoder, so
        // the two descriptor names must line up on the base model name.
        let m = default_model();
        assert_eq!(m.ggml_file, "ggml-large-v3-turbo-q5_0.bin");
        assert_eq!(m.coreml_dir, "ggml-large-v3-turbo-encoder.mlmodelc");
    }

    #[test]
    fn not_installed_in_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_installed(dir.path(), &default_model()));
    }

    #[test]
    fn installed_when_ggml_present_off_macos() {
        // On non-macOS, GGML alone is enough. On macOS the Core ML dir is also
        // required, so this asserts the platform-appropriate behaviour.
        let dir = tempfile::tempdir().unwrap();
        let paths = model_paths(dir.path(), &default_model());
        std::fs::write(&paths.ggml, b"fake").unwrap();
        assert_eq!(
            is_installed(dir.path(), &default_model()),
            !coreml_required()
        );
    }
}
