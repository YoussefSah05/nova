//! Stream a 16 kHz mono f32 file through the resident ASR worker at real
//! time and print provisional and committed text with wall-clock times.
//! This binary also serves as its own worker, like the app does.
//!
//!   ffmpeg -i clip.wav -ac 1 -ar 16000 -f f32le clip.f32
//!   cargo run --release -p nova-transcribe --example live_asr -- MODEL.bin clip.f32 [lang]

use std::time::{Duration, Instant};

use nova_core::intelligence::{AsrSessionConfig, AsrUpdate, PcmQueue, SpeechToTextEngine};
use nova_transcribe::streaming::{worker, WhisperStreamingEngine, WorkerSpec};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(worker::WORKER_FLAG) {
        std::process::exit(worker::run(&args[1..]));
    }
    let [model, audio, rest @ ..] = args.as_slice() else {
        eprintln!("usage: live_asr MODEL.bin AUDIO.f32 [lang]");
        std::process::exit(2);
    };
    let bytes = std::fs::read(audio).expect("reading audio");
    let pcm: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();

    let started = Instant::now();
    let engine = WhisperStreamingEngine::start(WorkerSpec::current_exe(model.into()).unwrap())
        .expect("starting the ASR worker");
    println!("[{:5.2}s] worker ready on {:?}", started.elapsed().as_secs_f64(), engine.accel());

    let queue = PcmQueue::with_capacity(16_000 * 60);
    let config = AsrSessionConfig {
        language: rest.first().cloned(),
        sample_rate: 16_000,
        step_ms: 1000,
    };
    let mut session = engine.open_session(config, queue.clone()).unwrap();
    let clock = Instant::now();
    let chunk = 1_600; // 100 ms, as an audio tap would deliver it
    let mut fed = 0;
    let print = |updates: Vec<AsrUpdate>, accel| {
        for update in updates {
            let t = clock.elapsed().as_secs_f64();
            match update {
                AsrUpdate::Committed(s) => println!("[{t:5.2}s] COMMIT {:?} {}", accel, s[0].text),
                AsrUpdate::Provisional(s) if !s.text.is_empty() => println!("[{t:5.2}s]   ...  {}", s.text),
                AsrUpdate::Provisional(_) => {}
            }
        }
    };
    while fed < pcm.len() {
        let due = clock.elapsed().as_secs_f64() * 16_000.0;
        while fed < pcm.len() && (fed as f64) < due {
            let end = (fed + chunk).min(pcm.len());
            queue.push(&pcm[fed..end]);
            fed = end;
        }
        print(session.poll().unwrap_or_default(), engine.accel());
        std::thread::sleep(Duration::from_millis(20));
    }
    print(session.flush().unwrap_or_default(), engine.accel());
    println!("dropped samples: {}", queue.dropped());
}
