//! Caption timing for Nova: speech activity detection and CTC forced
//! alignment, combined with Whisper's transcript.
//!
//! ```text
//!             ┌──────────── Whisper (text + rough times) ─────────────┐
//! 16 kHz PCM ─┼──────────── Silero VAD (is anyone speaking?) ─────────┼─▶ refine ─▶ timed segments
//!             └──────────── MMS CTC aligner (when is each letter?) ───┘
//! ```
//!
//! This is the production recipe popularised by WhisperX: Whisper is the best
//! open model at deciding *what* was said, but its timestamps come from the
//! decoder and drift; a wav2vec2 model trained with CTC on characters is
//! tied to the audio frames and decides *when*. The VAD drops captions
//! Whisper invented over music or silence.
//!
//! Every stage is optional. Without the aligner, Whisper's (DTW) word times
//! are kept; without the VAD, no segment is dropped; a word the aligner is
//! unsure of keeps a time interpolated from its confident neighbours.

pub mod ctc;
pub mod romanize;
pub mod vad;

use std::path::Path;

use nova_core::{TranscriptSegment, TranscriptWord};
use thiserror::Error;

pub use ctc::{CtcModel, Emissions};
pub use vad::{Vad, VadParams};

#[derive(Debug, Error)]
pub enum AlignError {
    #[error("could not load the alignment model: {0}")]
    Load(String),
    #[error("alignment inference failed: {0}")]
    Inference(String),
    #[error("audio is too short to align")]
    TooShort,
    #[error("audio is too long to align in one pass")]
    TooLong,
}

/// The pinned model files, relative to each model's directory.
pub const CTC_MODEL_FILE: &str = "onnx/model_uint8.onnx";
pub const VAD_MODEL_FILE: &str = "onnx/model.onnx";

/// Aligner threads: alignment runs beside playback, Whisper and translation.
const CTC_THREADS: usize = 4;
/// A segment with less detected speech than this is a hallucination...
const MIN_SEGMENT_SPEECH_SEC: f64 = 0.15;
/// ...counting speech this close to either end of it.
const SPEECH_SLACK_SEC: f64 = 0.3;
/// Reading time: a cue stays up at least this long (subtitle guidelines ask
/// for about 5/6 s)...
const MIN_CUE_SEC: f64 = 1.0;
/// ...and lingers this long after its last word...
const CUE_LINGER_SEC: f64 = 0.4;
/// ...but always clears this long before the next cue.
const CUE_GAP_SEC: f64 = 0.08;
/// Mean per-letter log-probability below which an aligned word is not
/// trusted (e^-4 ≈ 1.8 %; well-heard words score above -1).
const MIN_WORD_SCORE: f32 = -4.0;

/// The CTC aligner as a loaded model.
pub struct Aligner {
    ctc: CtcModel,
}

impl Aligner {
    pub fn load(dir: &Path) -> Result<Self, AlignError> {
        Ok(Self {
            ctc: CtcModel::load(&dir.join(CTC_MODEL_FILE), CTC_THREADS)?,
        })
    }

    pub fn emissions(&self, pcm: &[f32]) -> Result<Emissions, AlignError> {
        self.ctc.emissions(pcm)
    }
}

/// The VAD as a loaded model.
pub struct SpeechDetector {
    vad: Vad,
}

impl SpeechDetector {
    pub fn load(dir: &Path) -> Result<Self, AlignError> {
        Ok(Self {
            vad: Vad::load(&dir.join(VAD_MODEL_FILE))?,
        })
    }

    /// Speech regions of `pcm`, in seconds from its first sample.
    pub fn regions(&self, pcm: &[f32]) -> Result<Vec<(f64, f64)>, AlignError> {
        Ok(vad::speech_regions(
            &self.vad.probabilities(pcm)?,
            &VadParams::default(),
        ))
    }
}

/// What a refine pass did, for logs and the evaluation harness.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RefineReport {
    pub words: usize,
    /// Words whose time now comes from the aligner.
    pub aligned: usize,
    /// Words placed between aligned neighbours.
    pub interpolated: usize,
    /// Segments dropped for covering no speech.
    pub dropped_segments: usize,
}

/// Retime `segments` (absolute media seconds) using the window's audio
/// analysis. `offset_sec` is the media time of the window's first sample;
/// `speech` and `emissions` are relative to that sample.
pub fn refine(
    segments: &mut Vec<TranscriptSegment>,
    offset_sec: f64,
    speech: Option<&[(f64, f64)]>,
    emissions: Option<&Emissions>,
) -> RefineReport {
    let mut report = RefineReport::default();
    for segment in segments.iter_mut() {
        ensure_words(segment);
        report.words += segment.words.as_ref().map_or(0, Vec::len);
    }
    // Align first, the whole window at once: Whisper's segment bounds can
    // sit a second off the speech they transcribe, and one pass over all
    // the words lets word order, not those bounds, place each one.
    let mut confident: Vec<Vec<bool>> = segments
        .iter()
        .map(|s| vec![false; s.words.as_ref().map_or(0, Vec::len)])
        .collect();
    if let Some(emissions) = emissions {
        let mut words: Vec<TranscriptWord> = Vec::with_capacity(report.words);
        for segment in segments.iter() {
            words.extend(segment.words.iter().flatten().cloned());
        }
        let (placed, interpolated) = align_words(&mut words, offset_sec, emissions);
        report.aligned = placed.iter().filter(|p| **p).count();
        report.interpolated = interpolated;
        // Nothing is spoken outside the audio that was analysed.
        let audio_end = offset_sec + emissions.frames() as f64 * emissions.frame_sec;
        let mut words = words.into_iter();
        let mut placed = placed.into_iter();
        for (segment, confident) in segments.iter_mut().zip(confident.iter_mut()) {
            let seg_words = segment.words.as_mut().expect("ensured");
            for (w, c) in seg_words.iter_mut().zip(confident.iter_mut()) {
                *w = words.next().expect("same words");
                w.start_sec = w.start_sec.clamp(offset_sec, audio_end);
                w.end_sec = w.end_sec.clamp(w.start_sec, audio_end);
                *c = placed.next().expect("same words");
            }
        }
    } else if let Some(speech) = speech {
        for segment in segments.iter_mut() {
            snap_to_speech(segment.words.as_mut().expect("ensured"), offset_sec, speech);
        }
    }
    for segment in segments.iter_mut() {
        let words = segment.words.as_deref().unwrap_or_default();
        if let (Some(first), Some(last)) = (words.first(), words.last()) {
            segment.start_sec = first.start_sec;
            segment.end_sec = last.end_sec.max(first.start_sec);
        }
    }
    // A segment is invented when no speech surrounds it once aligned. (The
    // aligner always places every word somewhere, so its having placed a
    // word proves nothing; the VAD is the independent witness.)
    if let Some(speech) = speech {
        let before = segments.len();
        let aligned = emissions.is_some();
        let mut confident = confident.into_iter();
        segments.retain(|s| {
            let confident = confident.next().unwrap_or_default();
            let at = |t: f64| t - offset_sec;
            // A confidently aligned word that lands on speech proves the line.
            let heard = s.words.iter().flatten().zip(&confident).any(|(w, &c)| {
                c && vad::speech_within(speech, at(w.start_sec) - 0.1, at(w.end_sec) + 0.1) > 0.0
            });
            // Otherwise the segment needs speech of its own. Aligned spans are
            // trusted as they are; Whisper's own bounds get some slack.
            let slack = if aligned { 0.0 } else { SPEECH_SLACK_SEC };
            heard
                || vad::speech_within(speech, at(s.start_sec) - slack, at(s.end_sec) + slack)
                    >= MIN_SEGMENT_SPEECH_SEC
        });
        report.dropped_segments = before - segments.len();
    }
    // Cues never overlap: a later segment wins the shared instant.
    for i in 1..segments.len() {
        let next_start = segments[i].start_sec;
        let prev = &mut segments[i - 1];
        if prev.end_sec > next_start {
            prev.end_sec = next_start.max(prev.start_sec);
        }
    }
    pad_for_reading(segments);
    report
}

/// Give each cue time to be read. Word times stay exact; only a segment's
/// display end grows, to at least [`MIN_CUE_SEC`] after its start and
/// [`CUE_LINGER_SEC`] after its last word, never into the next cue.
fn pad_for_reading(segments: &mut [TranscriptSegment]) {
    for i in 0..segments.len() {
        let limit = segments
            .get(i + 1)
            .map_or(f64::INFINITY, |next| next.start_sec - CUE_GAP_SEC);
        let s = &mut segments[i];
        let wanted = (s.end_sec + CUE_LINGER_SEC).max(s.start_sec + MIN_CUE_SEC);
        s.end_sec = s.end_sec.max(wanted.min(limit));
    }
}

/// Segments without word timing get words spread over the segment by length,
/// so every segment can be aligned.
fn ensure_words(segment: &mut TranscriptSegment) {
    if segment.words.as_ref().is_some_and(|w| !w.is_empty()) {
        return;
    }
    let parts: Vec<&str> = segment.text.split_whitespace().collect();
    let total: usize = parts.iter().map(|w| w.chars().count().max(1)).sum();
    let span = (segment.end_sec - segment.start_sec).max(0.0);
    let mut at = segment.start_sec;
    let words = parts
        .iter()
        .map(|w| {
            let d = span * w.chars().count().max(1) as f64 / total.max(1) as f64;
            let word = TranscriptWord {
                start_sec: at,
                end_sec: at + d,
                word: w.to_string(),
            };
            at += d;
            word
        })
        .collect();
    segment.words = Some(words);
}

/// Align `words` (in order) to the whole of `emissions`. Returns which words
/// the aligner placed confidently, and how many others were interpolated.
fn align_words(
    words: &mut [TranscriptWord],
    offset_sec: f64,
    emissions: &Emissions,
) -> (Vec<bool>, usize) {
    let mut tokens = Vec::new();
    let mut ranges = Vec::with_capacity(words.len());
    for w in words.iter() {
        let t = romanize::word_tokens(&w.word);
        ranges.push(tokens.len()..tokens.len() + t.len());
        tokens.extend(t);
    }
    let none = vec![false; words.len()];
    let Some(spans) = ctc::force_align(emissions, &tokens) else {
        return (none, 0);
    };
    let to_sec = |f: usize| offset_sec + f as f64 * emissions.frame_sec;
    let mut placed: Vec<Option<(f64, f64)>> = ranges
        .iter()
        .map(|r| {
            if r.is_empty() {
                return None;
            }
            let span = &spans[r.clone()];
            let score = span.iter().map(|s| s.score).sum::<f32>() / span.len() as f32;
            (score >= MIN_WORD_SCORE)
                .then(|| (to_sec(span[0].first), to_sec(span[span.len() - 1].last + 1)))
        })
        .collect();
    let confident: Vec<bool> = placed.iter().map(Option::is_some).collect();
    if !confident.iter().any(|&c| c) {
        return (none, 0);
    }
    let interpolated = interpolate(words, &mut placed);
    for (w, p) in words.iter_mut().zip(placed) {
        if let Some((s, e)) = p {
            w.start_sec = s;
            w.end_sec = e.max(s);
        }
    }
    (confident, interpolated)
}

/// Fill unplaced words between placed neighbours, sharing the gap by word
/// length. Words before the first or after the last placed word keep their
/// duration and abut it. Returns how many were filled.
fn interpolate(words: &[TranscriptWord], placed: &mut [Option<(f64, f64)>]) -> usize {
    let mut filled = 0;
    let mut i = 0;
    while i < placed.len() {
        if placed[i].is_some() {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < placed.len() && placed[i].is_none() {
            i += 1;
        }
        let run = run_start..i;
        let before = run_start
            .checked_sub(1)
            .and_then(|k| placed[k])
            .map(|p| p.1);
        let after = placed.get(i).copied().flatten().map(|p| p.0);
        let lens: Vec<f64> = words[run.clone()]
            .iter()
            .map(|w| w.word.chars().count().max(1) as f64)
            .collect();
        let total: f64 = lens.iter().sum();
        match (before, after) {
            (Some(a), Some(b)) => {
                let gap = (b - a).max(0.0);
                let mut at = a;
                for (k, len) in run.clone().zip(&lens) {
                    let d = gap * len / total;
                    placed[k] = Some((at, at + d));
                    at += d;
                }
            }
            (Some(a), None) => {
                let mut at = a;
                for k in run.clone() {
                    let d = (words[k].end_sec - words[k].start_sec).max(0.0);
                    placed[k] = Some((at, at + d));
                    at += d;
                }
            }
            (None, Some(b)) => {
                let mut at = b;
                for k in run.clone().rev() {
                    let d = (words[k].end_sec - words[k].start_sec).max(0.0);
                    placed[k] = Some((at - d, at));
                    at -= d;
                }
            }
            (None, None) => continue,
        }
        filled += run.len();
    }
    filled
}

/// Without an aligner, keep the first and last word off silence: a start in
/// a non-speech stretch moves to the next speech onset, an end to the last
/// speech offset, when that onset/offset falls inside the word.
fn snap_to_speech(words: &mut [TranscriptWord], offset_sec: f64, speech: &[(f64, f64)]) {
    if let Some(first) = words.first_mut() {
        let start = first.start_sec - offset_sec;
        if vad::speech_within(speech, start, start + 0.01) == 0.0 {
            if let Some(&(onset, _)) = speech.iter().find(|r| r.0 > start) {
                if onset + offset_sec < first.end_sec {
                    first.start_sec = onset + offset_sec;
                }
            }
        }
    }
    if let Some(last) = words.last_mut() {
        let end = last.end_sec - offset_sec;
        if vad::speech_within(speech, end - 0.01, end) == 0.0 {
            if let Some(&(_, offset)) = speech.iter().rev().find(|r| r.1 < end) {
                if offset + offset_sec > last.start_sec {
                    last.end_sec = offset + offset_sec;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    fn word(s: f64, e: f64, w: &str) -> TranscriptWord {
        TranscriptWord {
            start_sec: s,
            end_sec: e,
            word: w.into(),
        }
    }

    fn segment(s: f64, e: f64, words: Vec<TranscriptWord>) -> TranscriptSegment {
        TranscriptSegment {
            seq: 0,
            text: words
                .iter()
                .map(|w| w.word.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            start_sec: s,
            end_sec: e,
            words: Some(words),
        }
    }

    /// 20 ms frames; `marks` lists (frame, label) peaks over blank.
    fn emissions(frames: usize, marks: &[(usize, char)]) -> Emissions {
        let mut lp = Array2::from_elem((frames, romanize::VOCAB_SIZE), (0.001f32).ln());
        for t in 0..frames {
            lp[[t, 0]] = (0.95f32).ln();
        }
        for &(t, c) in marks {
            let id = romanize::word_tokens(&c.to_string())[0] as usize;
            lp[[t, 0]] = (0.001f32).ln();
            lp[[t, id]] = (0.95f32).ln();
        }
        Emissions {
            log_probs: lp,
            frame_sec: 0.02,
        }
    }

    #[test]
    fn the_aligner_moves_words_onto_the_audio() {
        // Whisper says "ab cd" at 0.8–1.2 s; the audio has a/b at frames
        // 50/51 (1.00 s) and c/d at 70/72 (1.40 s). Window starts at 10 s.
        let e = emissions(100, &[(50, 'a'), (51, 'b'), (70, 'c'), (72, 'd')]);
        let mut segs = vec![segment(
            10.8,
            11.2,
            vec![word(10.8, 11.0, "ab"), word(11.0, 11.2, "cd")],
        )];
        let report = refine(&mut segs, 10.0, None, Some(&e));
        assert_eq!(report.aligned, 2);
        let w = segs[0].words.as_ref().unwrap();
        assert!((w[0].start_sec - 11.0).abs() < 1e-6, "{w:?}");
        assert!((w[0].end_sec - 11.04).abs() < 1e-6);
        assert!((w[1].start_sec - 11.4).abs() < 1e-6);
        assert!((segs[0].start_sec - 11.0).abs() < 1e-6);
        assert!((w[1].end_sec - 11.46).abs() < 1e-6);
        assert!(
            (segs[0].end_sec - 12.0).abs() < 1e-6,
            "shown for at least a second"
        );
    }

    #[test]
    fn words_without_letters_are_interpolated_between_neighbours() {
        let e = emissions(100, &[(20, 'a'), (60, 'b')]);
        let mut segs = vec![segment(
            0.3,
            1.3,
            vec![
                word(0.3, 0.5, "a"),
                word(0.5, 0.9, "42"),
                word(0.9, 1.3, "b"),
            ],
        )];
        let report = refine(&mut segs, 0.0, None, Some(&e));
        assert_eq!((report.aligned, report.interpolated), (2, 1));
        let w = segs[0].words.as_ref().unwrap();
        assert!((w[1].start_sec - w[0].end_sec).abs() < 1e-9);
        assert!((w[1].end_sec - w[2].start_sec).abs() < 1e-9);
    }

    #[test]
    fn a_misplaced_real_word_is_kept_once_the_aligner_finds_it() {
        // Whisper put "ab" in the pause at 3–4 s; the audio has it at 1 s,
        // where the VAD heard speech.
        let e = emissions(250, &[(50, 'a'), (51, 'b')]);
        let speech = [(0.9, 1.2)];
        let mut segs = vec![segment(3.0, 4.0, vec![word(3.0, 4.0, "ab")])];
        let report = refine(&mut segs, 0.0, Some(&speech), Some(&e));
        assert_eq!(report.dropped_segments, 0);
        assert!((segs[0].start_sec - 1.0).abs() < 1e-6, "{segs:?}");
    }

    #[test]
    fn an_unaligned_tail_after_the_last_line_is_dropped() {
        // Whisper's classic sign-off, past the end of the dialogue: the
        // aligner finds no letters for it and the VAD no speech under it.
        let e = emissions(100, &[(20, 'a'), (21, 'b')]);
        let speech = [(0.35, 0.5)];
        let mut segs = vec![
            segment(0.4, 0.5, vec![word(0.4, 0.5, "ab")]),
            segment(
                1.0,
                30.0,
                vec![word(1.0, 15.0, "xyz"), word(15.0, 30.0, "qq")],
            ),
        ];
        let report = refine(&mut segs, 0.0, Some(&speech), Some(&e));
        assert_eq!(report.dropped_segments, 1, "{segs:?}");
        assert_eq!(segs[0].text, "ab");
    }

    #[test]
    fn short_lines_stay_up_long_enough_to_read() {
        let mut segs = vec![
            segment(25.13, 25.31, vec![word(25.13, 25.31, "Emo?")]),
            segment(26.43, 28.64, vec![word(26.43, 28.64, "Emo?")]),
            segment(28.70, 30.92, vec![word(28.70, 30.92, "Watch out!")]),
        ];
        refine(&mut segs, 0.0, None, None);
        assert!((segs[0].end_sec - 26.13).abs() < 1e-9, "one second minimum");
        assert!(
            (segs[1].end_sec - 28.64).abs() < 1e-9,
            "never pushed into the next cue"
        );
        assert!(
            (segs[2].end_sec - 31.32).abs() < 1e-9,
            "lingers after the last word"
        );
        assert!((segs[0].words.as_ref().unwrap()[0].end_sec - 25.31).abs() < 1e-9);
    }

    #[test]
    fn captions_over_silence_are_dropped() {
        let speech = [(0.0, 2.0)];
        let mut segs = vec![
            segment(0.5, 1.5, vec![word(0.5, 1.5, "hello")]),
            segment(5.0, 7.0, vec![word(5.0, 7.0, "thanks for watching")]),
        ];
        let report = refine(&mut segs, 0.0, Some(&speech), None);
        assert_eq!(report.dropped_segments, 1);
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn without_an_aligner_cue_edges_snap_to_speech() {
        let speech = [(1.2, 2.5)];
        let mut segs = vec![segment(
            0.8,
            3.0,
            vec![word(0.8, 1.6, "early"), word(1.6, 3.0, "late")],
        )];
        refine(&mut segs, 0.0, Some(&speech), None);
        let w = segs[0].words.as_ref().unwrap();
        assert!((segs[0].start_sec - 1.2).abs() < 1e-9);
        assert!((w[1].end_sec - 2.5).abs() < 1e-9);
    }

    #[test]
    fn segments_without_words_are_split_by_length_then_never_overlap() {
        let mut segs = vec![
            TranscriptSegment {
                seq: 0,
                start_sec: 0.0,
                end_sec: 2.0,
                text: "one three".into(),
                words: None,
            },
            segment(1.5, 3.0, vec![word(1.5, 3.0, "next")]),
        ];
        refine(&mut segs, 0.0, None, None);
        let w = segs[0].words.as_ref().unwrap();
        assert_eq!(w.len(), 2);
        assert!(segs[0].end_sec <= segs[1].start_sec);
    }
}
