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
use tokio::io::{AsyncReadExt, AsyncSeekExt};
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
    #[error("decoded audio is not a valid 16 kHz mono PCM WAV")]
    InvalidAudio,
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
    let mut cmd = base_ffmpeg(input);
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
) -> Result<f64, AudioError> {
    let mut cmd = base_ffmpeg(input);
    cmd.args(window_args(input, out_wav, start_sec, dur_sec, audio_track));
    run(cmd, deadline).await?;
    decoded_wav_duration(out_wav).await
}

/// Inspect actual decoded samples, rather than assuming a successful FFmpeg
/// exit means audio exists. A valid empty data chunk represents EOF; malformed
/// or truncated output is an extraction failure. Chunk parsing handles FFmpeg's
/// optional LIST metadata without reading the PCM body into memory.
pub async fn decoded_wav_duration(path: &Path) -> Result<f64, AudioError> {
    let mut file = tokio::fs::File::open(path).await?;
    let length = file.metadata().await?.len();
    let mut header = [0u8; 12];
    file.read_exact(&mut header).await?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return Err(AudioError::InvalidAudio);
    }
    let mut position = 12u64;
    let mut valid_format = false;
    while position + 8 <= length {
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk).await?;
        position += 8;
        let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as u64;
        if size > length - position {
            return Err(AudioError::InvalidAudio);
        }
        if &chunk[..4] == b"fmt " {
            if size < 16 {
                return Err(AudioError::InvalidAudio);
            }
            let mut format = [0u8; 16];
            file.read_exact(&mut format).await?;
            valid_format = u16::from_le_bytes([format[0], format[1]]) == 1
                && u16::from_le_bytes([format[2], format[3]]) == 1
                && u32::from_le_bytes([format[4], format[5], format[6], format[7]]) == 16_000
                && u16::from_le_bytes([format[12], format[13]]) == 2
                && u16::from_le_bytes([format[14], format[15]]) == 16;
        } else if &chunk[..4] == b"data" {
            if !valid_format || !size.is_multiple_of(2) {
                return Err(AudioError::InvalidAudio);
            }
            return Ok(size as f64 / 32_000.0);
        }
        position += size + size % 2;
        file.seek(std::io::SeekFrom::Start(position)).await?;
    }
    Err(AudioError::InvalidAudio)
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
fn base_ffmpeg(input: &str) -> Command {
    let mut cmd = Command::new(ffmpeg_bin());
    cmd.kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .args(["-hide_banner", "-loglevel", "error"]);
    // HTTP protocol options are rejected for local files. Local fixtures and
    // downloaded movies must use the same extraction path without these flags.
    if input.starts_with("https://") || input.starts_with("http://") {
        cmd.args([
            "-reconnect",
            "1",
            "-reconnect_streamed",
            "1",
            "-reconnect_delay_max",
            "5",
        ]);
    }
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

    fn wav_bytes(samples: &[i16]) -> Vec<u8> {
        let data_size = (samples.len() * 2) as u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&32_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    #[tokio::test]
    async fn decoded_samples_distinguish_silence_eof_and_truncated_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.wav");
        tokio::fs::write(&path, wav_bytes(&[0; 16_000]))
            .await
            .unwrap();
        assert_eq!(
            decoded_wav_duration(&path).await.unwrap(),
            1.0,
            "silence has real samples"
        );
        tokio::fs::write(&path, wav_bytes(&[])).await.unwrap();
        assert_eq!(
            decoded_wav_duration(&path).await.unwrap(),
            0.0,
            "header-only output is EOF"
        );
        let mut truncated = wav_bytes(&[1; 100]);
        truncated.truncate(truncated.len() - 2);
        tokio::fs::write(&path, truncated).await.unwrap();
        assert!(matches!(
            decoded_wav_duration(&path).await,
            Err(AudioError::InvalidAudio)
        ));
    }

    #[tokio::test]
    #[ignore = "requires local FFmpeg; creates a sparse two-hour audio fixture"]
    async fn extracts_late_windows_and_empty_eof_from_two_hour_media() {
        use std::io::{Seek, Write};
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("two-hours.wav");
        let samples = 7_200u32 * 16_000;
        let data_size = samples * 2;
        let mut header = wav_bytes(&[]);
        header[4..8].copy_from_slice(&(36 + data_size).to_le_bytes());
        header[40..44].copy_from_slice(&data_size.to_le_bytes());
        let mut file = std::fs::File::create(&input).unwrap();
        file.write_all(&header).unwrap();
        file.set_len(44 + u64::from(data_size)).unwrap();
        file.seek(std::io::SeekFrom::Start(44 + 7_190 * 32_000))
            .unwrap();
        // Deterministic signal near the end, with silence throughout the first
        // 119 minutes. Sparse holes avoid writing an entire movie-sized fixture.
        for sample in 0..16_000 {
            let value = if sample % 40 < 20 {
                2_000i16
            } else {
                -2_000i16
            };
            file.write_all(&value.to_le_bytes()).unwrap();
        }
        drop(file);
        let output = dir.path().join("late.wav");
        let duration = extract_audio_window(
            input.to_str().unwrap(),
            &output,
            7_185.0,
            30.0,
            Some(0),
            Duration::from_secs(20),
        )
        .await
        .unwrap();
        assert!((duration - 15.0).abs() < 0.01);
        let bytes = tokio::fs::read(&output).await.unwrap();
        assert!(bytes.len() < 500_000, "only a bounded late window decoded");
        assert!(bytes.windows(2).any(|pair| pair == 2_000i16.to_le_bytes()));
        let duration = extract_audio_window(
            input.to_str().unwrap(),
            &output,
            7_210.0,
            30.0,
            Some(0),
            Duration::from_secs(20),
        )
        .await
        .unwrap();
        assert_eq!(duration, 0.0);
    }

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
