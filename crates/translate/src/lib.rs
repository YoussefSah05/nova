//! Translation pipeline.
//!
//! Phase 7 ships the **local NLLB-200 distilled-600M** backend over CTranslate2
//! (via ct2rs). DeepL / Google REST backends are deliberately deferred — they
//! drop in behind the [`Translator`] trait without touching callers.
//!
//! Per-segment caching lives in SQLite (`transcript_translations`), so
//! re-watching or switching back to a target language is instant. See
//! `docs/reference/translate-config.md` for the BCP-47 ↔ NLLB mapping, batching,
//! and caching rules.

pub mod error;
pub mod lang;
pub mod model;
pub mod nllb;

pub use error::TranslateError;
pub use lang::{to_nllb, TargetLanguage, TARGET_LANGUAGES};
pub use nllb::NllbTranslator;

/// A translation backend. Backends are synchronous (CTranslate2 inference
/// blocks); callers run them on `spawn_blocking`. `backend_id` is the stable
/// string persisted in the `transcript_translations.backend` cache column.
pub trait Translator: Send + Sync {
    fn backend_id(&self) -> &str;

    /// Translate `texts` from `src_nllb` to `tgt_nllb` (Flores-200 codes),
    /// preserving order. Implementations batch internally.
    fn translate_batch(
        &self,
        texts: &[String],
        src_nllb: &str,
        tgt_nllb: &str,
    ) -> Result<Vec<String>, TranslateError>;
}

impl Translator for NllbTranslator {
    fn backend_id(&self) -> &str {
        model::MODEL_ID
    }

    fn translate_batch(
        &self,
        texts: &[String],
        src_nllb: &str,
        tgt_nllb: &str,
    ) -> Result<Vec<String>, TranslateError> {
        NllbTranslator::translate_batch(self, texts, src_nllb, tgt_nllb)
    }
}
