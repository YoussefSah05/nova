//! Error type for the translation pipeline.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TranslateError {
    /// The NLLB model isn't installed yet (caller should prompt a download).
    #[error("translation model not installed at {0}")]
    ModelMissing(String),

    /// A source or target BCP-47 code has no NLLB-200 mapping.
    #[error("unsupported language code: {0}")]
    UnsupportedLanguage(String),

    /// Tokenizer load / encode / decode failure.
    #[error("tokenizer error: {0}")]
    Tokenize(String),

    /// CTranslate2 model load or inference failure.
    #[error("inference error: {0}")]
    Inference(String),

    /// Network error while fetching a model artifact.
    #[error("network error fetching {url}: {source}")]
    Fetch {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    /// A model download returned a non-success status.
    #[error("download of {url} returned HTTP {status}")]
    Status { url: String, status: u16 },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
