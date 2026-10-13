//! Time to first caption, before and after the resident caption pipeline.
//!
//! Runs the windows a caption session would run over a real 16 kHz mono
//! recording, two ways:
//! - **per-window CLI** (before): one `whisper-cli` process per 30 s window,
//!   so every window loads the model again;
//! - **resident worker** (after): one worker process, a short first window,
//!   and the VAD + CTC timing stage inside the worker.
//!
//! This binary is also its own ASR worker, like the app. Set
//! `CAPTION_BENCH=before` or `after` to run one side only (for example
//! against a freshly copied Core ML encoder, to measure a cold compile).
//!
//! ```text
//! cargo run --release -p nova-transcribe --example caption_cold_start -- \
//!     <app-data-dir> <aligner-dir> <vad-dir> <speech.wav> <language> [windows]
//! ```

use std::path::PathBuf;
use std::time::Instant;

use nova_transcribe::streaming::{
    segments_of, worker, DecodeRequest, TimingModels, WhisperStreamingEngine, WorkerSpec,
};
use nova_transcribe::whisper::configure_whisper_cli;
use nova_transcribe::{default_model, model_paths, transcribe_wav, TranscribeOptions};

const SR: usize = 16_000;

fn read_pcm(path: &std::path::Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    let data = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
    bytes[data..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}

fn write_wav(path: &std::path::Path, pcm: &[f32]) {
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    let data = (pcm.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(SR as u32).to_le_bytes());
    out.extend_from_slice(&(SR as u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(worker::WORKER_FLAG) {
        std::process::exit(worker::run(&args[1..]));
    }
    let [app, aligner, vad, wav, language, rest @ ..] = args.as_slice() else {
        eprintln!("usage: caption_cold_start <app-data-dir> <aligner-dir> <vad-dir> <speech.wav> <language> [windows]");
        std::process::exit(2);
    };
    let windows: usize = rest.first().and_then(|n| n.parse().ok()).unwrap_or(4);
    let app = PathBuf::from(app);
    configure_whisper_cli(&app);
    let model = default_model();
    let ggml = model_paths(&app.join("models/whisper"), &model).ggml;
    let pcm = read_pcm(PathBuf::from(wav).as_path());
    let tmp = tempfile::tempdir().unwrap();

    // Window plans: [start, end) of the audio handed to the recognizer.
    let plan = |first: f64, rest: f64| {
        let mut out = vec![(0.0, first)];
        let mut at = first;
        while out.len() < windows {
            out.push(((at - 3.0).max(0.0), at + rest));
            at += rest;
        }
        out
    };
    let slice = |(s, e): (f64, f64)| {
        let (s, e) = (
            (s * SR as f64) as usize,
            ((e * SR as f64) as usize).min(pcm.len()),
        );
        pcm[s..e].to_vec()
    };

    let only = std::env::var("CAPTION_BENCH").unwrap_or_default();
    if only != "after" {
        before(
            &ggml,
            &model,
            language,
            &plan(30.0, 27.0),
            &slice,
            tmp.path(),
        )
        .await;
    }
    if only != "before" {
        after(&ggml, aligner, vad, language, &plan(11.0, 27.0), &slice);
    }
}

async fn before(
    ggml: &std::path::Path,
    model: &nova_transcribe::WhisperModel,
    language: &str,
    plan: &[(f64, f64)],
    slice: &dyn Fn((f64, f64)) -> Vec<f32>,
    tmp: &std::path::Path,
) {
    println!("== Before: one whisper-cli process per 30 s window");
    let opts = TranscribeOptions {
        language: Some(language.to_string()),
        model_id: model.id.into(),
        dtw_preset: model.dtw_preset.into(),
        initial_prompt: None,
        max_len: 60,
    };
    let started = Instant::now();
    for (i, &window) in plan.iter().enumerate() {
        let path = tmp.join(format!("cli{i}.wav"));
        write_wav(&path, &slice(window));
        let t = Instant::now();
        let out = transcribe_wav(&path, ggml, &tmp.join(format!("cli{i}")), &opts, |_| {})
            .await
            .unwrap();
        println!(
            "window {i} [{:>5.1}–{:>5.1} s]: {:.2} s ({} segments){}",
            window.0,
            window.1,
            t.elapsed().as_secs_f64(),
            out.segments.len(),
            if i == 0 {
                format!(
                    "  ← first caption at {:.2} s",
                    started.elapsed().as_secs_f64()
                )
            } else {
                String::new()
            }
        );
    }
}

fn after(
    ggml: &std::path::Path,
    aligner: &str,
    vad: &str,
    language: &str,
    plan: &[(f64, f64)],
    slice: &dyn Fn((f64, f64)) -> Vec<f32>,
) {
    println!("\n== After: resident worker, 11 s first window, VAD + CTC timing");
    let started = Instant::now();
    let engine =
        WhisperStreamingEngine::start(WorkerSpec::current_exe(ggml.to_path_buf()).unwrap())
            .expect("worker");
    println!(
        "worker ready on {:?} in {:.2} s",
        engine.accel(),
        started.elapsed().as_secs_f64()
    );
    let timing = TimingModels {
        aligner: Some(aligner.into()),
        vad: Some(vad.into()),
    };
    for (i, &window) in plan.iter().enumerate() {
        let audio = slice(window);
        let request = DecodeRequest {
            samples: audio.len(),
            offset_sec: window.0,
            prompt: String::new(),
            language: Some(language.to_string()),
            max_len: 60,
            timing: Some(timing.clone()),
        };
        let t = Instant::now();
        let decoded = engine.decode(&request, &audio).expect("decode");
        let report = decoded.timing.clone().unwrap_or_default();
        println!(
            "window {i} [{:>5.1}–{:>5.1} s]: {:.2} s (whisper {} ms, timing {} ms, {} words aligned, {} segments){}{}",
            window.0,
            window.1,
            t.elapsed().as_secs_f64(),
            decoded.decode_ms,
            report.timing_ms,
            report.aligned_words,
            segments_of(&decoded.words).len(),
            if i == 0 {
                format!("  ← first caption at {:.2} s", started.elapsed().as_secs_f64())
            } else {
                String::new()
            },
            report.error.map(|e| format!(" [timing error: {e}]")).unwrap_or_default()
        );
    }
    engine.shutdown();
}
