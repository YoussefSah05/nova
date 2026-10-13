//! Speech activity detection with Silero VAD v5 on ONNX Runtime.
//!
//! Whisper writes text for whatever it hears, including music and silence
//! ("Thanks for watching!", "Sous-titrage ST' 501"), and its segment times
//! often start in the silence before a line. The VAD gives the caption
//! pipeline an independent answer to "is anyone speaking here?", used to drop
//! captions that cover no speech and to keep cues off silent stretches.
//!
//! The model is a small recurrent network that scores 32 ms frames; frames
//! are fed in order, with the model's recurrent state and the last 64
//! samples of context carried from one frame to the next.

use std::path::Path;

use ndarray::{arr0, Array2, Array3};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Value;

use crate::AlignError;

/// Samples per scored frame at 16 kHz.
pub const FRAME: usize = 512;
const CONTEXT: usize = 64;
const FRAME_SEC: f64 = FRAME as f64 / 16_000.0;

/// Thresholds of Silero's reference `get_speech_timestamps`.
#[derive(Debug, Clone, Copy)]
pub struct VadParams {
    /// A frame at or above this probability starts or continues speech.
    pub threshold: f32,
    /// Speech ends only once probability drops below this...
    pub neg_threshold: f32,
    /// ...for at least this long.
    pub min_silence_sec: f64,
    /// Shorter bursts are not speech.
    pub min_speech_sec: f64,
    /// Margin added on both sides of every region.
    pub pad_sec: f64,
}

impl Default for VadParams {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            neg_threshold: 0.35,
            min_silence_sec: 0.1,
            min_speech_sec: 0.25,
            pad_sec: 0.03,
        }
    }
}

pub struct Vad {
    session: Session,
}

impl Vad {
    pub fn load(path: &Path) -> Result<Self, AlignError> {
        let load = |e: ort::Error| AlignError::Load(format!("{}: {e}", path.display()));
        let session = Session::builder()
            .map_err(load)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(load)?
            .with_intra_threads(1)
            .map_err(load)?
            .commit_from_file(path)
            .map_err(load)?;
        Ok(Self { session })
    }

    /// Speech probability of each 32 ms frame of 16 kHz mono `pcm`.
    pub fn probabilities(&self, pcm: &[f32]) -> Result<Vec<f32>, AlignError> {
        let infer = |e: ort::Error| AlignError::Inference(e.to_string());
        let mut state = Array3::<f32>::zeros((2, 1, 128));
        let mut context = vec![0f32; CONTEXT];
        let mut probs = Vec::with_capacity(pcm.len() / FRAME + 1);
        for chunk in pcm.chunks(FRAME) {
            let mut input = Vec::with_capacity(CONTEXT + FRAME);
            input.extend_from_slice(&context);
            input.extend_from_slice(chunk);
            input.resize(CONTEXT + FRAME, 0.0);
            context.copy_from_slice(&input[FRAME..]);
            let input = Array2::from_shape_vec((1, CONTEXT + FRAME), input)
                .map_err(|e| AlignError::Inference(e.to_string()))?;
            let outputs = self
                .session
                .run(vec![
                    ("input", Value::from_array(input).map_err(infer)?.into_dyn()),
                    (
                        "state",
                        Value::from_array(state.clone()).map_err(infer)?.into_dyn(),
                    ),
                    (
                        "sr",
                        Value::from_array(arr0(16_000i64))
                            .map_err(infer)?
                            .into_dyn(),
                    ),
                ])
                .map_err(infer)?;
            let p = outputs["output"]
                .try_extract_tensor::<f32>()
                .map_err(infer)?;
            probs.push(p.iter().next().copied().unwrap_or(0.0));
            let next = outputs["stateN"]
                .try_extract_tensor::<f32>()
                .map_err(infer)?;
            state = next
                .to_owned()
                .into_shape_with_order((2, 1, 128))
                .map_err(|e| AlignError::Inference(e.to_string()))?;
        }
        Ok(probs)
    }
}

/// Speech regions `[start, end)` in seconds from frame probabilities,
/// following Silero's reference hysteresis.
pub fn speech_regions(probs: &[f32], params: &VadParams) -> Vec<(f64, f64)> {
    let min_silence = (params.min_silence_sec / FRAME_SEC).ceil() as usize;
    let mut raw: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    let mut silence_from: Option<usize> = None;
    for (i, &p) in probs.iter().enumerate() {
        if p >= params.threshold {
            silence_from = None;
            start.get_or_insert(i);
        } else if p < params.neg_threshold {
            if let Some(s) = start {
                let from = *silence_from.get_or_insert(i);
                if i + 1 - from >= min_silence {
                    raw.push((s, from));
                    start = None;
                    silence_from = None;
                }
            }
        }
    }
    if let Some(s) = start {
        raw.push((s, silence_from.unwrap_or(probs.len())));
    }
    let mut regions: Vec<(f64, f64)> = Vec::new();
    let total = probs.len() as f64 * FRAME_SEC;
    for (s, e) in raw {
        let (s, e) = (s as f64 * FRAME_SEC, e as f64 * FRAME_SEC);
        if e - s < params.min_speech_sec {
            continue;
        }
        let (s, e) = (
            (s - params.pad_sec).max(0.0),
            (e + params.pad_sec).min(total),
        );
        match regions.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => regions.push((s, e)),
        }
    }
    regions
}

/// Seconds of `[start, end)` covered by `regions` (sorted, disjoint).
pub fn speech_within(regions: &[(f64, f64)], start: f64, end: f64) -> f64 {
    regions
        .iter()
        .map(|&(s, e)| (e.min(end) - s.max(start)).max(0.0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(pattern: &[(f32, usize)]) -> Vec<f32> {
        pattern
            .iter()
            .flat_map(|&(p, n)| std::iter::repeat_n(p, n))
            .collect()
    }

    #[test]
    fn regions_follow_hysteresis_and_minimums() {
        let params = VadParams {
            pad_sec: 0.0,
            ..Default::default()
        };
        // 10 frames silence, 20 speech, 2 dips (shorter than min silence),
        // 10 speech, 20 silence, 3 frames burst (too short), silence.
        let probs = frames(&[
            (0.1, 10),
            (0.9, 20),
            (0.2, 2),
            (0.9, 10),
            (0.1, 20),
            (0.9, 3),
            (0.1, 10),
        ]);
        let regions = speech_regions(&probs, &params);
        assert_eq!(regions.len(), 1, "{regions:?}");
        let (s, e) = regions[0];
        assert!((s - 10.0 * FRAME_SEC).abs() < 1e-9);
        assert!((e - 42.0 * FRAME_SEC).abs() < 1e-9);
    }

    #[test]
    fn values_between_thresholds_continue_speech() {
        let probs = frames(&[(0.9, 10), (0.4, 30), (0.9, 10), (0.0, 10)]);
        let regions = speech_regions(&probs, &VadParams::default());
        assert_eq!(regions.len(), 1);
    }

    #[test]
    fn speech_within_sums_overlap() {
        let regions = [(1.0, 2.0), (3.0, 5.0)];
        assert!((speech_within(&regions, 1.5, 4.0) - 1.5).abs() < 1e-9);
        assert_eq!(speech_within(&regions, 2.0, 3.0), 0.0);
    }
}
