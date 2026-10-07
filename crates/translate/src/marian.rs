//! Encoder-decoder translation (Marian/Opus-MT) on ONNX Runtime, in process
//! on the single ORT that fastembed already links, with the pure-Rust
//! `tokenizers` crate. Greedy decoding with a KV cache through the "merged"
//! decoder (`use_cache_branch`), so each new token costs one short decoder
//! step. Ported from the AI22 benchmark harness, whose measurements
//! (`docs/evaluations/ai22-2026-10-07.md`) chose it.

use std::path::Path;

use ndarray::{Array, Array2, Array4, ArrayD, IxDyn};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::{DynValue, Value};
use serde::Deserialize;
use tokenizers::Tokenizer;

use crate::TranslateError;

/// The model files, relative to the model directory (the manifest pins them).
pub const CONFIG: &str = "config.json";
pub const TOKENIZER: &str = "tokenizer.json";
pub const ENCODER: &str = "onnx/encoder_model_quantized.onnx";
pub const DECODER: &str = "onnx/decoder_model_merged_quantized.onnx";

/// The cache tensors of each decoder layer, in the model's input order.
const CACHE_PARTS: [&str; 4] = [
    "decoder.key",
    "decoder.value",
    "encoder.key",
    "encoder.value",
];

#[derive(Deserialize)]
struct Config {
    d_model: usize,
    decoder_layers: usize,
    decoder_attention_heads: usize,
    #[serde(default)]
    decoder_start_token_id: Option<i64>,
    eos_token_id: i64,
    #[serde(default)]
    pad_token_id: Option<i64>,
}

pub struct Marian {
    encoder: Session,
    decoder: Session,
    tokenizer: Tokenizer,
    config: Config,
    /// Target-language token that multi-target Marian models need at the
    /// start of the source (e.g. `>>ara<<` for opus-mt-en-ar).
    source_prefix: Option<i64>,
}

fn ort(error: ort::Error) -> TranslateError {
    TranslateError::Inference(error.to_string())
}

fn shape(error: ndarray::ShapeError) -> TranslateError {
    TranslateError::Inference(error.to_string())
}

fn session(path: &Path, threads: usize) -> Result<Session, TranslateError> {
    let load = |e: ort::Error| TranslateError::Load(format!("{}: {e}", path.display()));
    Session::builder()
        .map_err(load)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(load)?
        .with_intra_threads(threads)
        .map_err(load)?
        .commit_from_file(path)
        .map_err(load)
}

/// Load `tokenizer.json`. Some converted Marian files carry a `Precompiled`
/// normalizer whose character map is null (there is no table), which the Rust
/// crate refuses; such a normalizer does nothing, so it is dropped.
fn load_tokenizer(path: &Path) -> Result<Tokenizer, TranslateError> {
    let load = |e: String| TranslateError::Load(format!("{}: {e}", path.display()));
    let text = std::fs::read_to_string(path).map_err(|e| load(e.to_string()))?;
    let mut json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| load(e.to_string()))?;
    let empty_precompiled = |n: &serde_json::Value| {
        n.get("type").and_then(|t| t.as_str()) == Some("Precompiled")
            && n.get("precompiled_charsmap").is_some_and(|m| m.is_null())
    };
    if let Some(normalizer) = json.get_mut("normalizer") {
        if empty_precompiled(normalizer) {
            *normalizer = serde_json::Value::Null;
        } else if let Some(list) = normalizer
            .get_mut("normalizers")
            .and_then(|l| l.as_array_mut())
        {
            list.retain(|n| !empty_precompiled(n));
        }
    }
    let bytes = serde_json::to_vec(&json).map_err(|e| load(e.to_string()))?;
    Tokenizer::from_bytes(bytes).map_err(|e| load(e.to_string()))
}

/// The ISO 639-3 code Marian multi-target models use in `>>xxx<<` tags.
fn marian_tag(language: &str) -> Option<&'static str> {
    Some(match language {
        "en" => "eng",
        "fr" => "fra",
        "ar" => "ara",
        _ => return None,
    })
}

impl Marian {
    /// `dir` holds [`CONFIG`], [`TOKENIZER`], [`ENCODER`] and [`DECODER`].
    /// `target` selects the output language of a multi-target model; it is
    /// ignored by single-target models, which have no language tags.
    pub fn load(dir: &Path, target: &str, threads: usize) -> Result<Self, TranslateError> {
        let config_text = std::fs::read_to_string(dir.join(CONFIG))
            .map_err(|e| TranslateError::Load(format!("{CONFIG}: {e}")))?;
        let config: Config = serde_json::from_str(&config_text)
            .map_err(|e| TranslateError::Load(format!("{CONFIG}: {e}")))?;
        if config.decoder_attention_heads == 0
            || !config
                .d_model
                .is_multiple_of(config.decoder_attention_heads)
        {
            return Err(TranslateError::Load(format!(
                "{CONFIG}: inconsistent dimensions"
            )));
        }
        let tokenizer = load_tokenizer(&dir.join(TOKENIZER))?;
        let source_prefix = marian_tag(target)
            .and_then(|tag| tokenizer.token_to_id(&format!(">>{tag}<<")))
            .map(i64::from);
        Ok(Self {
            encoder: session(&dir.join(ENCODER), threads)?,
            decoder: session(&dir.join(DECODER), threads)?,
            tokenizer,
            config,
            source_prefix,
        })
    }

    fn source_ids(&self, text: &str) -> Result<Vec<i64>, TranslateError> {
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|e| TranslateError::Inference(e.to_string()))?;
        let mut ids: Vec<i64> = encoding.get_ids().iter().map(|&i| i64::from(i)).collect();
        if let Some(prefix) = self.source_prefix {
            ids.insert(0, prefix);
        }
        ids.push(self.config.eos_token_id);
        Ok(ids)
    }

    /// Translate one segment (greedy).
    pub fn translate(&self, text: &str) -> Result<String, TranslateError> {
        if text.trim().is_empty() {
            return Ok(String::new());
        }
        let ids = self.source_ids(text)?;
        let n = ids.len();
        let input_ids = Array2::from_shape_vec((1, n), ids).map_err(shape)?;
        let mask = Array2::<i64>::ones((1, n));
        let encoded = self
            .encoder
            .run(vec![
                (
                    "input_ids",
                    Value::from_array(input_ids).map_err(ort)?.into_dyn(),
                ),
                (
                    "attention_mask",
                    Value::from_array(mask.clone()).map_err(ort)?.into_dyn(),
                ),
            ])
            .map_err(ort)?;
        let hidden: ArrayD<f32> = encoded["last_hidden_state"]
            .try_extract_tensor::<f32>()
            .map_err(ort)?
            .to_owned();

        let start = self
            .config
            .decoder_start_token_id
            .unwrap_or(self.config.eos_token_id);
        let heads = self.config.decoder_attention_heads;
        let head_dim = self.config.d_model / heads;
        let layers = self.config.decoder_layers;
        // Past keys/values: placeholders on the first step (the merged decoder
        // ignores them when `use_cache_branch` is false, and ONNX Runtime
        // rejects zero-length tensors), then the decoder's presents.
        let placeholder = || Array4::<f32>::zeros((1, heads, 1, head_dim)).into_dyn();
        let mut past: Vec<ArrayD<f32>> = (0..layers * 4).map(|_| placeholder()).collect();
        let mut generated: Vec<u32> = Vec::new();
        let mut step_ids = vec![start];
        // Bounded output: a degenerate loop never runs past this.
        let max_len = 3 * n + 16;
        for step in 0..max_len {
            let mut inputs: Vec<(String, DynValue)> = vec![
                (
                    "input_ids".into(),
                    Value::from_array(
                        Array2::from_shape_vec((1, step_ids.len()), step_ids.clone())
                            .map_err(shape)?,
                    )
                    .map_err(ort)?
                    .into_dyn(),
                ),
                (
                    "encoder_hidden_states".into(),
                    Value::from_array(hidden.clone()).map_err(ort)?.into_dyn(),
                ),
                (
                    "encoder_attention_mask".into(),
                    Value::from_array(mask.clone()).map_err(ort)?.into_dyn(),
                ),
                (
                    "use_cache_branch".into(),
                    Value::from_array(
                        Array::from_shape_vec(IxDyn(&[1]), vec![step > 0]).map_err(shape)?,
                    )
                    .map_err(ort)?
                    .into_dyn(),
                ),
            ];
            for layer in 0..layers {
                for (k, part) in CACHE_PARTS.iter().enumerate() {
                    inputs.push((
                        format!("past_key_values.{layer}.{part}"),
                        Value::from_array(past[layer * 4 + k].clone())
                            .map_err(ort)?
                            .into_dyn(),
                    ));
                }
            }
            let out = self.decoder.run(inputs).map_err(ort)?;
            let logits = out["logits"].try_extract_tensor::<f32>().map_err(ort)?;
            let vocab = logits.shape()[2];
            let last = logits.shape()[1] - 1;
            let mut best = (f32::NEG_INFINITY, 0i64);
            for v in 0..vocab {
                if Some(v as i64) == self.config.pad_token_id {
                    continue;
                }
                let score = logits[[0, last, v]];
                if score > best.0 {
                    best = (score, v as i64);
                }
            }
            let next = best.1;
            for layer in 0..layers {
                for (k, part) in CACHE_PARTS.iter().enumerate() {
                    // The encoder cross-attention cache is computed on the
                    // first step only; later steps return it empty.
                    if k < 2 || step == 0 {
                        let present = out[format!("present.{layer}.{part}").as_str()]
                            .try_extract_tensor::<f32>()
                            .map_err(ort)?;
                        past[layer * 4 + k] = present.to_owned();
                    }
                }
            }
            if next == self.config.eos_token_id {
                break;
            }
            generated.push(next as u32);
            step_ids = vec![next];
        }
        self.tokenizer
            .decode(&generated, true)
            .map(|s| s.trim().to_string())
            .map_err(|e| TranslateError::Inference(e.to_string()))
    }
}
