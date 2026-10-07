//! Speech recognition benchmark: engines, the streaming policy and the
//! virtual-clock runner.
//!
//! Streaming is simulated on a virtual clock rather than waited out in real
//! time: a step can start only once its audio would have arrived and the
//! engine is free, and its measured compute time is added. Latencies are
//! therefore what a live viewer would see, at a fraction of the wall time.

pub mod apple;
pub mod local_agreement;
pub mod run;
pub mod whisper;

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::metrics::{cer, wer, ErrorRate};
use crate::text::normalize;
use local_agreement::{LocalAgreement, Word};

pub const SAMPLE_RATE: usize = 16_000;

pub struct Decode {
    pub words: Vec<Word>,
    pub language: Option<String>,
}

/// An engine that decodes a whole audio buffer (whisper-style). Streaming
/// comes from the policy re-decoding a sliding buffer.
pub trait BufferDecoder {
    fn decode(
        &mut self,
        pcm: &[f32],
        offset_sec: f64,
        prompt: &str,
        language: Option<&str>,
    ) -> Result<Decode>;
}

/// A benchmark candidate: offline decoding of one clip, and live streaming of
/// a programme. Whisper-style engines get both from [`run_offline`] and
/// [`run_stream`]; natively streaming engines (Apple) implement them directly.
pub trait AsrCandidate {
    fn offline(&mut self, pcm: &[f32], language: Option<&str>) -> Result<OfflineResult>;
    fn stream(&mut self, pcm: &[f32], cfg: &StreamConfig) -> Result<StreamResult>;
    /// Whether `offline(.., None)` reports a detected language.
    fn detects_language(&self) -> bool {
        true
    }
}

impl<T: BufferDecoder> AsrCandidate for T {
    fn offline(&mut self, pcm: &[f32], language: Option<&str>) -> Result<OfflineResult> {
        run_offline(self, pcm, language)
    }
    fn stream(&mut self, pcm: &[f32], cfg: &StreamConfig) -> Result<StreamResult> {
        run_stream(self, pcm, cfg)
    }
}

/// Decode any audio file to 16 kHz mono f32 with ffmpeg.
pub fn load_audio(path: &Path) -> Result<Vec<f32>> {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-ac", "1", "-ar", "16000", "-f", "f32le", "pipe:1"])
        .output()
        .context("running ffmpeg")?;
    if !out.status.success() {
        bail!("ffmpeg could not decode {}", path.display());
    }
    Ok(out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

#[derive(Debug, Clone)]
pub struct StreamConfig {
    /// New audio between decodes.
    pub step_sec: f64,
    /// Longest buffer before it is trimmed at the committed point.
    pub max_buffer_sec: f64,
    pub prompt_words: usize,
    pub language: Option<String>,
}

/// A committed word with when it was first shown and when it was committed
/// (virtual or wall seconds since stream start). Its own start/end are the
/// engine's timestamps, used only to assign it to an utterance.
#[derive(Debug, Clone, Serialize)]
pub struct TimedWord {
    pub start: f64,
    pub end: f64,
    pub shown_at: f64,
    pub committed_at: f64,
}

#[derive(Debug, Default, Serialize)]
pub struct StreamResult {
    pub audio_sec: f64,
    pub compute_sec: f64,
    pub steps: usize,
    /// Virtual time when any text first appeared.
    pub first_text_at: Option<f64>,
    /// Per word: first shown (provisional or committed) minus its spoken end.
    pub shown_lag: Vec<f64>,
    /// Per word: committed minus its spoken end.
    pub committed_lag: Vec<f64>,
    /// Provisional words later replaced by different text.
    pub retracted_words: usize,
    pub committed_words: usize,
    /// Committed by the end-of-stream flush, excluded from the lag figures.
    pub flushed_words: usize,
    /// Committed words (flush excluded), for reference-timed latency.
    #[serde(skip)]
    pub timeline: Vec<TimedWord>,
    /// Whether a real-time engine kept pace with the audio (None: simulated).
    pub keeps_up: Option<bool>,
    #[serde(skip)]
    pub text: String,
}

struct Seen {
    key: String,
    start: f64,
    at: f64,
}

fn first_seen(seen: &mut Vec<Seen>, word: &Word, now: f64) -> f64 {
    let key = normalize(&word.text);
    if let Some(s) = seen
        .iter()
        .find(|s| s.key == key && (s.start - word.start).abs() < 0.6)
    {
        return s.at;
    }
    seen.push(Seen {
        key,
        start: word.start,
        at: now,
    });
    now
}

/// Stream `pcm` through `decoder` with LocalAgreement-2 on a virtual clock.
pub fn run_stream(
    decoder: &mut dyn BufferDecoder,
    pcm: &[f32],
    cfg: &StreamConfig,
) -> Result<StreamResult> {
    let total = pcm.len() as f64 / SAMPLE_RATE as f64;
    let mut la = LocalAgreement::default();
    let mut result = StreamResult {
        audio_sec: total,
        ..Default::default()
    };
    let (mut clock, mut buffer_start) = (0.0f64, 0.0f64);
    let mut next_audio = cfg.step_sec.min(total);
    let mut shown: Vec<Word> = Vec::new();
    let mut seen: Vec<Seen> = Vec::new();
    loop {
        // Start when this step's audio has arrived and the engine is free; a
        // late start decodes everything that arrived meanwhile.
        let start = clock.max(next_audio);
        let audio_end = start.min(total);
        let from = (buffer_start * SAMPLE_RATE as f64) as usize;
        let to = (audio_end * SAMPLE_RATE as f64) as usize;
        let began = Instant::now();
        let decode = decoder.decode(
            &pcm[from..to],
            buffer_start,
            &la.prompt(buffer_start, cfg.prompt_words),
            cfg.language.as_deref(),
        )?;
        let spent = began.elapsed().as_secs_f64();
        result.compute_sec += spent;
        result.steps += 1;
        clock = start + spent;

        let mut hypothesis = decode.words;
        local_agreement::truncate_repetition(&mut hypothesis);
        let out = la.step(hypothesis, buffer_start);
        let now_shown: Vec<Word> = out
            .committed
            .iter()
            .chain(&out.provisional)
            .cloned()
            .collect();
        let kept = shown
            .iter()
            .zip(&now_shown)
            .take_while(|(a, b)| normalize(&a.text) == normalize(&b.text))
            .count();
        result.retracted_words += shown.len().saturating_sub(kept);
        for w in &now_shown {
            first_seen(&mut seen, w, clock);
        }
        if result.first_text_at.is_none() && !now_shown.is_empty() {
            result.first_text_at = Some(clock);
        }
        for w in &out.committed {
            let first = first_seen(&mut seen, w, clock);
            result.shown_lag.push(first - w.end);
            result.committed_lag.push(clock - w.end);
            result.timeline.push(TimedWord {
                start: w.start,
                end: w.end,
                shown_at: first,
                committed_at: clock,
            });
        }
        shown = out.provisional;

        if audio_end >= total {
            break;
        }
        if audio_end - buffer_start > cfg.max_buffer_sec {
            // Prefer a committed sentence end; otherwise keep half a second
            // of committed audio, which the text alignment strips again.
            buffer_start = la
                .trim_point(buffer_start)
                .unwrap_or_else(|| (la.committed_until() - 0.5).max(buffer_start))
                .max(audio_end - 2.0 * cfg.max_buffer_sec);
        }
        next_audio = (audio_end + cfg.step_sec).min(total);
    }
    // Words committed only because the input ended are not steady-state
    // latency (a live channel never ends): counted, not timed.
    result.flushed_words = la.flush().len();
    result.committed_words = la.committed().len();
    result.text = la
        .committed()
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(result)
}

#[derive(Debug, Default, Serialize)]
pub struct OfflineResult {
    pub audio_sec: f64,
    pub compute_sec: f64,
    #[serde(skip)]
    pub text: String,
    pub language: Option<String>,
}

/// One decode of the whole clip: the quality ceiling and raw speed.
pub fn run_offline(
    decoder: &mut dyn BufferDecoder,
    pcm: &[f32],
    language: Option<&str>,
) -> Result<OfflineResult> {
    let began = Instant::now();
    let decode = decoder.decode(pcm, 0.0, "", language)?;
    Ok(OfflineResult {
        audio_sec: pcm.len() as f64 / SAMPLE_RATE as f64,
        compute_sec: began.elapsed().as_secs_f64(),
        text: decode
            .words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        language: decode.language,
    })
}

/// Quality of a hypothesis against its reference.
pub fn score(reference: &str, hypothesis: &str) -> (ErrorRate, ErrorRate) {
    (wer(reference, hypothesis), cer(reference, hypothesis))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake engine that "hears" a fixed script: each word becomes
    /// decodable once its audio is in the buffer, and costs 0.1 s per call.
    struct Scripted {
        script: Vec<(&'static str, f64, f64)>,
    }

    impl BufferDecoder for Scripted {
        fn decode(&mut self, pcm: &[f32], offset: f64, _: &str, _: Option<&str>) -> Result<Decode> {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let end = offset + pcm.len() as f64 / SAMPLE_RATE as f64;
            let words = self
                .script
                .iter()
                .filter(|(_, s, e)| *s >= offset - 0.01 && *e <= end)
                .map(|(t, s, e)| Word {
                    text: t.to_string(),
                    start: *s,
                    end: *e,
                })
                .collect();
            Ok(Decode {
                words,
                language: Some("en".into()),
            })
        }
    }

    #[test]
    fn streaming_commits_every_word_with_bounded_lag() {
        let script = vec![
            ("one", 0.2, 0.6),
            ("two", 1.2, 1.6),
            ("three", 2.2, 2.6),
            ("four", 3.2, 3.6),
        ];
        let pcm = vec![0.0f32; 4 * SAMPLE_RATE];
        let cfg = StreamConfig {
            step_sec: 1.0,
            max_buffer_sec: 15.0,
            prompt_words: 10,
            language: None,
        };
        let r = run_stream(&mut Scripted { script }, &pcm, &cfg).unwrap();
        assert_eq!(r.text, "one two three four");
        assert_eq!(r.committed_words, 4);
        assert_eq!(r.retracted_words, 0);
        // Shown about one step after spoken; committed one step later.
        assert!(
            r.shown_lag.iter().all(|l| *l > 0.0 && *l < 1.6),
            "{:?}",
            r.shown_lag
        );
        assert!(r
            .committed_lag
            .iter()
            .zip(&r.shown_lag)
            .all(|(c, s)| c >= s));
        assert!(r.first_text_at.unwrap() < 1.2);
    }
}
