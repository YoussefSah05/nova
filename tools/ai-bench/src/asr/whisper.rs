//! whisper.cpp kept resident in-process (whisper-rs): the model loads once and
//! every decode reuses the same context and state, which is the whole point of
//! the streaming design. Metal always; the Core ML encoder with `--features coreml`.

use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use super::local_agreement::Word;
use super::{BufferDecoder, Decode};

pub struct WhisperEngine {
    // The state borrows nothing from the context in whisper-rs 0.16, but the
    // context must outlive it: keep both, context last to drop.
    state: WhisperState,
    _ctx: WhisperContext,
    eot: i32,
    threads: i32,
    tuning: WhisperTuning,
    pub load_sec: f64,
}

/// Decode tuning measured as separate candidates.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct WhisperTuning {
    /// Size the encoder context to the buffer instead of always 30 s
    /// (whisper.cpp pads every decode to 30 s otherwise).
    pub adaptive_ctx: bool,
    /// Smallest adaptive context, in seconds: too small a context makes
    /// whisper loop and hallucinate.
    pub ctx_floor_sec: f64,
    /// No temperature fallback re-decodes: bounded latency per step.
    pub no_fallback: bool,
}

/// Encoder frames per second of audio (1500 frames = 30 s).
const FRAMES_PER_SEC: f64 = 50.0;

impl WhisperEngine {
    pub fn load(model: &Path, threads: i32, tuning: WhisperTuning) -> Result<Self> {
        let started = Instant::now();
        let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
            .with_context(|| format!("loading {}", model.display()))?;
        let state = ctx.create_state().context("creating whisper state")?;
        let eot = ctx.token_eot();
        Ok(Self {
            state,
            _ctx: ctx,
            eot,
            threads,
            tuning,
            load_sec: started.elapsed().as_secs_f64(),
        })
    }
}

impl BufferDecoder for WhisperEngine {
    fn decode(
        &mut self,
        pcm: &[f32],
        offset_sec: f64,
        prompt: &str,
        language: Option<&str>,
    ) -> Result<Decode> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.threads);
        params.set_language(Some(language.unwrap_or("auto")));
        params.set_token_timestamps(true);
        // The caller supplies continuity through the prompt; nothing else
        // carries over between decodes of an overlapping buffer.
        params.set_no_context(true);
        if !prompt.is_empty() {
            params.set_initial_prompt(prompt);
        }
        params.set_suppress_blank(true);
        if self.tuning.adaptive_ctx {
            // A little headroom past the buffer; never more than the full 30 s.
            let secs =
                (pcm.len() as f64 / super::SAMPLE_RATE as f64).max(self.tuning.ctx_floor_sec);
            let frames = (secs * FRAMES_PER_SEC).ceil() as i32 + 64;
            params.set_audio_ctx(frames.min(1500));
        }
        if self.tuning.no_fallback {
            params.set_temperature_inc(0.0);
        }
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        self.state
            .full(params, pcm)
            .map_err(|e| anyhow!("whisper decode failed: {e}"))?;
        let language =
            whisper_rs::get_lang_str(self.state.full_lang_id_from_state()).map(str::to_string);

        // Rebuild words from tokens: a token starting with a space begins a
        // new word. Bytes are joined before decoding, since a multibyte
        // character (Arabic) can be split across tokens.
        let mut words = Vec::new();
        let mut current: Option<(Vec<u8>, f64, f64)> = None;
        for segment in self.state.as_iter() {
            for i in 0..segment.n_tokens() {
                let Some(token) = segment.get_token(i) else {
                    continue;
                };
                if token.token_id() >= self.eot {
                    continue;
                }
                let data = token.token_data();
                let (t0, t1) = (
                    offset_sec + data.t0 as f64 / 100.0,
                    offset_sec + data.t1 as f64 / 100.0,
                );
                let bytes = token.to_bytes().unwrap_or_default();
                let starts_word = bytes.first() == Some(&b' ');
                match &mut current {
                    Some((buf, _, end)) if !starts_word => {
                        buf.extend_from_slice(bytes);
                        *end = t1;
                    }
                    _ => {
                        if let Some((buf, s, e)) = current.take() {
                            push_word(&mut words, &buf, s, e);
                        }
                        current = Some((bytes.to_vec(), t0, t1));
                    }
                }
            }
        }
        if let Some((buf, s, e)) = current.take() {
            push_word(&mut words, &buf, s, e);
        }
        Ok(Decode { words, language })
    }
}

fn push_word(words: &mut Vec<Word>, bytes: &[u8], start: f64, end: f64) {
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    if !text.is_empty() {
        words.push(Word {
            text,
            start,
            end: end.max(start),
        });
    }
}
