//! Word-timing evaluation: how far caption word times land from reference
//! word boundaries, for Whisper's interpolated timestamps, Whisper DTW, and
//! DTW refined by the VAD and the CTC aligner.
//!
//! ```text
//! cargo run --release -p nova-align --example timing_eval -- \
//!     <app-data-dir> <aligner-dir> <vad-dir> <windows-dir>
//! ```
//!
//! `<windows-dir>` holds `NAME.wav` (16 kHz mono) and `NAME.json`
//! (`{"words":[{"word","start","end"}]}`) pairs. The published numbers in
//! `docs/evaluations/caption-timing-2026-10-13.md` come from LibriSpeech
//! test-clean with Montreal Forced Aligner references.

use std::path::{Path, PathBuf};
use std::time::Instant;

use nova_align::{refine, Aligner, SpeechDetector};
use nova_core::TranscriptSegment;
use nova_transcribe::whisper::configure_whisper_cli;
use nova_transcribe::{default_model, model_paths, transcribe_wav, TranscribeOptions};
use serde::Deserialize;

#[derive(Deserialize)]
struct Reference {
    words: Vec<RefWord>,
}

#[derive(Deserialize)]
struct RefWord {
    word: String,
    start: f64,
    end: f64,
}

fn norm(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Longest-common-subsequence matching of hypothesis to reference words.
fn matches(hyp: &[(String, f64, f64)], refw: &[RefWord]) -> Vec<(usize, usize)> {
    let (n, m) = (hyp.len(), refw.len());
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if hyp[i].0 == norm(&refw[j].word) && !hyp[i].0.is_empty() {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n && j < m {
        if hyp[i].0 == norm(&refw[j].word) && !hyp[i].0.is_empty() {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

#[derive(Default)]
struct Stats {
    start: Vec<f64>,
    end: Vec<f64>,
    reference_words: usize,
    secs: f64,
}

impl Stats {
    fn add(&mut self, segs: &[TranscriptSegment], refw: &[RefWord]) {
        let hyp: Vec<(String, f64, f64)> = segs
            .iter()
            .flat_map(|s| s.words.clone().unwrap_or_default())
            .map(|w| (norm(&w.word), w.start_sec, w.end_sec))
            .collect();
        for (i, j) in matches(&hyp, refw) {
            self.start.push((hyp[i].1 - refw[j].start).abs());
            self.end.push((hyp[i].2 - refw[j].end).abs());
        }
        self.reference_words += refw.len();
    }

    fn report(&self, name: &str) {
        let summary = |v: &[f64]| {
            let mut v = v.to_vec();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mean = v.iter().sum::<f64>() / v.len().max(1) as f64;
            let pct = |t: f64| {
                100.0 * v.iter().filter(|&&x| x <= t).count() as f64 / v.len().max(1) as f64
            };
            format!(
                "mean {:>4.0} ms | median {:>4.0} ms | p90 {:>5.0} ms | ≤100 ms {:>5.1}% | ≤200 ms {:>5.1}%",
                mean * 1000.0,
                v.get(v.len() / 2).copied().unwrap_or(0.0) * 1000.0,
                v.get(v.len() * 9 / 10).copied().unwrap_or(0.0) * 1000.0,
                pct(0.1),
                pct(0.2)
            )
        };
        println!(
            "{name}\n  matched {}/{} words, {:.2} s compute\n  start: {}\n  end:   {}",
            self.start.len(),
            self.reference_words,
            self.secs,
            summary(&self.start),
            summary(&self.end)
        );
    }
}

fn read_pcm(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    // Our own 16-bit mono WAVs with a 44-byte header.
    bytes[44..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}

#[tokio::main]
async fn main() {
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let [app, aligner_dir, vad_dir, windows] = &args[..] else {
        eprintln!("usage: timing_eval <app-data-dir> <aligner-dir> <vad-dir> <windows-dir>");
        std::process::exit(2);
    };
    configure_whisper_cli(app);
    let model = default_model();
    let ggml = model_paths(&app.join("models/whisper"), &model).ggml;
    let t = Instant::now();
    let aligner = Aligner::load(aligner_dir).expect("aligner");
    let vad = SpeechDetector::load(vad_dir).expect("vad");
    println!("models loaded in {:.2} s", t.elapsed().as_secs_f64());

    let mut names: Vec<PathBuf> = std::fs::read_dir(windows)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "wav"))
        .collect();
    names.sort();
    let (mut shipped, mut dtw, mut aligned) =
        (Stats::default(), Stats::default(), Stats::default());
    let (mut vad_secs, mut ctc_secs, mut dropped) = (0.0, 0.0, 0);
    let out = tempfile::tempdir().unwrap();
    for wav in &names {
        let reference: Reference =
            serde_json::from_str(&std::fs::read_to_string(wav.with_extension("json")).unwrap())
                .unwrap();
        let mut opts = TranscribeOptions {
            language: Some("en".into()),
            model_id: model.id.into(),
            dtw_preset: String::new(),
            initial_prompt: None,
            max_len: 60,
        };
        let t = Instant::now();
        let plain = transcribe_wav(wav, &ggml, &out.path().join("plain"), &opts, |_| {})
            .await
            .unwrap();
        shipped.secs += t.elapsed().as_secs_f64();
        shipped.add(&plain.segments, &reference.words);

        opts.dtw_preset = model.dtw_preset.into();
        let t = Instant::now();
        let with_dtw = transcribe_wav(wav, &ggml, &out.path().join("dtw"), &opts, |_| {})
            .await
            .unwrap();
        dtw.secs += t.elapsed().as_secs_f64();
        dtw.add(&with_dtw.segments, &reference.words);

        let pcm = read_pcm(wav);
        let t = Instant::now();
        let regions = vad.regions(&pcm).unwrap();
        vad_secs += t.elapsed().as_secs_f64();
        let t = Instant::now();
        let emissions = aligner.emissions(&pcm).unwrap();
        ctc_secs += t.elapsed().as_secs_f64();
        let mut segs = with_dtw.segments.clone();
        let report = refine(&mut segs, 0.0, Some(&regions), Some(&emissions));
        dropped += report.dropped_segments;
        aligned.secs += t.elapsed().as_secs_f64();
        aligned.add(&segs, &reference.words);
        if std::env::var_os("TIMING_EVAL_TRACE").is_some() {
            for s in segs.iter().rev().take(2) {
                eprintln!(
                    "  tail segment {:?} at {:.2}-{:.2}",
                    s.text, s.start_sec, s.end_sec
                );
            }
        }
        let kept: Vec<&str> = segs.iter().map(|s| s.text.as_str()).collect();
        for s in &with_dtw.segments {
            if !kept.contains(&s.text.as_str()) {
                eprintln!(
                    "  dropped as non-speech: {:?} at {:.2}-{:.2}",
                    s.text, s.start_sec, s.end_sec
                );
            }
        }
        eprintln!("{}: {report:?}", wav.file_name().unwrap().to_string_lossy());
    }
    println!("\n{} windows", names.len());
    shipped.report("Whisper interpolated timestamps (shipped before this change)");
    dtw.report("Whisper DTW (flash attention off)");
    aligned.report("Whisper DTW + Silero VAD + MMS CTC alignment");
    println!(
        "VAD {:.2} s, CTC emissions {:.2} s total; {} segments dropped as non-speech",
        vad_secs, ctc_secs, dropped
    );
}
