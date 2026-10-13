//! Print Whisper DTW word times next to the aligned ones for one recording.
//!
//! ```text
//! cargo run --release -p nova-align --example align_trace -- \
//!     <app-data-dir> <aligner-dir> <vad-dir> <file.wav> <language>
//! ```

use std::path::PathBuf;

use nova_align::{refine, Aligner, SpeechDetector};
use nova_transcribe::whisper::configure_whisper_cli;
use nova_transcribe::{default_model, model_paths, transcribe_wav, TranscribeOptions};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [app, aligner, vad, wav, language] = &args[..] else {
        eprintln!(
            "usage: align_trace <app-data-dir> <aligner-dir> <vad-dir> <file.wav> <language>"
        );
        std::process::exit(2);
    };
    let app = PathBuf::from(app);
    configure_whisper_cli(&app);
    let model = default_model();
    let ggml = model_paths(&app.join("models/whisper"), &model).ggml;
    let out = tempfile::tempdir().unwrap();
    let opts = TranscribeOptions {
        language: Some(language.clone()),
        model_id: model.id.into(),
        dtw_preset: model.dtw_preset.into(),
        initial_prompt: None,
        max_len: 60,
    };
    let wav = PathBuf::from(wav);
    let t = transcribe_wav(&wav, &ggml, &out.path().join("t"), &opts, |_| {})
        .await
        .unwrap();
    let pcm = nova_media_free_read(&wav);
    let regions = SpeechDetector::load(vad.as_ref())
        .unwrap()
        .regions(&pcm)
        .unwrap();
    let emissions = Aligner::load(aligner.as_ref())
        .unwrap()
        .emissions(&pcm)
        .unwrap();
    let mut segs = t.segments.clone();
    let report = refine(&mut segs, 0.0, Some(&regions), Some(&emissions));
    println!("speech regions: {regions:.2?}\n{report:?}");
    let before: Vec<_> = t
        .segments
        .iter()
        .flat_map(|s| s.words.clone().unwrap_or_default())
        .collect();
    let after: Vec<_> = segs
        .iter()
        .flat_map(|s| s.words.clone().unwrap_or_default())
        .collect();
    for (b, a) in before.iter().zip(&after) {
        println!(
            "{:<18} dtw {:>6.2}–{:<6.2} aligned {:>6.2}–{:<6.2} [{}]",
            b.word,
            b.start_sec,
            b.end_sec,
            a.start_sec,
            a.end_sec,
            nova_align::romanize::romanize(&a.word)
        );
    }
}

fn nova_media_free_read(path: &std::path::Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    let data = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
    bytes[data..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}
