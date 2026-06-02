//! Embedding backend. `Embedder` is the seam the pipeline depends on so jobs +
//! tests can use a mock; `FastEmbedder` is the on-device fastembed/ONNX impl.

use std::path::Path;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::EnrichError;

/// The model used for catalog text. Multilingual (worldwide IPTV), small + fast
/// (~118M, 384-dim). e5 models expect a task prefix; we use `passage:` for titles.
pub const MODEL: EmbeddingModel = EmbeddingModel::MultilingualE5Small;

/// Produces embedding vectors for short catalog strings. `Send + Sync` so it can
/// live in a background job behind an `Arc`.
pub trait Embedder: Send + Sync {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EnrichError>;
}

/// On-device embedder (fastembed + ONNX Runtime). The model is downloaded to
/// `cache_dir` on first load (mirrors the whisper model flow) and reused after.
pub struct FastEmbedder {
    model: TextEmbedding,
}

impl FastEmbedder {
    /// Load (downloading on first use) the embedding model into `cache_dir`.
    pub fn load(cache_dir: &Path, show_progress: bool) -> Result<Self, EnrichError> {
        let opts = InitOptions::new(MODEL)
            .with_cache_dir(cache_dir.to_path_buf())
            .with_show_download_progress(show_progress);
        let model = TextEmbedding::try_new(opts).map_err(|e| EnrichError::Model(e.to_string()))?;
        Ok(Self { model })
    }
}

impl Embedder for FastEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EnrichError> {
        let docs: Vec<String> = texts.iter().map(|t| format!("passage: {t}")).collect();
        self.model
            .embed(docs, None)
            .map_err(|e| EnrichError::Model(e.to_string()))
    }
}
