//! Local NLLB-200 translator over CTranslate2 (via ct2rs / `ruy` CPU on Apple
//! Silicon — CTranslate2 has no Metal GPU device, so "Metal" in the spec means
//! the optimized ARM CPU path).
//!
//! NLLB needs the **source** language token prepended to the source token stream
//! and the **target** language token as a decoder prefix. The ct2rs high-level
//! wrapper can't inject the source-lang token (it would default to the
//! tokenizer's baked-in language, ≈ `eng_Latn`), which would mistranslate
//! non-English IPTV. So we tokenize with SentencePiece ourselves and drive the
//! sys-level translator with the documented CTranslate2 NLLB recipe:
//!   source       = [<src_lang>] + sentencepiece(text) + [</s>]
//!   target_prefix = [<tgt_lang>]
//! (the SP `encode` already appends `</s>`).

use std::path::Path;

use ct2rs::sys::{Config, Device, TranslationOptions, Translator as SysTranslator};
use ct2rs::tokenizers::sentencepiece::Tokenizer as SpTokenizer;
use ct2rs::Tokenizer as _;

use crate::error::TranslateError;
use crate::model;

/// Max segments per inference call (translation batching rule).
pub const MAX_BATCH: usize = 32;

pub struct NllbTranslator {
    translator: SysTranslator,
    sp: SpTokenizer,
}

impl NllbTranslator {
    /// Load the CTranslate2 model + SentencePiece tokenizer from `models_dir`.
    pub fn load(models_dir: &Path) -> Result<Self, TranslateError> {
        if !model::is_installed(models_dir) {
            return Err(TranslateError::ModelMissing(models_dir.display().to_string()));
        }
        let spm = model::spm_path(models_dir);
        // NLLB shares one SentencePiece model for both sides.
        let sp = SpTokenizer::from_file(&spm, &spm)
            .map_err(|e| TranslateError::Tokenize(e.to_string()))?;
        let translator = SysTranslator::new(
            models_dir,
            &Config {
                device: Device::CPU,
                ..Default::default()
            },
        )
        .map_err(|e| TranslateError::Inference(e.to_string()))?;
        Ok(Self { translator, sp })
    }

    /// Translate `texts` from `src_nllb` to `tgt_nllb` (Flores-200 codes), in
    /// batches of [`MAX_BATCH`]. Output order matches input. Blocking — call from
    /// `spawn_blocking`.
    pub fn translate_batch(
        &self,
        texts: &[String],
        src_nllb: &str,
        tgt_nllb: &str,
    ) -> Result<Vec<String>, TranslateError> {
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(MAX_BATCH) {
            let sources: Vec<Vec<String>> = chunk
                .iter()
                .map(|t| {
                    let mut toks = vec![src_nllb.to_string()];
                    toks.extend(
                        self.sp
                            .encode(t)
                            .map_err(|e| TranslateError::Tokenize(e.to_string()))?,
                    );
                    Ok(toks)
                })
                .collect::<Result<_, TranslateError>>()?;
            let target_prefixes: Vec<Vec<String>> = vec![vec![tgt_nllb.to_string()]; chunk.len()];

            // beam_size 1 keeps per-line latency in the ~100–300ms band the spec
            // budgets; quality at 600M is already good.
            let opts = TranslationOptions {
                beam_size: 1,
                ..Default::default()
            };
            let results = self
                .translator
                .translate_batch_with_target_prefix(&sources, &target_prefixes, &opts, None)
                .map_err(|e| TranslateError::Inference(e.to_string()))?;

            for r in results {
                let hyp = r
                    .hypotheses
                    .into_iter()
                    .next()
                    .ok_or_else(|| TranslateError::Inference("no hypothesis returned".into()))?;
                // Drop the target-lang prefix token and any trailing EOS, then
                // detokenize back to text.
                let toks: Vec<String> = hyp
                    .into_iter()
                    .skip(1)
                    .filter(|t| t != "</s>")
                    .collect();
                let text = self
                    .sp
                    .decode(toks)
                    .map_err(|e| TranslateError::Tokenize(e.to_string()))?;
                out.push(text);
            }
        }
        Ok(out)
    }
}
