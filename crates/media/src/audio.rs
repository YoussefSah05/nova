//! Audio extraction for transcription.
//!
//! Whisper needs 16 kHz mono PCM. The reference recipe pipes raw PCM to
//! stdout for the FFI path, but Nova drives `whisper-cli` as a subprocess and
//! that wants a real input file, so we extract to a temp WAV (`-f wav`,
//! `pcm_s16le`) instead. The caller owns the lifetime of `out_wav` (typically a
//! file inside a `tempfile::TempDir`) and deletes it once transcription is done.

use std::path::Path;
use std::time::Duration;

use thiserror::Error;
use tokio::process::Command;
use tokio::time::timeout;

use crate::ffmpeg_bin;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("ffmpeg timed out after {0:?}")]
    Timeout(Duration),
    #[error("ffmpeg exited with status {0:?}: {1}")]
    Failed(Option<i32>, String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Extract `input` (a URL or local path) to a 16 kHz mono signed-16-bit WAV at
/// `out_wav`, suitable as `whisper-cli -f` input.
///
/// This reads the whole source to completion, so `deadline` should be generous
/// (a feature-length VOD takes a while to demux); it exists only to bound a
/// hung upstream, not to cap legitimate long extractions.
pub async fn extract_audio_16k_mono(
    input: &str,
    out_wav: &Path,
    deadline: Duration,
) -> Result<(), AudioError> {
    let mut cmd = base_ffmpeg();
    cmd.arg("-i").arg(input);
    pcm_output(&mut cmd, out_wav);
    run(cmd, deadline).await
}

/// Extract just the window `[start_sec, start_sec + dur_sec)` to a 16 kHz mono
/// WAV — the unit of the streaming caption pipeline. `-ss` is placed **before**
/// `-i` for a fast input seek (keyframe-accurate is unnecessary for audio).
/// The caller offsets the resulting transcript timings by `start_sec`.
///
/// `audio_track` selects which audio stream to transcribe (`-map 0:a:<n>`,
/// 0-based among audio streams) for multi-track content; `None` lets ffmpeg pick
/// its default stream. The index is expected to match the player's audio-track
/// order (true for typical MKV/MP4 containers).
pub async fn extract_audio_window(
    input: &str,
    out_wav: &Path,
    start_sec: f64,
    dur_sec: f64,
    audio_track: Option<u32>,
    deadline: Duration,
) -> Result<(), AudioError> {
    let mut cmd = base_ffmpeg();
    cmd.args(window_args(input, out_wav, start_sec, dur_sec, audio_track));
    run(cmd, deadline).await
}

/// Build the ffmpeg arguments (after the common base flags) for a windowed
/// extraction. Pure so the `-ss`/`-t`/`-map` wiring is unit-testable without
/// spawning ffmpeg.
fn window_args(
    input: &str,
    out_wav: &Path,
    start_sec: f64,
    dur_sec: f64,
    audio_track: Option<u32>,
) -> Vec<String> {
    let mut args = vec![
        "-ss".to_string(),
        format!("{:.3}", start_sec.max(0.0)),
        "-i".to_string(),
        input.to_string(),
        "-t".to_string(),
        format!("{:.3}", dur_sec.max(0.0)),
    ];
    if let Some(track) = audio_track {
        args.push("-map".to_string());
        args.push(format!("0:a:{track}"));
    }
    args.extend(PCM_ARGS.iter().map(|s| s.to_string()));
    args.push(out_wav.to_string_lossy().into_owned());
    args
}

/// ffmpeg invocation common to both extractors: quiet, reconnecting (IPTV CDNs
/// flap), with piped stderr for error reporting.
fn base_ffmpeg() -> Command {
    let mut cmd = Command::new(ffmpeg_bin());
    cmd.kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-reconnect",
            "1",
            "-reconnect_streamed",
            "1",
            "-reconnect_delay_max",
            "5",
        ]);
    cmd
}

/// Whisper's required output format (mono 16 kHz signed-16 PCM WAV). `-y` is
/// safe: the caller owns the temp output path.
const PCM_ARGS: &[&str] = &[
    "-vn",
    "-ac",
    "1",
    "-ar",
    "16000",
    "-c:a",
    "pcm_s16le",
    "-f",
    "wav",
    "-y",
];

/// Append Whisper's required output format (mono 16 kHz signed-16 PCM WAV).
fn pcm_output(cmd: &mut Command, out_wav: &Path) {
    cmd.args(PCM_ARGS).arg(out_wav);
}

/// Run `cmd` with a timeout and surface a clean error on non-zero exit.
async fn run(mut cmd: Command, deadline: Duration) -> Result<(), AudioError> {
    let output = match timeout(deadline, cmd.output()).await {
        Ok(result) => result?,
        Err(_) => return Err(AudioError::Timeout(deadline)),
    };
    if !output.status.success() {
        // ffmpeg stderr can contain authenticated URLs (including redirects).
        let stderr = "audio extraction failed".to_string();
        return Err(AudioError::Failed(output.status.code(), stderr));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_args_map_selected_track() {
        let out = Path::new("/tmp/win.wav");
        let args = window_args("http://x/movie.mkv", out, 30.0, 32.0, Some(1));
        // `-map 0:a:1` must appear as an adjacent flag/value pair.
        let pos = args.iter().position(|a| a == "-map").expect("has -map");
        assert_eq!(args[pos + 1], "0:a:1");
        // Seek before input, duration present.
        assert_eq!(args[0], "-ss");
        assert!(args.contains(&"-i".to_string()));
    }

    #[test]
    fn window_args_default_track_has_no_map() {
        let out = Path::new("/tmp/win.wav");
        let args = window_args("in.mkv", out, 0.0, 30.0, None);
        assert!(
            !args.iter().any(|a| a == "-map"),
            "no -map when track is None"
        );
    }

    #[tokio::test]
    async fn missing_input_fails_cleanly() {
        // ffmpeg must be on PATH for this to be meaningful; skip otherwise.
        if Command::new(ffmpeg_bin())
            .arg("-version")
            .output()
            .await
            .is_err()
        {
            eprintln!("ffmpeg not available on PATH; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.wav");
        let err = extract_audio_16k_mono(
            "/nonexistent/nova-test-input.mkv",
            &out,
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AudioError::Failed(..)));
    }
}
