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
//!
//! Caption timing. When a request names them, the worker also runs the
//! Silero VAD before Whisper (a buffer with no speech is never decoded, so
//! Whisper cannot invent dialogue over music) and the MMS CTC aligner after
//! it (word times tied to the audio, see `nova_align`). Both stay loaded
//! beside Whisper; a failure in either keeps Whisper's own times.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use whisper_rs::{
    DtwMode, DtwModelPreset, DtwParameters, FullParams, SamplingStrategy, WhisperContext,
    WhisperContextParameters, WhisperState,
};

use nova_align::{refine, Aligner, SpeechDetector};
use nova_core::{TranscriptSegment, TranscriptWord};

use super::protocol::{
    read_line, read_samples, write_line, Accel, DecodeRequest, Decoded, Ready, Request, Response,
    TimingModels, TimingReport, WireWord,
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

/// The DTW alignment-head preset of a model file, from its whisper.cpp name.
/// DTW over these cross-attention heads gives token times tied to the audio;
/// without it whisper.cpp only interpolates times between timestamp tokens.
/// It needs flash attention off, which is whisper-rs's default.
pub fn dtw_preset(model: &Path) -> Option<DtwModelPreset> {
    let name = model.file_name()?.to_str()?;
    let stem = name.strip_prefix("ggml-")?.trim_end_matches(".bin");
    let family = stem.split("-q").next().unwrap_or(stem);
    Some(match family {
        "large-v3-turbo" => DtwModelPreset::LargeV3Turbo,
        "large-v3" => DtwModelPreset::LargeV3,
        "large-v2" => DtwModelPreset::LargeV2,
        "medium" => DtwModelPreset::Medium,
        "small" => DtwModelPreset::Small,
        "base" => DtwModelPreset::Base,
        "tiny" => DtwModelPreset::Tiny,
        _ => return None,
    })
}

fn load(path: &Path, accel: Accel, dtw: Option<DtwModelPreset>) -> Result<Loaded, String> {
    let mut params = WhisperContextParameters::default();
    if let Some(model_preset) = dtw {
        params.flash_attn(false).dtw_parameters(DtwParameters {
            mode: DtwMode::ModelPreset { model_preset },
            ..DtwParameters::default()
        });
    }
    let ctx = WhisperContext::new_with_params(path, params)
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
fn load_metal_first(model: &Path, dtw: Option<DtwModelPreset>) -> Result<Loaded, String> {
    // A private, unpredictable directory (mkdtemp): nobody else can plant
    // a link there.
    let dir = tempfile::Builder::new()
        .prefix("nova-asr-")
        .tempdir()
        .map_err(|e| format!("creating a temporary directory: {e}"))?;
    let link = dir.path().join("metal-only.bin");
    #[cfg(unix)]
    std::os::unix::fs::symlink(model, &link).map_err(|e| format!("linking model: {e}"))?;
    load(&link, Accel::Metal, dtw)
    // `dir` is removed here; the weights were read during the load.
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
    if request.max_len > 0 {
        params.set_max_len(request.max_len as i32);
        params.set_split_on_word(true);
    }
    engine
        .state
        .full(params, pcm)
        .map_err(|e| format!("whisper decode failed: {e}"))?;
    let language =
        whisper_rs::get_lang_str(engine.state.full_lang_id_from_state()).map(str::to_string);

    // Rebuild words from tokens: a token starting with a space begins a new
    // word. Bytes are joined before decoding, since a multibyte character
    // (Arabic) can be split across tokens.
    let mut words = Vec::new();
    let mut current: Option<(Vec<u8>, f64, f64, u32)> = None;
    for (index, segment) in engine.state.as_iter().enumerate() {
        let index = index as u32;
        for i in 0..segment.n_tokens() {
            let Some(token) = segment.get_token(i) else {
                continue;
            };
            if token.token_id() >= engine.eot {
                continue;
            }
            let data = token.token_data();
            let (t0, t1) = token_times(data.t0, data.t1, data.t_dtw);
            let t0 = request.offset_sec + t0;
            let t1 = request.offset_sec + t1;
            let bytes = token.to_bytes().unwrap_or_default();
            // A new segment always starts a new word.
            let starts_word =
                bytes.first() == Some(&b' ') || current.as_ref().is_some_and(|c| c.3 != index);
            match &mut current {
                Some((buf, _, end, _)) if !starts_word => {
                    buf.extend_from_slice(bytes);
                    *end = t1;
                }
                _ => {
                    if let Some((buf, s, e, seg)) = current.take() {
                        push_word(&mut words, &buf, s, e, seg);
                    }
                    current = Some((bytes.to_vec(), t0, t1, index));
                }
            }
        }
    }
    if let Some((buf, s, e, seg)) = current.take() {
        push_word(&mut words, &buf, s, e, seg);
    }
    Ok(Decoded {
        words,
        language,
        accel: engine.accel,
        decode_ms: started.elapsed().as_millis() as u64,
        timing: None,
    })
}

/// The caption timing models this worker has loaded, keyed by directory.
#[derive(Default)]
struct TimingEngine {
    aligner: Option<(PathBuf, Aligner)>,
    vad: Option<(PathBuf, SpeechDetector)>,
}

impl TimingEngine {
    /// Load what `models` names and is not loaded yet. Errors are reported,
    /// and the stage then runs without that model.
    fn prepare(&mut self, models: &TimingModels) -> Option<String> {
        let mut error = None;
        if let Some(dir) = &models.vad {
            if self.vad.as_ref().is_none_or(|(d, _)| d != dir) {
                match SpeechDetector::load(dir) {
                    Ok(vad) => self.vad = Some((dir.clone(), vad)),
                    Err(e) => error = Some(e.to_string()),
                }
            }
        }
        if let Some(dir) = &models.aligner {
            if self.aligner.as_ref().is_none_or(|(d, _)| d != dir) {
                match Aligner::load(dir) {
                    Ok(aligner) => self.aligner = Some((dir.clone(), aligner)),
                    Err(e) => error = Some(e.to_string()),
                }
            }
        }
        error
    }

    fn vad(&self, models: &TimingModels) -> Option<&SpeechDetector> {
        self.vad
            .as_ref()
            .filter(|(d, _)| models.vad.as_ref() == Some(d))
            .map(|(_, v)| v)
    }

    fn aligner(&self, models: &TimingModels) -> Option<&Aligner> {
        self.aligner
            .as_ref()
            .filter(|(d, _)| models.aligner.as_ref() == Some(d))
            .map(|(_, a)| a)
    }
}

/// Decode with the caption timing stage around Whisper.
fn decode_timed(
    engine: &mut Loaded,
    timing: &mut TimingEngine,
    models: &TimingModels,
    request: &DecodeRequest,
    pcm: &[f32],
) -> Result<Decoded, String> {
    let started = Instant::now();
    let mut report = TimingReport {
        error: timing.prepare(models),
        ..TimingReport::default()
    };
    let speech = match timing.vad(models).map(|v| v.regions(pcm)) {
        Some(Ok(regions)) => Some(regions),
        Some(Err(e)) => {
            report.error = Some(e.to_string());
            None
        }
        None => None,
    };
    if let Some(regions) = &speech {
        let total: f64 = regions.iter().map(|(s, e)| e - s).sum();
        report.speech_sec = Some(total);
        if regions.is_empty() {
            report.skipped_silence = true;
            report.timing_ms = started.elapsed().as_millis() as u64;
            return Ok(Decoded {
                words: Vec::new(),
                language: None,
                accel: engine.accel,
                decode_ms: 0,
                timing: Some(report),
            });
        }
    }
    let mut decoded = decode(engine, request, pcm)?;
    let timing_started = Instant::now();
    let emissions = match timing.aligner(models).map(|a| a.emissions(pcm)) {
        Some(Ok(emissions)) => Some(emissions),
        Some(Err(e)) => {
            report.error = Some(e.to_string());
            None
        }
        None => None,
    };
    let mut segments = segments_of(&decoded.words);
    let refined = refine(
        &mut segments,
        request.offset_sec,
        speech.as_deref(),
        emissions.as_ref(),
    );
    report.aligned_words = refined.aligned;
    report.interpolated_words = refined.interpolated;
    report.dropped_segments = refined.dropped_segments;
    report.timing_ms = timing_started.elapsed().as_millis() as u64;
    decoded.words = from_segments(segments);
    decoded.timing = Some(report);
    Ok(decoded)
}

/// Group decoded words into caption segments by their whisper segment.
pub fn segments_of(words: &[WireWord]) -> Vec<TranscriptSegment> {
    let mut segments: Vec<TranscriptSegment> = Vec::new();
    let mut current: Option<u32> = None;
    for w in words {
        if current != Some(w.segment) || segments.is_empty() {
            current = Some(w.segment);
            segments.push(TranscriptSegment {
                seq: segments.len() as i64,
                start_sec: w.start,
                end_sec: w.end,
                text: String::new(),
                words: Some(Vec::new()),
            });
        }
        let segment = segments.last_mut().expect("pushed");
        if !segment.text.is_empty() {
            segment.text.push(' ');
        }
        segment.text.push_str(&w.text);
        segment.end_sec = w.end;
        segment
            .words
            .get_or_insert_with(Vec::new)
            .push(TranscriptWord {
                start_sec: w.start,
                end_sec: w.end,
                word: w.text.clone(),
            });
    }
    segments
}

fn from_segments(segments: Vec<TranscriptSegment>) -> Vec<WireWord> {
    segments
        .into_iter()
        .enumerate()
        .flat_map(|(i, s)| {
            s.words
                .unwrap_or_default()
                .into_iter()
                .map(move |w| WireWord {
                    text: w.word,
                    start: w.start_sec,
                    end: w.end_sec,
                    segment: i as u32,
                })
        })
        .collect()
}

/// A token's (start, end) in seconds. DTW gives one instant per token, the
/// moment it is spoken, which is more faithful than the interpolated
/// `t0`/`t1`; the token then lasts until the next one starts, so only the
/// start is taken from DTW and the end keeps `t1` when it is later.
fn token_times(t0: i64, t1: i64, t_dtw: i64) -> (f64, f64) {
    if t_dtw >= 0 {
        let at = t_dtw as f64 / 100.0;
        (at, (t1 as f64 / 100.0).max(at))
    } else {
        (t0 as f64 / 100.0, t1 as f64 / 100.0)
    }
}

fn push_word(words: &mut Vec<WireWord>, bytes: &[u8], start: f64, end: f64, segment: u32) {
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    if !text.is_empty() {
        words.push(WireWord {
            text,
            start,
            end: end.max(start),
            segment,
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
    let dtw = dtw_preset(model);
    let first = if coreml {
        load_metal_first(model, dtw.clone())
    } else {
        let accel = if cfg!(target_os = "macos") {
            Accel::Metal
        } else {
            Accel::Cpu
        };
        load(model, accel, dtw.clone())
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
            // whisper.cpp falls back to Metal if the encoder fails to load
            // (WHISPER_COREML_ALLOW_FALLBACK), so `coreml` means "Core ML
            // load attempted after its compile"; its log on stderr says
            // whether the encoder actually loaded.
            match load(&model, Accel::Coreml, dtw) {
                Ok(loaded) => {
                    let _ = upgrade_tx.send(loaded);
                }
                // Stay on Metal: slower steps, same accuracy.
                Err(e) => eprintln!("nova asr worker: Core ML upgrade failed: {e}"),
            }
        });
    }

    let mut timing = TimingEngine::default();
    while let Some(Request::Decode(request)) = read_line::<Request>(&mut input)? {
        let pcm = read_samples(&mut input, request.samples)?;
        if let Ok(upgraded) = upgrade.try_recv() {
            // The Metal engine drops here, releasing its memory.
            engine = upgraded;
        }
        let result = match &request.timing {
            Some(models) => decode_timed(&mut engine, &mut timing, models, &request, &pcm),
            None => decode(&mut engine, &request, &pcm),
        };
        let response = match result {
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
    fn words_round_trip_through_segments() {
        let w = |text: &str, start: f64, segment: u32| WireWord {
            text: text.into(),
            start,
            end: start + 0.2,
            segment,
        };
        let words = vec![
            w("Bonsoir", 1.0, 0),
            w("et", 1.3, 0),
            w("bienvenue.", 2.0, 3),
        ];
        let segments = segments_of(&words);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "Bonsoir et");
        assert!((segments[0].end_sec - 1.5).abs() < 1e-9);
        let back = from_segments(segments);
        assert_eq!(
            back.iter().map(|w| w.segment).collect::<Vec<_>>(),
            [0, 0, 1]
        );
        assert_eq!(back[2].text, "bienvenue.");
    }

    #[test]
    fn dtw_presets_follow_the_model_family() {
        assert!(matches!(
            dtw_preset(Path::new("/m/ggml-large-v3-turbo-q5_0.bin")),
            Some(DtwModelPreset::LargeV3Turbo)
        ));
        assert!(matches!(
            dtw_preset(Path::new("/m/ggml-large-v3.bin")),
            Some(DtwModelPreset::LargeV3)
        ));
        assert!(matches!(
            dtw_preset(Path::new("/m/ggml-small-q8_0.bin")),
            Some(DtwModelPreset::Small)
        ));
        assert!(dtw_preset(Path::new("/m/custom.bin")).is_none());
    }

    #[test]
    fn dtw_times_win_when_present() {
        assert_eq!(token_times(120, 160, 226), (2.26, 2.26));
        assert_eq!(token_times(120, 300, 226), (2.26, 3.0));
        assert_eq!(token_times(120, 160, -1), (1.2, 1.6));
    }

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
