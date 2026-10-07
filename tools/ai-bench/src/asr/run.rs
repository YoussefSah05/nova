//! `asr` subcommand: one candidate over one dataset, per language, offline
//! (quality ceiling) and streaming (what a live viewer sees).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{load_audio, score, AsrCandidate, StreamConfig, SAMPLE_RATE};
use crate::metrics::ErrorRate;

#[derive(Deserialize)]
struct Utterance {
    id: String,
    audio: String,
    text: String,
}

fn read_manifest(dir: &Path) -> Result<Vec<Utterance>> {
    let path = dir.join("manifest.jsonl");
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).context("manifest line"))
        .collect()
}

#[derive(Debug, Default, Serialize)]
pub struct Quality {
    pub wer: f64,
    pub cer: f64,
    pub words: usize,
    /// Bootstrap 95% interval of the WER (utterances resampled), when known.
    pub wer_ci95: Option<(f64, f64)>,
}

/// Bootstrap 95% confidence interval of a corpus error rate over items
/// (edits, reference words); deterministic seed.
pub fn bootstrap_ci(items: &[(usize, usize)]) -> Option<(f64, f64)> {
    if items.len() < 2 {
        return None;
    }
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut rates: Vec<f64> = (0..1000)
        .map(|_| {
            let (mut e, mut r) = (0usize, 0usize);
            for _ in 0..items.len() {
                let (ie, ir) = items[(next() % items.len() as u64) as usize];
                e += ie;
                r += ir;
            }
            e as f64 / r.max(1) as f64
        })
        .collect();
    rates.sort_by(|a, b| a.total_cmp(b));
    Some((rates[25], rates[974]))
}

/// Speech onset and offset (seconds) in a clip: the first and last runs of
/// at least 160 ms (8 frames of 20 ms) whose RMS reaches 10% of the clip's
/// loudest frame. A sustained run ignores clicks and breaths. Reference
/// timing that does not depend on any engine's timestamps.
pub fn speech_bounds(pcm: &[f32]) -> Option<(f64, f64)> {
    const FRAME: usize = SAMPLE_RATE / 50;
    const RUN: usize = 8;
    let rms: Vec<f32> = pcm
        .chunks(FRAME)
        .map(|f| (f.iter().map(|x| x * x).sum::<f32>() / f.len() as f32).sqrt())
        .collect();
    let peak = rms.iter().cloned().fold(0.0f32, f32::max);
    if peak <= 1e-4 {
        return None;
    }
    let loud: Vec<bool> = rms.iter().map(|r| *r >= peak * 0.1).collect();
    let sustained = |i: usize| i + RUN <= loud.len() && loud[i..i + RUN].iter().all(|l| *l);
    let first = (0..loud.len()).find(|&i| sustained(i))?;
    let last_run = (0..loud.len()).rev().find(|&i| sustained(i))?;
    Some((first as f64 * 0.02, (last_run + RUN) as f64 * 0.02))
}

impl Quality {
    fn from(w: ErrorRate, c: ErrorRate) -> Self {
        Self {
            wer: w.rate(),
            cer: c.rate(),
            words: w.reference,
            wer_ci95: None,
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct OfflineSummary {
    pub utterances: usize,
    pub quality: Quality,
    /// Compute seconds per audio second.
    pub rtf: f64,
    /// Utterances whose detected language differs from the expected one.
    pub wrong_language: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct StreamingSummary {
    pub streams: usize,
    pub audio_sec: f64,
    pub quality: Quality,
    pub rtf: f64,
    /// Real-time engines: whether every stream kept pace (None: simulated).
    pub keeps_up: Option<bool>,
    /// Reference-timed (engine-independent): first text after speech onset.
    pub first_text_latency_p50: f64,
    /// Per utterance: its last word shown / committed, after its speech end.
    pub utterance_shown_lag_p50: f64,
    pub utterance_shown_lag_p95: f64,
    pub utterance_committed_lag_p50: f64,
    pub utterance_committed_lag_p95: f64,
    /// Utterances with no committed word at all (flush excluded). They count
    /// as infinite lag in the percentiles: dropping speech never looks fast.
    pub missed_utterances: usize,
    pub utterances: usize,
    /// Utterance lags below zero: a sign of hallucination in silence or a
    /// misplaced speech boundary. Should be zero.
    pub negative_lags: usize,
    pub first_text_at_p50: f64,
    pub shown_lag_p50: f64,
    pub shown_lag_p95: f64,
    pub committed_lag_p50: f64,
    pub committed_lag_p95: f64,
    /// Retracted provisional words per committed word.
    pub rewrite_rate: f64,
    /// Reference and hypothesis per stream, for error analysis.
    pub samples: Vec<Sample>,
}

#[derive(Debug, Serialize)]
pub struct Sample {
    pub reference: String,
    pub hypothesis: String,
}

#[derive(Debug, Default, Serialize)]
pub struct LanguageReport {
    pub language: String,
    pub offline: OfflineSummary,
    pub streaming: StreamingSummary,
}

/// Percentile by rounded linear index into the sorted values.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let rank = ((p / 100.0) * (v.len() - 1) as f64).round() as usize;
    v[rank]
}

pub struct RunOptions {
    pub offline_limit: usize,
    pub streams: usize,
    pub stream_sec: f64,
    pub stream: StreamConfig,
}

/// Whisper language code for a dataset language directory (`en_us` → `en`).
fn whisper_language(dir_name: &str) -> String {
    dir_name
        .split(['_', '-'])
        .next()
        .unwrap_or(dir_name)
        .to_string()
}

pub fn run_language(
    candidate: &mut dyn AsrCandidate,
    lang_dir: &Path,
    opts: &RunOptions,
) -> Result<LanguageReport> {
    // FLEURS records each sentence several times (different speakers), in
    // consecutive rows. Keep the first recording of each sentence: repeated
    // sentences would let the streaming prompt "remember" the answer.
    let mut seen = std::collections::HashSet::new();
    let utterances: Vec<Utterance> = read_manifest(lang_dir)?
        .into_iter()
        .filter(|u| seen.insert(crate::text::normalize(&u.text)))
        .collect();
    let dir_name = lang_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string();
    let language = whisper_language(&dir_name);
    let audio = |u: &Utterance| -> Result<Vec<f32>> { load_audio(&lang_dir.join(&u.audio)) };

    // Offline: each utterance alone, language given (the app knows or detects
    // it once per programme), plus a detection check without the hint.
    let mut offline = OfflineSummary::default();
    let (mut w, mut c) = (ErrorRate::default(), ErrorRate::default());
    let mut per_utterance = Vec::new();
    let (mut audio_sec, mut compute_sec) = (0.0, 0.0);
    for u in utterances.iter().take(opts.offline_limit) {
        let pcm = audio(u)?;
        let r = candidate.offline(&pcm, Some(&language))?;
        let (wu, cu) = score(&u.text, &r.text);
        per_utterance.push((wu.edits, wu.reference));
        w.add(wu);
        c.add(cu);
        audio_sec += r.audio_sec;
        compute_sec += r.compute_sec;
        offline.utterances += 1;
        if candidate.detects_language() {
            let detected = candidate.offline(&pcm[..pcm.len().min(SAMPLE_RATE * 5)], None)?;
            if detected.language.as_deref() != Some(language.as_str()) {
                offline.wrong_language += 1;
            }
        }
        eprintln!("  [{dir_name}] offline {} wer {:.3}", u.id, wu.rate());
    }
    offline.quality = Quality::from(w, c);
    offline.quality.wer_ci95 = bootstrap_ci(&per_utterance);
    offline.rtf = compute_sec / audio_sec.max(1e-9);

    // Streaming: consecutive utterances joined into programmes with short
    // pauses, as continuous broadcast audio.
    let mut streaming = StreamingSummary::default();
    let (mut w, mut c) = (ErrorRate::default(), ErrorRate::default());
    let (mut shown, mut committed, mut first) = (Vec::new(), Vec::new(), Vec::new());
    let (mut retracted, mut committed_words, mut s_audio, mut s_compute) =
        (0usize, 0usize, 0.0, 0.0);
    let (mut first_latency, mut utt_shown, mut utt_committed) =
        (Vec::new(), Vec::new(), Vec::new());
    let (mut missed, mut utterance_count, mut negative) = (0usize, 0usize, 0usize);
    let mut all_kept_up: Option<bool> = None;
    let gap = vec![0.0f32; SAMPLE_RATE * 2 / 5];
    let mut queue = utterances.iter().skip(opts.offline_limit).peekable();
    for _ in 0..opts.streams {
        let (mut pcm, mut reference) = (Vec::new(), Vec::new());
        // Per utterance: (span start, span end, speech end) in stream seconds.
        let mut spans: Vec<(f64, f64, f64)> = Vec::new();
        let mut onset: Option<f64> = None;
        while (pcm.len() as f64) < opts.stream_sec * SAMPLE_RATE as f64 {
            let Some(u) = queue.next() else { break };
            let clip = audio(u)?;
            let at = pcm.len() as f64 / SAMPLE_RATE as f64;
            let len = clip.len() as f64 / SAMPLE_RATE as f64;
            let (on, off) = speech_bounds(&clip).unwrap_or((0.0, len));
            onset.get_or_insert(at + on);
            spans.push((
                at,
                at + len + gap.len() as f64 / SAMPLE_RATE as f64,
                at + off,
            ));
            pcm.extend(clip);
            pcm.extend(&gap);
            reference.push(u.text.as_str());
        }
        if pcm.is_empty() {
            break;
        }
        // Trailing silence so the last utterance commits on its own instead
        // of through the end-of-input flush (which is not timed).
        pcm.extend(std::iter::repeat_n(0.0f32, SAMPLE_RATE * 5));
        let mut cfg = opts.stream.clone();
        cfg.language = Some(language.clone());
        let r = candidate.stream(&pcm, &cfg)?;
        let (ws, cs) = score(&reference.join(" "), &r.text);
        streaming.samples.push(Sample {
            reference: reference.join(" "),
            hypothesis: r.text.clone(),
        });
        eprintln!(
            "  [{dir_name}] stream {:.0}s wer {:.3} first {:.2}s",
            r.audio_sec,
            ws.rate(),
            r.first_text_at.unwrap_or(f64::NAN)
        );
        w.add(ws);
        c.add(cs);
        shown.extend(r.shown_lag);
        committed.extend(r.committed_lag);
        if let Some(f) = r.first_text_at {
            first.push(f);
            if let Some(on) = onset {
                first_latency.push(f - on);
            }
        }
        // Reference-timed utterance lag: words are assigned to utterances by
        // their midpoint (the only use of engine timestamps here).
        for &(from, to, speech_end) in &spans {
            utterance_count += 1;
            let words: Vec<&super::TimedWord> = r
                .timeline
                .iter()
                .filter(|w| {
                    let mid = (w.start + w.end) / 2.0;
                    mid >= from && mid < to
                })
                .collect();
            if words.is_empty() {
                missed += 1;
                utt_shown.push(f64::INFINITY);
                utt_committed.push(f64::INFINITY);
                continue;
            }
            let shown = words.iter().map(|w| w.shown_at).fold(f64::MIN, f64::max);
            let committed = words
                .iter()
                .map(|w| w.committed_at)
                .fold(f64::MIN, f64::max);
            utt_shown.push(shown - speech_end);
            utt_committed.push(committed - speech_end);
            if committed < speech_end {
                negative += 1;
            }
        }
        if let Some(k) = r.keeps_up {
            all_kept_up = Some(all_kept_up.unwrap_or(true) && k);
        }
        retracted += r.retracted_words;
        committed_words += r.committed_words;
        s_audio += r.audio_sec;
        s_compute += r.compute_sec;
        streaming.streams += 1;
    }
    streaming.audio_sec = s_audio;
    streaming.quality = Quality::from(w, c);
    streaming.rtf = s_compute / s_audio.max(1e-9);
    streaming.first_text_at_p50 = percentile(&first, 50.0);
    streaming.shown_lag_p50 = percentile(&shown, 50.0);
    streaming.shown_lag_p95 = percentile(&shown, 95.0);
    streaming.committed_lag_p50 = percentile(&committed, 50.0);
    streaming.committed_lag_p95 = percentile(&committed, 95.0);
    streaming.rewrite_rate = retracted as f64 / committed_words.max(1) as f64;
    streaming.keeps_up = all_kept_up;
    streaming.first_text_latency_p50 = percentile(&first_latency, 50.0);
    streaming.utterance_shown_lag_p50 = percentile(&utt_shown, 50.0);
    streaming.utterance_shown_lag_p95 = percentile(&utt_shown, 95.0);
    streaming.utterance_committed_lag_p50 = percentile(&utt_committed, 50.0);
    streaming.utterance_committed_lag_p95 = percentile(&utt_committed, 95.0);
    streaming.missed_utterances = missed;
    streaming.negative_lags = negative;
    streaming.utterances = utterance_count;

    Ok(LanguageReport {
        language: dir_name,
        offline,
        streaming,
    })
}

#[derive(Debug, Serialize)]
pub struct CandidateReport {
    pub candidate: String,
    pub engine: String,
    pub coreml: bool,
    pub tuning: super::whisper::WhisperTuning,
    pub load_sec: f64,
    pub model_path: PathBuf,
    pub dataset: PathBuf,
    pub step_sec: f64,
    pub peak_rss_mb: f64,
    pub languages: Vec<LanguageReport>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn percentiles_use_nearest_rank() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(super::percentile(&v, 50.0), 3.0);
        assert_eq!(super::percentile(&v, 95.0), 5.0);
        assert!(super::percentile(&[], 50.0).is_nan());
    }

    #[test]
    fn dataset_directories_map_to_whisper_languages() {
        assert_eq!(super::whisper_language("en_us"), "en");
        assert_eq!(super::whisper_language("ar_eg"), "ar");
        assert_eq!(super::whisper_language("fr"), "fr");
    }

    #[test]
    fn speech_bounds_skip_leading_and_trailing_silence() {
        let mut pcm = vec![0.0f32; super::SAMPLE_RATE / 2];
        pcm.extend(std::iter::repeat_n(0.5f32, super::SAMPLE_RATE));
        pcm.extend(vec![0.0f32; super::SAMPLE_RATE / 2]);
        let (on, off) = super::speech_bounds(&pcm).unwrap();
        assert!(
            (on - 0.5).abs() < 0.03 && (off - 1.5).abs() < 0.03,
            "{on} {off}"
        );
        // A 40 ms click after the speech does not move the speech end.
        let mut clicked = pcm.clone();
        let at = super::SAMPLE_RATE * 18 / 10;
        clicked.splice(
            at..at + super::SAMPLE_RATE / 25,
            std::iter::repeat_n(0.9f32, super::SAMPLE_RATE / 25),
        );
        let (_, off) = super::speech_bounds(&clicked).unwrap();
        assert!((off - 1.5).abs() < 0.03, "click ignored: {off}");
        assert!(super::speech_bounds(&vec![0.0; 100]).is_none());
    }

    #[test]
    fn bootstrap_interval_brackets_the_corpus_rate() {
        let items: Vec<(usize, usize)> = (0..40).map(|i| (i % 3, 10)).collect();
        let (lo, hi) = super::bootstrap_ci(&items).unwrap();
        let rate = items.iter().map(|x| x.0).sum::<usize>() as f64 / 400.0;
        assert!(lo <= rate && rate <= hi && lo < hi);
    }
}
