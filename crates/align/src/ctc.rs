//! CTC forced alignment: given the frame-level character probabilities of a
//! wav2vec2 CTC model and the transcript Whisper produced, find the most
//! likely time of every character, and so of every word.
//!
//! This is the second half of the WhisperX recipe: Whisper decides *what*
//! was said, and a model trained frame by frame on characters decides
//! *when*. Whisper's own timestamps come from its decoder (heuristic offsets,
//! or DTW over cross-attention), which drift by hundreds of milliseconds on
//! long words and after pauses; CTC emissions are tied to the audio frames.
//!
//! The search is the standard Viterbi pass over the CTC lattice with a blank
//! between every two tokens (the same semantics as torchaudio's
//! `forced_align`). It is exact, runs in O(frames × tokens) time, and keeps
//! one byte of back-pointer per lattice cell.

use std::path::Path;

use ndarray::Array2;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Value;

use crate::romanize::{BLANK, VOCAB_SIZE};
use crate::AlignError;

/// Audio sample rate the model expects.
pub const SAMPLE_RATE: usize = 16_000;
/// Longest buffer aligned in one pass. Self-attention cost grows with the
/// square of the length; caption windows are at most about 33 s.
pub const MAX_SAMPLES: usize = 45 * SAMPLE_RATE;

/// Frame-level log-probabilities over the aligner vocabulary.
#[derive(Debug, Clone)]
pub struct Emissions {
    /// `frames × VOCAB_SIZE`, log-softmax normalized.
    pub log_probs: Array2<f32>,
    /// Seconds covered by one frame (about 20 ms).
    pub frame_sec: f64,
}

impl Emissions {
    pub fn frames(&self) -> usize {
        self.log_probs.nrows()
    }
}

/// The wav2vec2 CTC model that produces [`Emissions`].
pub struct CtcModel {
    session: Session,
}

impl CtcModel {
    pub fn load(path: &Path, threads: usize) -> Result<Self, AlignError> {
        let load = |e: ort::Error| AlignError::Load(format!("{}: {e}", path.display()));
        let session = Session::builder()
            .map_err(load)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(load)?
            .with_intra_threads(threads)
            .map_err(load)?
            .commit_from_file(path)
            .map_err(load)?;
        Ok(Self { session })
    }

    /// Emissions for 16 kHz mono `pcm`. The feature extractor's
    /// normalization (zero mean, unit variance) is applied here.
    pub fn emissions(&self, pcm: &[f32]) -> Result<Emissions, AlignError> {
        if pcm.len() < SAMPLE_RATE / 10 {
            return Err(AlignError::TooShort);
        }
        if pcm.len() > MAX_SAMPLES {
            return Err(AlignError::TooLong);
        }
        let n = pcm.len() as f64;
        let mean = pcm.iter().map(|&x| x as f64).sum::<f64>() / n;
        let var = pcm.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / n;
        let scale = 1.0 / (var + 1e-7).sqrt();
        let input: Vec<f32> = pcm
            .iter()
            .map(|&x| ((x as f64 - mean) * scale) as f32)
            .collect();
        let input = Array2::from_shape_vec((1, pcm.len()), input)
            .map_err(|e| AlignError::Inference(e.to_string()))?;
        let infer = |e: ort::Error| AlignError::Inference(e.to_string());
        let outputs = self
            .session
            .run(vec![(
                "input_values",
                Value::from_array(input).map_err(infer)?.into_dyn(),
            )])
            .map_err(infer)?;
        let logits = outputs["logits"]
            .try_extract_tensor::<f32>()
            .map_err(infer)?;
        let shape = logits.shape().to_vec();
        if shape.len() != 3 || shape[0] != 1 || shape[2] != VOCAB_SIZE || shape[1] == 0 {
            return Err(AlignError::Inference(format!(
                "unexpected logits shape {shape:?}"
            )));
        }
        let frames = shape[1];
        let mut log_probs = Array2::<f32>::zeros((frames, VOCAB_SIZE));
        for t in 0..frames {
            let row: Vec<f32> = (0..VOCAB_SIZE).map(|v| logits[[0, t, v]]).collect();
            let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let sum: f32 = row.iter().map(|&x| (x - max).exp()).sum();
            let log_sum = max + sum.ln();
            for (v, x) in row.into_iter().enumerate() {
                log_probs[[t, v]] = x - log_sum;
            }
        }
        Ok(Emissions {
            log_probs,
            frame_sec: pcm.len() as f64 / SAMPLE_RATE as f64 / frames as f64,
        })
    }
}

/// Where one token of the transcript was spoken.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenSpan {
    /// First and last frame (inclusive) the token occupies.
    pub first: usize,
    pub last: usize,
    /// Mean log-probability of the token over its frames.
    pub score: f32,
}

const STAY: u8 = 0;
const STEP: u8 = 1;
const SKIP: u8 = 2;

/// Viterbi-align `tokens` (no blanks) to `emissions`. Returns one span per
/// token, or `None` when the audio is too short to hold the text (CTC needs
/// a frame per token, plus a blank between two equal neighbours).
pub fn force_align(emissions: &Emissions, tokens: &[u32]) -> Option<Vec<TokenSpan>> {
    let frames = emissions.frames();
    let lp = &emissions.log_probs;
    if tokens.is_empty() {
        return Some(Vec::new());
    }
    let repeats = tokens.windows(2).filter(|w| w[0] == w[1]).count();
    if frames < tokens.len() + repeats {
        return None;
    }
    // Lattice states: even = blank, odd = tokens[(s - 1) / 2].
    let states = 2 * tokens.len() + 1;
    let label = |s: usize| {
        if s.is_multiple_of(2) {
            BLANK
        } else {
            tokens[s / 2]
        }
    };
    let neg = f32::NEG_INFINITY;
    let mut prev = vec![neg; states];
    let mut next = vec![neg; states];
    let mut back = vec![STAY; frames * states];
    prev[0] = lp[[0, BLANK as usize]];
    prev[1] = lp[[0, tokens[0] as usize]];
    for t in 1..frames {
        // States that cannot reach the end in the remaining frames, or
        // that could not have been reached yet, stay at -inf.
        for s in 0..states {
            let mut best = prev[s];
            let mut how = STAY;
            if s >= 1 && prev[s - 1] > best {
                best = prev[s - 1];
                how = STEP;
            }
            if s >= 2 && s % 2 == 1 && label(s) != label(s - 2) && prev[s - 2] > best {
                best = prev[s - 2];
                how = SKIP;
            }
            next[s] = if best == neg {
                neg
            } else {
                best + lp[[t, label(s) as usize]]
            };
            back[t * states + s] = how;
        }
        std::mem::swap(&mut prev, &mut next);
    }
    let last = states - 1;
    let mut s = if prev[last] >= prev[last - 1] {
        last
    } else {
        last - 1
    };
    if prev[s] == neg {
        return None;
    }
    // Walk back, recording for each token the frames spent in its state.
    let mut spans: Vec<Option<(usize, usize, f32, usize)>> = vec![None; tokens.len()];
    for t in (0..frames).rev() {
        if s % 2 == 1 {
            let k = s / 2;
            let score = lp[[t, tokens[k] as usize]];
            spans[k] = Some(match spans[k] {
                None => (t, t, score, 1),
                Some((_, last, sum, n)) => (t, last, sum + score, n + 1),
            });
        }
        if t == 0 {
            break;
        }
        match back[t * states + s] {
            STEP => s -= 1,
            SKIP => s -= 2,
            _ => {}
        }
    }
    spans
        .into_iter()
        .map(|span| {
            span.map(|(first, last, sum, n)| TokenSpan {
                first,
                last,
                score: sum / n as f32,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Emissions where `peaks[t]` is the confident label of frame `t`.
    fn emissions(peaks: &[u32]) -> Emissions {
        let mut log_probs = Array2::from_elem((peaks.len(), VOCAB_SIZE), (0.01f32).ln());
        for (t, &p) in peaks.iter().enumerate() {
            log_probs[[t, p as usize]] = (0.9f32).ln();
        }
        Emissions {
            log_probs,
            frame_sec: 0.02,
        }
    }

    #[test]
    fn tokens_land_on_their_peaks() {
        // blank blank a a blank b blank blank
        let e = emissions(&[0, 0, 4, 4, 0, 20, 0, 0]);
        let spans = force_align(&e, &[4, 20]).unwrap();
        assert_eq!((spans[0].first, spans[0].last), (2, 3));
        assert_eq!((spans[1].first, spans[1].last), (5, 5));
        assert!(spans[0].score > -0.2);
    }

    #[test]
    fn repeated_letters_need_a_blank_between_them() {
        let e = emissions(&[0, 15, 0, 15, 0]);
        let spans = force_align(&e, &[15, 15]).unwrap();
        assert_eq!(spans[0].first, 1);
        assert_eq!(spans[1].first, 3);
        assert!(force_align(&emissions(&[15]), &[15, 15]).is_none());
    }

    #[test]
    fn text_is_still_placed_when_the_audio_disagrees() {
        // The model hears only blanks; the alignment still returns ordered
        // spans, with low scores the caller can reject.
        let e = emissions(&[0; 10]);
        let spans = force_align(&e, &[4, 20, 23]).unwrap();
        assert!(spans.windows(2).all(|w| w[0].last < w[1].first));
        assert!(spans.iter().all(|s| s.score < -3.0));
    }

    #[test]
    fn too_little_audio_is_refused() {
        assert!(force_align(&emissions(&[4, 20]), &[4, 20, 23]).is_none());
        assert_eq!(force_align(&emissions(&[0]), &[]), Some(Vec::new()));
    }
}
