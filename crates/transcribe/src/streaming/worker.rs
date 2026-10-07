//! The ASR worker process: whisper.cpp kept resident, decoding buffers on
//! request (see [`super::protocol`]).
//!
//! It runs out of the app's process (ADR-0011) so a whisper.cpp abort, a
//! Core ML compile stall or a Metal fault ends the worker, never playback.
//!
//! Cold start. The first Core ML load of an encoder compiles it for the
//! Neural Engine (31–38 s measured, and macOS evicts that cache from time to
//! time), while the Metal path loads in about 1.5 s with equal accuracy. So
//! the worker always starts on Metal and reports ready at once, then loads
//! the Core ML encoder on a background thread and switches to it between two
//! decodes. Captions never wait for the compile.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use super::protocol::{
    read_line, read_samples, write_line, Accel, DecodeRequest, Decoded, Ready, Request, Response,
    WireWord,
};

/// The argument that selects worker mode in the app binary.
pub const WORKER_FLAG: &str = "--asr-worker";

struct Loaded {
    // whisper-rs states keep their context alive internally; holding the
    // context as well documents the ownership.
    state: WhisperState,
    _ctx: WhisperContext,
    eot: i32,
    accel: Accel,
}

fn load(path: &Path, accel: Accel) -> Result<Loaded, String> {
    let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
        .map_err(|e| format!("loading whisper model: {e}"))?;
    let state = ctx
        .create_state()
        .map_err(|e| format!("creating whisper state: {e}"))?;
    Ok(Loaded {
        eot: ctx.token_eot(),
        state,
        _ctx: ctx,
        accel,
    })
}

/// Where whisper.cpp looks for the Core ML encoder of `model`: the model
/// path without its extension and quantization suffix, plus
/// `-encoder.mlmodelc` (mirrors `whisper_get_coreml_path_encoder`).
pub fn coreml_encoder_path(model: &Path) -> PathBuf {
    let mut stem = model.with_extension("").to_string_lossy().into_owned();
    if let Some(pos) = stem.rfind('-') {
        let suffix = &stem.as_bytes()[pos..];
        if suffix.len() == 5 && suffix[1] == b'q' && suffix[3] == b'_' {
            stem.truncate(pos);
        }
    }
    PathBuf::from(stem + "-encoder.mlmodelc")
}

/// Load on Metal even when a Core ML encoder sits next to the model: through
/// a link whose name has no encoder beside it, whisper.cpp finds no Core ML
/// model and (built with WHISPER_COREML_ALLOW_FALLBACK) keeps the GPU path.
/// The weights are read during the load, so the link is removed right after.
fn load_metal_first(model: &Path) -> Result<Loaded, String> {
    let dir = std::env::temp_dir().join(format!("nova-asr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let link = dir.join("metal-only.bin");
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    std::os::unix::fs::symlink(model, &link).map_err(|e| format!("linking model: {e}"))?;
    let loaded = load(&link, Accel::Metal);
    let _ = std::fs::remove_dir_all(&dir);
    loaded
}

fn decode(engine: &mut Loaded, request: &DecodeRequest, pcm: &[f32]) -> Result<Decoded, String> {
    let started = Instant::now();
    // The settings the AI22 benchmark measured: greedy, temperature fallback
    // on, full 30 s encoder context (a shorter one made whisper hallucinate).
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(4);
    params.set_language(Some(request.language.as_deref().unwrap_or("auto")));
    params.set_token_timestamps(true);
    // Continuity comes from the prompt alone; nothing else carries over
    // between decodes of an overlapping buffer.
    params.set_no_context(true);
    if !request.prompt.is_empty() {
        params.set_initial_prompt(&request.prompt);
    }
    params.set_suppress_blank(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    engine
        .state
        .full(params, pcm)
        .map_err(|e| format!("whisper decode failed: {e}"))?;
    let language = whisper_rs::get_lang_str(engine.state.full_lang_id_from_state())
        .map(str::to_string);

    // Rebuild words from tokens: a token starting with a space begins a new
    // word. Bytes are joined before decoding, since a multibyte character
    // (Arabic) can be split across tokens.
    let mut words = Vec::new();
    let mut current: Option<(Vec<u8>, f64, f64)> = None;
    for segment in engine.state.as_iter() {
        for i in 0..segment.n_tokens() {
            let Some(token) = segment.get_token(i) else {
                continue;
            };
            if token.token_id() >= engine.eot {
                continue;
            }
            let data = token.token_data();
            let t0 = request.offset_sec + data.t0 as f64 / 100.0;
            let t1 = request.offset_sec + data.t1 as f64 / 100.0;
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
    Ok(Decoded {
        words,
        language,
        accel: engine.accel,
        decode_ms: started.elapsed().as_millis() as u64,
    })
}

fn push_word(words: &mut Vec<WireWord>, bytes: &[u8], start: f64, end: f64) {
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    if !text.is_empty() {
        words.push(WireWord {
            text,
            start,
            end: end.max(start),
        });
    }
}

/// Keep the protocol stream private: whisper.cpp and ggml log with printf,
/// and a stray line on stdout would corrupt the protocol. Returns a handle
/// to the original stdout and points fd 1 at stderr.
#[cfg(unix)]
fn take_stdout() -> io::Result<File> {
    use std::os::fd::FromRawFd;
    // SAFETY: plain descriptor duplication on fds this process owns; the
    // duplicate is owned by the returned File from here on.
    unsafe {
        let protocol = libc::dup(1);
        if protocol < 0 || libc::dup2(2, 1) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(File::from_raw_fd(protocol))
    }
}

/// Worker entry point. `args` are the arguments after [`WORKER_FLAG`]:
/// the GGML model path. Returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let Some(model) = args.first().map(PathBuf::from) else {
        eprintln!("nova asr worker: missing model path");
        return 2;
    };
    match serve(&model) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("nova asr worker: {e}");
            1
        }
    }
}

fn serve(model: &Path) -> io::Result<()> {
    #[cfg(unix)]
    let stdout = take_stdout()?;
    #[cfg(not(unix))]
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout);
    let mut input = BufReader::new(io::stdin().lock());

    let started = Instant::now();
    let coreml = cfg!(target_os = "macos") && coreml_encoder_path(model).is_dir();
    let first = if coreml {
        load_metal_first(model)
    } else {
        let accel = if cfg!(target_os = "macos") {
            Accel::Metal
        } else {
            Accel::Cpu
        };
        load(model, accel)
    };
    let mut engine = match first {
        Ok(engine) => engine,
        Err(e) => {
            write_line(&mut out, &Response::Failed(e))?;
            return Ok(());
        }
    };
    write_line(
        &mut out,
        &Response::Ready(Ready {
            accel: engine.accel,
            load_ms: started.elapsed().as_millis() as u64,
        }),
    )?;

    let (upgrade_tx, upgrade) = mpsc::channel();
    if coreml {
        let model = model.to_path_buf();
        std::thread::spawn(move || {
            match load(&model, Accel::Coreml) {
                Ok(loaded) => {
                    let _ = upgrade_tx.send(loaded);
                }
                // Stay on Metal: slower steps, same accuracy.
                Err(e) => eprintln!("nova asr worker: Core ML upgrade failed: {e}"),
            }
        });
    }

    while let Some(Request::Decode(request)) = read_line::<Request>(&mut input)? {
        let pcm = read_samples(&mut input, request.samples)?;
        if let Ok(upgraded) = upgrade.try_recv() {
            // The Metal engine drops here, releasing its memory.
            engine = upgraded;
        }
        let response = match decode(&mut engine, &request, &pcm) {
            Ok(decoded) => Response::Decoded(decoded),
            Err(e) => Response::Failed(e),
        };
        write_line(&mut out, &response)?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coreml_encoder_path_matches_whisper_cpp() {
        assert_eq!(
            coreml_encoder_path(Path::new("/m/ggml-large-v3-turbo-q5_0.bin")),
            PathBuf::from("/m/ggml-large-v3-turbo-encoder.mlmodelc")
        );
        assert_eq!(
            coreml_encoder_path(Path::new("/m/ggml-small.bin")),
            PathBuf::from("/m/ggml-small-encoder.mlmodelc")
        );
        // The Metal-first link has no encoder beside it.
        assert_eq!(
            coreml_encoder_path(Path::new("/tmp/x/metal-only.bin")),
            PathBuf::from("/tmp/x/metal-only-encoder.mlmodelc")
        );
    }
}
