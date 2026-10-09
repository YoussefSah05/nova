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

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use nova_core::{RawTranscript, TranscriptSegment, TranscriptWord};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug, Error)]
pub enum WhisperError {
    #[error("whisper-cli failed to start; repair Nova's runtime with `just install-whisper`: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("whisper-cli exited with status {0:?}: {1}")]
    Failed(Option<i32>, String),
    #[error("whisper output was not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("transcription cancelled")]
    Cancelled,
    #[error("transcription exceeded its time limit")]
    TimedOut,
}

static MANAGED_CLI: OnceLock<PathBuf> = OnceLock::new();

/// Register Nova's private runtime location at startup. Installation may occur
/// later: discovery checks the file on each run, so no path is cached as missing.
pub fn configure_whisper_cli(app_data_dir: &Path) {
    let _ = MANAGED_CLI.set(app_data_dir.join("runtime/whisper/whisper-cli"));
}

/// Prefer an explicit override, then Nova's managed CLI, PATH and Homebrew.
/// Never select a generic `main`: it may be an unrelated executable.
pub fn whisper_bin() -> PathBuf {
    resolve_cli(
        std::env::var_os("NOVA_WHISPER_BIN").as_deref(),
        MANAGED_CLI.get().map(PathBuf::as_path),
        std::env::var_os("PATH").as_deref(),
        homebrew_cli_candidates(),
    )
}

fn homebrew_cli_candidates() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &[
            "/opt/homebrew/bin/whisper-cli",
            "/usr/local/bin/whisper-cli",
        ]
    } else {
        &[]
    }
}

fn executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
}

fn absolute(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|dir| dir.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

fn resolve_cli(
    explicit: Option<&OsStr>,
    managed: Option<&Path>,
    paths: Option<&OsStr>,
    known: &[&str],
) -> PathBuf {
    let on_path = |name: &OsStr| {
        paths.and_then(|paths| {
            std::env::split_paths(paths)
                .map(|dir| dir.join(name))
                .find(|path| executable(path))
        })
    };
    if let Some(explicit) = explicit.filter(|value| !value.is_empty()) {
        let path = Path::new(explicit);
        return if path.components().count() == 1 && !path.is_absolute() {
            on_path(explicit)
                .map(|path| absolute(&path))
                .unwrap_or_else(|| absolute(path))
        } else {
            absolute(path)
        };
    }
    if let Some(path) = managed.filter(|path| executable(path)) {
        return absolute(path);
    }
    if let Some(path) = on_path(OsStr::new("whisper-cli")) {
        return absolute(&path);
    }
    if let Some(path) = known.iter().map(Path::new).find(|path| executable(path)) {
        return absolute(path);
    }
    PathBuf::from("whisper-cli")
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
    on_progress: impl FnMut(u8),
) -> Result<RawTranscript, WhisperError> {
    transcribe_wav_cancellable(
        wav,
        ggml_model,
        out_base,
        opts,
        on_progress,
        std::future::pending(),
    )
    .await
}

/// Cancellation kills and reaps whisper-cli before returning to the lease
/// owner. The five-minute bound applies to one existing ~30-second ASR window.
pub async fn transcribe_wav_cancellable(
    wav: &Path,
    ggml_model: &Path,
    out_base: &Path,
    opts: &TranscribeOptions,
    mut on_progress: impl FnMut(u8),
    cancelled: impl std::future::Future<Output = ()>,
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

    // Normalize the language hint to a code whisper.cpp accepts. The player
    // reports ISO-639-2 ("eng"), which whisper-cli rejects — it then prints help
    // and writes no JSON (the "could not read whisper output" failure). Unknown
    // codes fall back to auto-detect rather than breaking the run.
    let normalized = opts.language.as_deref().and_then(normalize_whisper_lang);
    cmd.args(["--language", normalized.as_deref().unwrap_or("auto")]);
    if normalized.as_deref() == Some("ar") {
        // Arabic hallucinates across silence without this; see whisper-config.
        cmd.arg("--no-context");
    }

    let mut child = cmd.spawn().map_err(WhisperError::Spawn)?;

    let (status, tail) = wait_for_child(
        &mut child,
        &mut on_progress,
        cancelled,
        Duration::from_secs(300),
    )
    .await?;
    if !status.success() {
        return Err(WhisperError::Failed(status.code(), tail.join("\n")));
    }
    on_progress(100);

    let json_path = out_base.with_extension("json");
    let json = match tokio::fs::read_to_string(&json_path).await {
        Ok(j) => j,
        // whisper-cli exited 0 but wrote no JSON — almost always an argument it
        // didn't recognise, in which case it dumps help to stderr. Surface that
        // tail instead of a bare "No such file" so the cause is visible.
        Err(_) => {
            return Err(WhisperError::Failed(
                status.code(),
                format!(
                    "whisper produced no JSON output. Last output:\n{}",
                    tail.join("\n")
                ),
            ))
        }
    };
    parse_whisper_json(&json, &opts.model_id, normalized.as_deref())
}

async fn wait_for_child(
    child: &mut Child,
    on_progress: &mut impl FnMut(u8),
    cancelled: impl std::future::Future<Output = ()>,
    deadline: Duration,
) -> Result<(std::process::ExitStatus, Vec<String>), WhisperError> {
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("whisper stderr is not piped"))?;
    let result = tokio::select! {
        biased;
        _ = cancelled => Err(WhisperError::Cancelled),
        _ = tokio::time::sleep(deadline) => Err(WhisperError::TimedOut),
        result = async {
            let mut lines = BufReader::new(stderr).lines();
            let mut tail = Vec::new();
            while let Some(line) = lines.next_line().await? {
                if let Some(pct) = parse_progress(&line) { on_progress(pct); }
                tail.push(line);
                if tail.len() > 20 { tail.remove(0); }
            }
            Ok::<_, WhisperError>((child.wait().await?, tail))
        } => result,
    };
    if result.is_err() {
        // start_kill is harmless if the child has already exited. Always wait
        // so the caller may release model files/grants only after child exit.
        let _ = child.start_kill();
        child.wait().await?;
    }
    result
}

/// Normalize a language hint into a code whisper.cpp accepts (ISO-639-1, two
/// letters) or `None` to auto-detect. The media player reports ISO-639-2 track
/// tags ("eng", "fra", …) which whisper-cli rejects — passing one makes it print
/// help and write no JSON. Anything unrecognised maps to `None` (auto-detect) so
/// a stray tag can never break a run.
pub fn normalize_whisper_lang(code: &str) -> Option<String> {
    let c = code.trim().to_ascii_lowercase();
    if c.is_empty() || c == "und" || c == "auto" || c == "mul" || c == "zxx" {
        return None;
    }
    // Already ISO-639-1 (whisper's native code form).
    if c.len() == 2 && c.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return Some(c);
    }
    // Common ISO-639-2 (bibliographic /B and terminological /T) → 639-1.
    let mapped = match c.as_str() {
        "eng" => "en",
        "fre" | "fra" => "fr",
        "ger" | "deu" => "de",
        "spa" => "es",
        "ita" => "it",
        "por" => "pt",
        "rus" => "ru",
        "jpn" => "ja",
        "kor" => "ko",
        "chi" | "zho" => "zh",
        "ara" => "ar",
        "hin" => "hi",
        "nld" | "dut" => "nl",
        "swe" => "sv",
        "nor" => "no",
        "dan" => "da",
        "fin" => "fi",
        "pol" => "pl",
        "tur" => "tr",
        "heb" => "he",
        "ell" | "gre" => "el",
        "ces" | "cze" => "cs",
        "ron" | "rum" => "ro",
        "ukr" => "uk",
        "vie" => "vi",
        "tha" => "th",
        "ind" => "id",
        "msa" | "may" => "ms",
        "hun" => "hu",
        "bul" => "bg",
        "hrv" => "hr",
        "srp" => "sr",
        "slk" | "slo" => "sk",
        "slv" => "sl",
        "lit" => "lt",
        "lav" => "lv",
        "est" => "et",
        "fas" | "per" => "fa",
        "urd" => "ur",
        "ben" => "bn",
        "tam" => "ta",
        "tel" => "te",
        "cat" => "ca",
        "isl" | "ice" => "is",
        _ => return None,
    };
    Some(mapped.to_string())
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

    #[cfg(unix)]
    fn cli_fixture(dir: &Path, name: &str, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path.canonicalize().unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn managed_cli_works_without_gui_path_and_wins_over_old_path_copies() {
        let managed = tempfile::tempdir().unwrap();
        let path_dir = tempfile::tempdir().unwrap();
        let cli = cli_fixture(managed.path(), "whisper-cli", 0o755);
        cli_fixture(path_dir.path(), "whisper-cli", 0o755);
        let paths = std::env::join_paths([path_dir.path()]).unwrap();
        assert_eq!(resolve_cli(None, Some(&cli), None, &[]), cli);
        assert_eq!(resolve_cli(None, Some(&cli), Some(&paths), &[]), cli);
        let explicit = cli_fixture(path_dir.path(), "custom-whisper", 0o755);
        assert_eq!(
            resolve_cli(Some(explicit.as_os_str()), Some(&cli), None, &[]),
            explicit
        );
    }

    #[cfg(unix)]
    #[test]
    fn discovery_checks_execution_permissions_and_never_launches_generic_main() {
        let dir = tempfile::tempdir().unwrap();
        cli_fixture(dir.path(), "main", 0o755);
        cli_fixture(dir.path(), "whisper-cpp", 0o755);
        let cli = cli_fixture(dir.path(), "whisper-cli", 0o644);
        let paths = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(
            resolve_cli(None, Some(&cli), Some(&paths), &[]),
            PathBuf::from("whisper-cli")
        );
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(resolve_cli(None, None, Some(&paths), &[]), cli);
        assert_eq!(resolve_cli(None, None, None, &[cli.to_str().unwrap()]), cli);
        assert_eq!(
            resolve_cli(Some(OsStr::new("whisper-cli")), None, Some(&paths), &[]),
            cli
        );
    }

    /// Deliberate device qualification, never a model download in CI.
    #[tokio::test]
    #[ignore = "requires NOVA_TEST_WHISPER_ROOT and NOVA_TEST_WHISPER_WAV (public JFK sample)"]
    async fn installed_cli_transcribes_real_speech_with_the_player_driver() {
        let root =
            PathBuf::from(std::env::var_os("NOVA_TEST_WHISPER_ROOT").expect("app data root"));
        let wav = PathBuf::from(std::env::var_os("NOVA_TEST_WHISPER_WAV").expect("public sample"));
        configure_whisper_cli(&root);
        assert_eq!(
            whisper_bin(),
            root.join("runtime/whisper/whisper-cli")
                .canonicalize()
                .unwrap()
        );
        let model = crate::default_model();
        let paths = crate::model_paths(&root.join("models/whisper"), &model);
        let out = tempfile::tempdir().unwrap();
        let opts = TranscribeOptions {
            model_id: model.id.into(),
            dtw_preset: model.dtw_preset.into(),
            language: Some("eng".into()),
            max_len: 60,
            initial_prompt: None,
        };
        let started = std::time::Instant::now();
        let transcript =
            transcribe_wav(&wav, &paths.ggml, &out.path().join("speech"), &opts, |_| {})
                .await
                .unwrap();
        let text = transcript
            .segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        assert!(
            text.contains("country"),
            "public reference speech was not recognized"
        );
        assert_eq!(transcript.language, "en");
        let words = transcript
            .segments
            .iter()
            .filter_map(|s| s.words.as_ref())
            .flatten()
            .collect::<Vec<_>>();
        assert!(words.len() > 10);
        for word in &words {
            assert!(word.start_sec.is_finite() && word.end_sec.is_finite());
            assert!(word.start_sec >= 0.0 && word.end_sec >= word.start_sec);
            assert!(word.end_sec <= 12.0);
        }
        eprintln!(
            "Whisper real-model smoke: {:.2}s to caption data, {} segments, {} words",
            started.elapsed().as_secs_f64(),
            transcript.segments.len(),
            words.len()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_reaps_child_before_returning_to_model_lease_owner() {
        let mut child = Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            wait_for_child(
                &mut child,
                &mut |_| {},
                std::future::ready(()),
                Duration::from_secs(60),
            ),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(WhisperError::Cancelled)));
        assert!(child.try_wait().unwrap().is_some());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stalled_child_is_killed_and_reaped_at_deadline() {
        let mut child = Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            wait_for_child(
                &mut child,
                &mut |_| {},
                std::future::pending(),
                Duration::ZERO,
            ),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(WhisperError::TimedOut)));
        assert!(child.try_wait().unwrap().is_some());
    }

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
    fn normalizes_language_codes() {
        // ISO-639-2 (what the player reports) → ISO-639-1.
        assert_eq!(normalize_whisper_lang("eng").as_deref(), Some("en"));
        assert_eq!(normalize_whisper_lang("fra").as_deref(), Some("fr"));
        assert_eq!(normalize_whisper_lang("FRE").as_deref(), Some("fr"));
        assert_eq!(normalize_whisper_lang("ara").as_deref(), Some("ar"));
        // Already 639-1 → unchanged.
        assert_eq!(normalize_whisper_lang("en").as_deref(), Some("en"));
        // Unknown / sentinel → auto-detect.
        assert_eq!(normalize_whisper_lang("und"), None);
        assert_eq!(normalize_whisper_lang(""), None);
        assert_eq!(normalize_whisper_lang("qqq"), None);
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
