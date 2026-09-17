//! NLLB-200 model management: on-disk layout + first-run download of a
//! **pre-converted CTranslate2 int8** model plus its SentencePiece tokenizer.
//!
//! We ship no model in the bundle (mirrors `crates/transcribe`'s whisper model).
//! On first use, [`ensure_model`] streams the CTranslate2 artifacts
//! (`model.bin` / `config.json` / shared vocabulary) from a pre-converted Hugging
//! Face repo and the shared `sentencepiece.bpe.model` from the original
//! `facebook/nllb-200-distilled-600M` repo, into `models_dir`.
//!
//! NOTE: the HF source repos + artifact filenames are centralized here so a 404
//! during on-device first-run is a one-line fix, not a code change elsewhere.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

use crate::error::TranslateError;

/// Stable id persisted alongside cached translations (the `backend` column).
pub const MODEL_ID: &str = "nllb-200-distilled-600M";

/// Pre-converted CTranslate2 (int8) weights + vocabulary.
const CT2_BASE: &str =
    "https://huggingface.co/JustFrederik/nllb-200-distilled-600M-ct2-int8/resolve/main";
/// Original model repo — the SentencePiece tokenizer lives here verbatim.
const SPM_BASE: &str = "https://huggingface.co/facebook/nllb-200-distilled-600M/resolve/main";

/// The CTranslate2 weights file (also the installed-marker).
const MODEL_BIN: &str = "model.bin";
const CONFIG_JSON: &str = "config.json";
const SPM_FILE: &str = "sentencepiece.bpe.model";
/// Converter output names the shared vocab `.json` (newer) or `.txt` (older);
/// we accept whichever the repo serves.
const VOCAB_CANDIDATES: &[&str] = &["shared_vocabulary.json", "shared_vocabulary.txt"];

/// True when every artifact CTranslate2 + the tokenizer need is already present.
pub fn is_installed(models_dir: &Path) -> bool {
    models_dir.join(MODEL_BIN).is_file()
        && models_dir.join(CONFIG_JSON).is_file()
        && models_dir.join(SPM_FILE).is_file()
        && VOCAB_CANDIDATES
            .iter()
            .any(|v| models_dir.join(v).is_file())
}

/// Progress for the first-run model download, surfaced to the UI.
#[derive(Debug, Clone)]
pub struct ModelDownloadProgress {
    pub artifact: &'static str,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

/// Ensure the model is present under `models_dir`, downloading any missing
/// artifact. Idempotent — a no-op when [`is_installed`] is already true.
pub async fn ensure_model(
    models_dir: &Path,
    mut on_progress: impl FnMut(ModelDownloadProgress),
) -> Result<(), TranslateError> {
    tokio::fs::create_dir_all(models_dir).await?;

    if !models_dir.join(MODEL_BIN).is_file() {
        download_to_file(
            &format!("{CT2_BASE}/{MODEL_BIN}"),
            &models_dir.join(MODEL_BIN),
            "weights",
            &mut on_progress,
        )
        .await?;
    }
    if !models_dir.join(CONFIG_JSON).is_file() {
        download_to_file(
            &format!("{CT2_BASE}/{CONFIG_JSON}"),
            &models_dir.join(CONFIG_JSON),
            "config",
            &mut on_progress,
        )
        .await?;
    }
    if !VOCAB_CANDIDATES
        .iter()
        .any(|v| models_dir.join(v).is_file())
    {
        // Try each candidate; the first that returns 200 wins.
        let mut got = false;
        for v in VOCAB_CANDIDATES {
            let url = format!("{CT2_BASE}/{v}");
            match download_to_file(&url, &models_dir.join(v), "vocabulary", &mut on_progress).await
            {
                Ok(()) => {
                    got = true;
                    break;
                }
                Err(TranslateError::Status { status: 404, .. }) => continue,
                Err(e) => return Err(e),
            }
        }
        if !got {
            return Err(TranslateError::Status {
                url: format!("{CT2_BASE}/{}", VOCAB_CANDIDATES[0]),
                status: 404,
            });
        }
    }
    if !models_dir.join(SPM_FILE).is_file() {
        download_to_file(
            &format!("{SPM_BASE}/{SPM_FILE}"),
            &models_dir.join(SPM_FILE),
            "tokenizer",
            &mut on_progress,
        )
        .await?;
    }
    Ok(())
}

/// Path to the SentencePiece model (used by `nllb::NllbTranslator`).
pub fn spm_path(models_dir: &Path) -> PathBuf {
    models_dir.join(SPM_FILE)
}

/// Stream `url` to `dest` via a `.part` sidecar, renaming on success so an
/// interrupted download is never mistaken for a complete one.
async fn download_to_file(
    url: &str,
    dest: &Path,
    artifact: &'static str,
    on_progress: &mut impl FnMut(ModelDownloadProgress),
) -> Result<(), TranslateError> {
    let resp = reqwest::get(url).await.map_err(|e| TranslateError::Fetch {
        url: url.to_string(),
        source: e,
    })?;
    if !resp.status().is_success() {
        return Err(TranslateError::Status {
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
        let chunk = resp.chunk().await.map_err(|e| TranslateError::Fetch {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_installed_in_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_installed(dir.path()));
    }

    #[test]
    fn installed_requires_all_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        for f in [MODEL_BIN, CONFIG_JSON, SPM_FILE, "shared_vocabulary.json"] {
            std::fs::write(dir.path().join(f), b"x").unwrap();
        }
        assert!(is_installed(dir.path()));
        // Remove the tokenizer → no longer installed.
        std::fs::remove_file(dir.path().join(SPM_FILE)).unwrap();
        assert!(!is_installed(dir.path()));
    }
}
