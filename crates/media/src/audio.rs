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
pub async fn extract_audio_window(
    input: &str,
    out_wav: &Path,
    start_sec: f64,
    dur_sec: f64,
    deadline: Duration,
) -> Result<(), AudioError> {
    let mut cmd = base_ffmpeg();
    cmd.arg("-ss")
        .arg(format!("{:.3}", start_sec.max(0.0)))
        .arg("-i")
        .arg(input)
        .arg("-t")
        .arg(format!("{:.3}", dur_sec.max(0.0)));
    pcm_output(&mut cmd, out_wav);
    run(cmd, deadline).await
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

/// Append Whisper's required output format (mono 16 kHz signed-16 PCM WAV).
fn pcm_output(cmd: &mut Command, out_wav: &Path) {
    cmd.args([
        "-vn",
        "-ac",
        "1",
        "-ar",
        "16000",
        "-c:a",
        "pcm_s16le",
        "-f",
        "wav",
        // Safe to overwrite: the caller owns this temp path.
        "-y",
    ])
    .arg(out_wav);
}

/// Run `cmd` with a timeout and surface a clean error on non-zero exit.
async fn run(mut cmd: Command, deadline: Duration) -> Result<(), AudioError> {
    let output = match timeout(deadline, cmd.output()).await {
        Ok(result) => result?,
        Err(_) => return Err(AudioError::Timeout(deadline)),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AudioError::Failed(output.status.code(), stderr));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
