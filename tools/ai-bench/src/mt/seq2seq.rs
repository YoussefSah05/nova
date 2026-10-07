//! Encoder-decoder translation on ONNX Runtime (in-process, the app's single
//! ORT) with the pure-Rust `tokenizers` crate: Marian/Opus-MT and NLLB share
//! this code. Greedy decoding with a KV cache through the "merged" decoder
//! (`use_cache_branch`), so each new token costs one short decoder step.

use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use ndarray::{Array, Array2, Array4, ArrayD, IxDyn};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::{DynValue, Value};
use serde::Deserialize;
use tokenizers::Tokenizer;

/// How the model is told which languages to translate between.
#[derive(Debug, Clone)]
pub enum Family {
    /// One model per direction; nothing to specify.
    Marian,
    /// Multilingual: source language code as prefix, target forced first.
    Nllb { source: String, target: String },
}

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

pub struct Seq2Seq {
    encoder: Session,
    decoder: Session,
    tokenizer: Tokenizer,
    config: Config,
    family: Family,
    /// Target-language token some multi-target Marian models need at the
    /// start of the source (e.g. `>>ara<<` for opus-mt-en-ar).
    source_prefix: Option<i64>,
    pub load_sec: f64,
}

fn session(path: &Path, threads: usize) -> Result<Session> {
    Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(threads)?
        .commit_from_file(path)
        .with_context(|| format!("loading {}", path.display()))
}

/// Load `tokenizer.json`. Some converted Marian files carry a `Precompiled`
/// normalizer whose character map is null (there is no table), which the Rust
/// crate refuses; such a normalizer does nothing, so it is dropped.
fn load_tokenizer(path: &Path) -> Result<Tokenizer> {
    let mut json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
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
    Tokenizer::from_bytes(serde_json::to_vec(&json)?)
        .map_err(|e| anyhow!("tokenizer {}: {e}", path.display()))
}

/// NLLB (FLORES-200) code for a two-letter language.
pub fn nllb_code(lang: &str) -> Result<&'static str> {
    Ok(match lang {
        "en" => "eng_Latn",
        "fr" => "fra_Latn",
        "ar" => "arb_Arab",
        other => bail!("no NLLB code for {other}"),
    })
}

impl Seq2Seq {
    /// `dir` holds `config.json`, `tokenizer.json` and `onnx/` with the
    /// quantized encoder and merged decoder.
    pub fn load(dir: &Path, family: Family, threads: usize) -> Result<Self> {
        let started = Instant::now();
        let config: Config =
            serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let tokenizer = load_tokenizer(&dir.join("tokenizer.json"))?;
        let encoder = session(&dir.join("onnx/encoder_model_quantized.onnx"), threads)?;
        let decoder = session(
            &dir.join("onnx/decoder_model_merged_quantized.onnx"),
            threads,
        )?;
        Ok(Self {
            encoder,
            decoder,
            tokenizer,
            config,
            family,
            source_prefix: None,
            load_sec: started.elapsed().as_secs_f64(),
        })
    }

    /// Whether the vocabulary has `token` (e.g. a Marian `>>ara<<` tag).
    pub fn has_token(&self, token: &str) -> bool {
        self.tokenizer.token_to_id(token).is_some()
    }

    /// Prepend `token` to every source (multi-target Marian models).
    pub fn set_source_prefix(&mut self, token: &str) -> Result<()> {
        self.source_prefix = Some(self.token_id(token)?);
        Ok(())
    }

    /// Switch an NLLB model's direction without reloading it.
    pub fn set_direction(&mut self, source: &str, target: &str) {
        if let Family::Nllb { .. } = self.family {
            self.family = Family::Nllb {
                source: source.into(),
                target: target.into(),
            };
        }
    }

    fn token_id(&self, token: &str) -> Result<i64> {
        self.tokenizer
            .token_to_id(token)
            .map(i64::from)
            .ok_or_else(|| anyhow!("token {token} not in vocabulary"))
    }

    fn source_ids(&self, text: &str) -> Result<Vec<i64>> {
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|e| anyhow!("encode: {e}"))?;
        let mut ids: Vec<i64> = encoding.get_ids().iter().map(|&i| i64::from(i)).collect();
        if let Family::Nllb { source, .. } = &self.family {
            ids.insert(0, self.token_id(nllb_code(source)?)?);
        }
        if let Some(prefix) = self.source_prefix {
            ids.insert(0, prefix);
        }
        ids.push(self.config.eos_token_id);
        Ok(ids)
    }

    /// Translate one segment (greedy).
    pub fn translate(&mut self, text: &str) -> Result<String> {
        let ids = self.source_ids(text)?;
        let n = ids.len();
        let input_ids = Array2::from_shape_vec((1, n), ids)?;
        let mask = Array2::<i64>::ones((1, n));
        let encoded = self.encoder.run(vec![
            ("input_ids", Value::from_array(input_ids)?.into_dyn()),
            (
                "attention_mask",
                Value::from_array(mask.clone())?.into_dyn(),
            ),
        ])?;
        let hidden: ArrayD<f32> = encoded["last_hidden_state"]
            .try_extract_tensor::<f32>()?
            .to_owned();

        let start = self
            .config
            .decoder_start_token_id
            .unwrap_or(self.config.eos_token_id);
        let mut prefix = vec![start];
        if let Family::Nllb { target, .. } = &self.family {
            prefix.push(self.token_id(nllb_code(target)?)?);
        }
        let heads = self.config.decoder_attention_heads;
        let head_dim = self.config.d_model / heads;
        let layers = self.config.decoder_layers;
        // Past keys/values: placeholders on the first step (the merged decoder
        // ignores them when `use_cache_branch` is false, and ONNX Runtime
        // rejects zero-length tensors), then the decoder's presents.
        let placeholder = || Array4::<f32>::zeros((1, heads, 1, head_dim)).into_dyn();
        let mut past: Vec<ArrayD<f32>> = (0..layers * 4).map(|_| placeholder()).collect();
        let mut generated: Vec<i64> = Vec::new();
        let mut step_ids = prefix.clone();
        let max_len = 3 * n + 16;
        for step in 0..max_len {
            let mut inputs: Vec<(String, DynValue)> = vec![
                (
                    "input_ids".into(),
                    Value::from_array(Array2::from_shape_vec(
                        (1, step_ids.len()),
                        step_ids.clone(),
                    )?)?
                    .into_dyn(),
                ),
                (
                    "encoder_hidden_states".into(),
                    Value::from_array(hidden.clone())?.into_dyn(),
                ),
                (
                    "encoder_attention_mask".into(),
                    Value::from_array(mask.clone())?.into_dyn(),
                ),
                (
                    "use_cache_branch".into(),
                    Value::from_array(Array::from_shape_vec(IxDyn(&[1]), vec![step > 0])?)?
                        .into_dyn(),
                ),
            ];
            for layer in 0..layers {
                for (k, part) in [
                    "decoder.key",
                    "decoder.value",
                    "encoder.key",
                    "encoder.value",
                ]
                .iter()
                .enumerate()
                {
                    inputs.push((
                        format!("past_key_values.{layer}.{part}"),
                        Value::from_array(past[layer * 4 + k].clone())?.into_dyn(),
                    ));
                }
            }
            let out = self.decoder.run(inputs)?;
            let logits = out["logits"].try_extract_tensor::<f32>()?;
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
                for (k, part) in [
                    "decoder.key",
                    "decoder.value",
                    "encoder.key",
                    "encoder.value",
                ]
                .iter()
                .enumerate()
                {
                    // The encoder cross-attention cache is computed on the
                    // first step only; later steps return it empty.
                    if k < 2 || step == 0 {
                        let present = out[format!("present.{layer}.{part}").as_str()]
                            .try_extract_tensor::<f32>()?;
                        past[layer * 4 + k] = present.to_owned();
                    }
                }
            }
            if next == self.config.eos_token_id {
                break;
            }
            generated.push(next);
            step_ids = vec![next];
        }
        let ids: Vec<u32> = generated.iter().map(|&i| i as u32).collect();
        self.tokenizer
            .decode(&ids, true)
            .map(|s| s.trim().to_string())
            .map_err(|e| anyhow!("decode: {e}"))
    }
}
