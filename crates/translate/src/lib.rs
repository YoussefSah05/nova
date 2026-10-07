//! On-device translation for Nova.
//!
//! - [`marian`]: Marian/Opus-MT models (one per direction) on the app's single
//!   ONNX Runtime. The AI22 benchmark chose them as the v1 default
//!   (`docs/evaluations/ai22-2026-10-07.md`).
//! - [`MarianTranslator`] wraps one model behind the backend-neutral
//!   [`Translator`] contract; [`PivotTranslator`] chains two of them through
//!   English for directions without a direct model (fr↔ar).
//!
//! Only committed caption text is translated; provisional text never is.
//! Callers cache results (see `transcript_translations`), so this crate only
//! computes.

pub mod marian;

use std::path::Path;
use std::sync::Arc;

use nova_core::intelligence::{EngineError, ProviderKind, TranslateBatch, Translator};
use thiserror::Error;

pub use marian::Marian;

#[derive(Debug, Error)]
pub enum TranslateError {
    #[error("could not load the translation model: {0}")]
    Load(String),
    #[error("translation failed: {0}")]
    Inference(String),
}

impl From<TranslateError> for EngineError {
    fn from(error: TranslateError) -> Self {
        match error {
            TranslateError::Load(_) => EngineError::LoadFailed,
            TranslateError::Inference(_) => EngineError::InferenceFailed,
        }
    }
}

/// Threads per model: translation runs beside playback and live ASR, so it
/// stays small. Measured at 175 ms p50 per sentence (en→fr).
const THREADS: usize = 2;

/// One Marian model for one direction.
pub struct MarianTranslator {
    source: String,
    target: String,
    model: Marian,
}

impl MarianTranslator {
    pub fn load(dir: &Path, source: &str, target: &str) -> Result<Self, TranslateError> {
        Ok(Self {
            source: source.to_string(),
            target: target.to_string(),
            model: Marian::load(dir, target, THREADS)?,
        })
    }
}

impl Translator for MarianTranslator {
    fn provider(&self) -> ProviderKind {
        ProviderKind::Local
    }

    fn supports(&self, source: &str, target: &str) -> bool {
        source == self.source && target == self.target
    }

    /// Sentence-level: Marian has no use for the preceding lines.
    fn translate(&self, batch: TranslateBatch<'_>) -> Result<Vec<String>, EngineError> {
        if !self.supports(batch.source, batch.target) {
            return Err(EngineError::UnsupportedLanguage);
        }
        batch
            .segments
            .iter()
            .map(|segment| self.model.translate(segment).map_err(EngineError::from))
            .collect()
    }
}

/// `source → pivot → target` through two translators, for a direction with
/// no direct model. Quality and latency compound, so it is chosen only when
/// no direct route exists.
pub struct PivotTranslator {
    first: Arc<dyn Translator>,
    second: Arc<dyn Translator>,
    pivot: String,
}

impl PivotTranslator {
    pub fn new(first: Arc<dyn Translator>, second: Arc<dyn Translator>, pivot: &str) -> Self {
        Self {
            first,
            second,
            pivot: pivot.to_string(),
        }
    }
}

impl Translator for PivotTranslator {
    fn provider(&self) -> ProviderKind {
        ProviderKind::Local
    }

    fn supports(&self, source: &str, target: &str) -> bool {
        self.first.supports(source, &self.pivot) && self.second.supports(&self.pivot, target)
    }

    fn translate(&self, batch: TranslateBatch<'_>) -> Result<Vec<String>, EngineError> {
        if !self.supports(batch.source, batch.target) {
            return Err(EngineError::UnsupportedLanguage);
        }
        let via = self.first.translate(TranslateBatch {
            target: &self.pivot,
            // Context is in the source and target languages, not the pivot.
            context: &[],
            ..batch
        })?;
        self.second.translate(TranslateBatch {
            source: &self.pivot,
            target: batch.target,
            context: &[],
            segments: &via,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Appends its direction to every segment.
    struct Tagging(&'static str, &'static str);
    impl Translator for Tagging {
        fn provider(&self) -> ProviderKind {
            ProviderKind::Local
        }
        fn supports(&self, source: &str, target: &str) -> bool {
            source == self.0 && target == self.1
        }
        fn translate(&self, batch: TranslateBatch<'_>) -> Result<Vec<String>, EngineError> {
            Ok(batch
                .segments
                .iter()
                .map(|s| format!("{s}>{}", self.1))
                .collect())
        }
    }

    #[test]
    fn pivot_chains_through_english_in_order() {
        let pivot = PivotTranslator::new(
            Arc::new(Tagging("fr", "en")),
            Arc::new(Tagging("en", "ar")),
            "en",
        );
        assert!(pivot.supports("fr", "ar"));
        assert!(!pivot.supports("ar", "fr"));
        let segments = vec!["bonjour".to_string(), "merci".to_string()];
        let out = pivot
            .translate(TranslateBatch {
                source: "fr",
                target: "ar",
                context: &[],
                segments: &segments,
            })
            .unwrap();
        assert_eq!(out, ["bonjour>en>ar", "merci>en>ar"]);
        assert_eq!(
            pivot.translate(TranslateBatch {
                source: "ar",
                target: "fr",
                context: &[],
                segments: &segments,
            }),
            Err(EngineError::UnsupportedLanguage)
        );
    }

    /// Real models, when available: `NOVA_TEST_OPUS_DIR=/path/to/models/opus`.
    #[test]
    fn opus_models_translate_when_present() {
        let Some(root) = std::env::var_os("NOVA_TEST_OPUS_DIR") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let en_fr = MarianTranslator::load(&root.join("opus-mt-en-fr"), "en", "fr").unwrap();
        let segments = vec!["The weather is nice today.".to_string()];
        let out = en_fr
            .translate(TranslateBatch {
                source: "en",
                target: "fr",
                context: &[],
                segments: &segments,
            })
            .unwrap();
        assert!(out[0].to_lowercase().contains("aujourd'hui"), "{out:?}");
        let en_ar = MarianTranslator::load(&root.join("opus-mt-en-ar"), "en", "ar").unwrap();
        let out = en_ar
            .translate(TranslateBatch {
                source: "en",
                target: "ar",
                context: &[],
                segments: &segments,
            })
            .unwrap();
        assert!(
            out[0]
                .chars()
                .any(|c| ('\u{0600}'..='\u{06FF}').contains(&c)),
            "Arabic output: {out:?}"
        );
    }
}
