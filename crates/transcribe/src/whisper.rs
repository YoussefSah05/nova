//! `whisper-cli` subprocess driver + JSON output parsing.
//!
//! We shell out to `whisper-cli` (the whisper.cpp CLI, Homebrew formula
//! `whisper-cpp`) rather than linking libwhisper — same pattern as our ffmpeg
//! and mpv handling. The CLI is built with Core ML, which it discovers
//! automatically next to the GGML weights. Flags follow
//! `docs/reference/whisper.md`: transcribe (whisper.cpp's default — there
//! is no `--task` flag), word/token timestamps via `--output-json-full`, and
//! `--dtw <preset>` for **DTW-aligned** token timestamps (dynamic time warping
//! over the cross-attention heads — far more accurate than whisper's default
//! heuristic offsets, and the source of our word-level cue timing). `-nc` for
//! Arabic; `--prompt` carries the previous window's tail across boundaries.
//!
//! Note: we deliberately do **not** pass `--vad`. In this whisper.cpp build VAD
//! remaps segment offsets to the original timeline but leaves the per-token DTW
//! offsets in the VAD-compressed timeline, which corrupts the word timings we
//! rely on. Adaptive, pause-aligned chunking is achieved at the session layer
//! using whisper's natural segment boundaries instead.

use std::path::Path;
use std::process::Stdio;

use nova_core::{RawTranscript, TranscriptSegment, TranscriptWord};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum WhisperError {
    #[error("whisper-cli failed to start (is it installed / on PATH?): {0}")]
    Spawn(#[source] std::io::Error),
    #[error("whisper-cli exited with status {0:?}: {1}")]
    Failed(Option<i32>, String),
    #[error("could not read whisper output {0}: {1}")]
    Output(String, #[source] std::io::Error),
    #[error("whisper output was not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Candidate binary names for whisper.cpp's CLI, newest naming first. The
/// Homebrew `whisper-cpp` formula installs `whisper-cli`; older builds shipped
/// `main`/`whisper-cpp`.
const WHISPER_CANDIDATES: &[&str] = &["whisper-cli", "whisper-cpp", "main"];

/// Resolve the whisper CLI binary. Honours `NOVA_WHISPER_BIN`; otherwise probes
/// `PATH` for a known name, falling back to `whisper-cli` (so the error message
/// names the expected binary when none is installed).
pub fn whisper_bin() -> String {
    if let Ok(explicit) = std::env::var("NOVA_WHISPER_BIN") {
        return explicit;
    }
    for cand in WHISPER_CANDIDATES {
        if binary_on_path(cand) {
            return (*cand).to_string();
        }
    }
    "whisper-cli".to_string()
}

/// True if `name` resolves to an executable on `PATH`.
fn binary_on_path(name: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(name).is_file())
}

/// Options for a single transcription run.
#[derive(Debug, Clone, Default)]
pub struct TranscribeOptions {
    /// BCP-47 source language; `None` lets Whisper auto-detect.
    pub language: Option<String>,
    /// Model identifier persisted into the transcript (e.g. `large-v3-turbo-q5_0`).
    pub model_id: String,
    /// whisper.cpp DTW alignment-head preset (e.g. `large.v3.turbo`). Empty
    /// disables DTW; when set, token offsets in the JSON are DTW-aligned.
    pub dtw_preset: String,
    /// Initial prompt fed via `--prompt` — the tail of the previous window so
    /// the model keeps lexical context across the boundary. `None` to omit.
    pub initial_prompt: Option<String>,
    /// Max characters per segment (`--max-len`, split at word boundaries via
    /// `--split-on-word`). `0` = unlimited (whisper's natural segmentation).
    /// A caption-sized value (~60) keeps cues short in dense, pauseless speech
    /// and gives the windowed session finer stitch boundaries.
    pub max_len: u32,
}

/// Transcribe `wav` (16 kHz mono) using the GGML weights at `ggml_model`.
/// whisper-cli writes `<out_base>.json`, which we parse into a [`RawTranscript`].
///
/// `on_progress` receives 0..=100 as whisper-cli reports it on stderr.
pub async fn transcribe_wav(
    wav: &Path,
    ggml_model: &Path,
    out_base: &Path,
    opts: &TranscribeOptions,
    mut on_progress: impl FnMut(u8),
) -> Result<RawTranscript, WhisperError> {
    let mut cmd = Command::new(whisper_bin());
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .arg("-m")
        .arg(ggml_model)
        .arg("-f")
        .arg(wav)
        // Transcription (not translation) is whisper.cpp's default — there is
        // no `--task` flag; passing one makes the CLI print help and exit.
        // Word/token timestamps land in the JSON; required for alignment.
        .arg("--output-json-full")
        .arg("--output-file")
        .arg(out_base)
        // Report progress to stderr so we can surface a percentage.
        .arg("--print-progress");

    // DTW-aligned token timestamps. Without this, token offsets are whisper's
    // coarse heuristic times and word/cue timing drifts.
    if !opts.dtw_preset.is_empty() {
        cmd.args(["--dtw", &opts.dtw_preset]);
    }

    // Lexical context carried from the previous window's tail.
    if let Some(prompt) = opts.initial_prompt.as_deref() {
        if !prompt.trim().is_empty() {
            cmd.args(["--prompt", prompt]);
        }
    }

    // Caption-sized segments split at word boundaries. Keeps cues short in
    // dense speech and gives the windowed session finer stitch boundaries.
    if opts.max_len > 0 {
        cmd.args(["--max-len", &opts.max_len.to_string()]);
        cmd.arg("--split-on-word");
    }

    let language = opts.language.as_deref().unwrap_or("auto");
    cmd.args(["--language", language]);
    if opts.language.as_deref() == Some("ar") {
        // Arabic hallucinates across silence without this; see whisper-config.
        cmd.arg("--no-context");
    }

    let mut child = cmd.spawn().map_err(WhisperError::Spawn)?;

    // Drain stderr line-by-line: parse "progress = NN%", keep the tail for
    // error reporting if whisper-cli exits non-zero.
    let stderr = child.stderr.take().expect("stderr piped");
    let mut lines = BufReader::new(stderr).lines();
    let mut tail: Vec<String> = Vec::new();
    while let Some(line) = lines.next_line().await? {
        if let Some(pct) = parse_progress(&line) {
            on_progress(pct);
        }
        tail.push(line);
        if tail.len() > 20 {
            tail.remove(0);
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        return Err(WhisperError::Failed(status.code(), tail.join("\n")));
    }
    on_progress(100);

    let json_path = out_base.with_extension("json");
    let json = tokio::fs::read_to_string(&json_path)
        .await
        .map_err(|e| WhisperError::Output(json_path.display().to_string(), e))?;
    parse_whisper_json(&json, &opts.model_id, opts.language.as_deref())
}

/// Parse a "progress = NN%" line from whisper-cli's stderr.
fn parse_progress(line: &str) -> Option<u8> {
    let idx = line.find("progress =")?;
    let rest = line[idx + "progress =".len()..].trim_start();
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse::<u8>().ok().map(|p| p.min(100))
}

// ── JSON model (whisper-cli --output-json-full) ───────────────────────────────

#[derive(serde::Deserialize)]
struct WhisperJson {
    #[serde(default)]
    transcription: Vec<RawSeg>,
    #[serde(default)]
    result: ResultMeta,
}

#[derive(serde::Deserialize, Default)]
struct ResultMeta {
    #[serde(default)]
    language: Option<String>,
}

#[derive(serde::Deserialize)]
struct RawSeg {
    offsets: Offsets,
    text: String,
    #[serde(default)]
    tokens: Vec<RawToken>,
}

#[derive(serde::Deserialize)]
struct RawToken {
    text: String,
    offsets: Offsets,
}

/// Millisecond offsets within the media.
#[derive(serde::Deserialize)]
struct Offsets {
    from: i64,
    to: i64,
}

/// Pure parse of whisper-cli's `--output-json-full` document into a
/// [`RawTranscript`]. `lang_override` wins over the auto-detected language.
pub fn parse_whisper_json(
    json: &str,
    model_id: &str,
    lang_override: Option<&str>,
) -> Result<RawTranscript, WhisperError> {
    let doc: WhisperJson = serde_json::from_str(json)?;

    let language = lang_override
        .map(|s| s.to_string())
        .or(doc.result.language)
        .unwrap_or_else(|| "und".to_string());

    let mut segments = Vec::with_capacity(doc.transcription.len());
    let mut duration_end_ms = 0i64;
    for (seq, seg) in doc.transcription.into_iter().enumerate() {
        let text = seg.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        duration_end_ms = duration_end_ms.max(seg.offsets.to);
        let words = tokens_to_words(&seg.tokens);
        segments.push(TranscriptSegment {
            seq: seq as i64,
            start_sec: ms_to_sec(seg.offsets.from),
            end_sec: ms_to_sec(seg.offsets.to),
            text,
            words: if words.is_empty() { None } else { Some(words) },
        });
    }

    // Re-number after dropping blanks so `seq` stays contiguous from 0.
    for (i, seg) in segments.iter_mut().enumerate() {
        seg.seq = i as i64;
    }

    Ok(RawTranscript {
        language,
        model: model_id.to_string(),
        segments,
        duration_processed_sec: Some(ms_to_sec(duration_end_ms)),
    })
}

fn ms_to_sec(ms: i64) -> f64 {
    ms as f64 / 1000.0
}

/// Shift all segment and word timings by `offset_sec`. whisper-cli emits times
/// relative to the start of the WAV it was given; for a windowed transcription
/// the WAV starts at `offset_sec` into the media, so add it back to get
/// absolute media timestamps.
pub fn shift_transcript(t: &mut RawTranscript, offset_sec: f64) {
    for seg in &mut t.segments {
        seg.start_sec += offset_sec;
        seg.end_sec += offset_sec;
        if let Some(words) = &mut seg.words {
            for w in words {
                w.start_sec += offset_sec;
                w.end_sec += offset_sec;
            }
        }
    }
}

/// Group whisper's sub-word tokens into words. Whisper marks a new word with a
/// leading space; continuation pieces (no leading space) are appended to the
/// previous word. Special tokens (`[_BEG_]`, `[_TT_..]`, …) are skipped.
fn tokens_to_words(tokens: &[RawToken]) -> Vec<TranscriptWord> {
    let mut words: Vec<TranscriptWord> = Vec::new();
    for tok in tokens {
        if tok.text.starts_with("[_") {
            continue;
        }
        let trimmed = tok.text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let starts_word = tok.text.starts_with(' ') || words.is_empty();
        if starts_word {
            words.push(TranscriptWord {
                start_sec: ms_to_sec(tok.offsets.from),
                end_sec: ms_to_sec(tok.offsets.to),
                word: trimmed.to_string(),
            });
        } else if let Some(last) = words.last_mut() {
            last.word.push_str(trimmed);
            last.end_sec = ms_to_sec(tok.offsets.to);
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/whisper_output.json");

    #[test]
    fn parses_segments_and_words() {
        let t = parse_whisper_json(FIXTURE, "large-v3-turbo-q5_0", None).unwrap();
        assert_eq!(t.language, "en");
        assert_eq!(t.model, "large-v3-turbo-q5_0");
        assert_eq!(t.segments.len(), 2);

        let first = &t.segments[0];
        assert_eq!(first.seq, 0);
        assert_eq!(first.text, "Hello world.");
        assert!((first.start_sec - 0.0).abs() < 1e-9);
        assert!((first.end_sec - 2.0).abs() < 1e-9);

        // "Hello" + "world." reassembled from sub-word tokens, specials dropped.
        let words = first.words.as_ref().unwrap();
        assert_eq!(
            words.iter().map(|w| w.word.as_str()).collect::<Vec<_>>(),
            vec!["Hello", "world."]
        );
        assert!((words[0].start_sec - 0.0).abs() < 1e-9);
    }

    #[test]
    fn lang_override_wins() {
        let t = parse_whisper_json(FIXTURE, "m", Some("fr")).unwrap();
        assert_eq!(t.language, "fr");
    }

    #[test]
    fn shift_transcript_offsets_all_timings() {
        let mut t = parse_whisper_json(FIXTURE, "m", None).unwrap();
        let before = t.segments[0].start_sec;
        let word_before = t.segments[0].words.as_ref().unwrap()[0].start_sec;
        shift_transcript(&mut t, 30.0);
        assert!((t.segments[0].start_sec - (before + 30.0)).abs() < 1e-9);
        assert!(
            (t.segments[0].words.as_ref().unwrap()[0].start_sec - (word_before + 30.0)).abs()
                < 1e-9
        );
    }

    #[test]
    fn blank_segments_dropped_and_renumbered() {
        let t = parse_whisper_json(FIXTURE, "m", None).unwrap();
        // The fixture has a blank middle segment; seqs must stay 0,1 contiguous.
        let seqs: Vec<i64> = t.segments.iter().map(|s| s.seq).collect();
        assert_eq!(seqs, vec![0, 1]);
    }

    #[test]
    fn progress_line_parsed() {
        assert_eq!(
            parse_progress("whisper_print_progress_callback: progress =  42%"),
            Some(42)
        );
        assert_eq!(parse_progress("no progress here"), None);
    }
}
