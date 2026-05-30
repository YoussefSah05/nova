//! Long-lived ffmpeg subprocess that turns arbitrary upstream content into
//! fMP4-HLS on disk. The HLS proxy serves the resulting manifest and segments
//! verbatim - this module owns nothing HTTP-shaped.
//!
//! Lifecycle: a [`RemuxSession`] is created per `(media_kind, media_id)`, kept
//! alive while the user is watching, and dropped (which kills the child and
//! removes the temp dir) when the player closes or the app exits.
//!
//! Phase 5 picks a conservative recipe - copy video, transcode audio to AAC,
//! fMP4 segments with `independent_segments+delete_segments` so seek/scrub
//! works and disk usage stays bounded. The recipe can be specialised per
//! [`crate::probe::PlaybackRoute`] when we need fuller transcoding.

use std::path::PathBuf;
use std::time::Duration;

use tempfile::TempDir;
use thiserror::Error;
use tokio::fs;
use tokio::process::{Child, Command};
use tokio::time::Instant;

use crate::ffmpeg_bin;
use crate::probe::PlaybackRoute;

#[derive(Debug, Error)]
pub enum RemuxError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffmpeg failed to produce master.m3u8 within {0:?}")]
    StartupTimeout(Duration),
    #[error("ffmpeg exited before becoming ready: status {0:?}")]
    EarlyExit(Option<i32>),
}

/// A running ffmpeg remux session.
///
/// The child is `kill_on_drop(true)`, so when this value is dropped the
/// process is reaped and the temp dir is removed. The HLS proxy holds these
/// in a `DashMap` keyed by `(media_kind, media_id)` for the player's lifetime.
pub struct RemuxSession {
    child: Child,
    dir: TempDir,
}

impl RemuxSession {
    /// Start an ffmpeg remux for `upstream_url` using a recipe matched to
    /// `class`. Returns once `master.m3u8` exists on disk (i.e. the player
    /// can fetch the manifest immediately).
    pub async fn start(
        upstream_url: &str,
        route: PlaybackRoute,
        startup_timeout: Duration,
    ) -> Result<Self, RemuxError> {
        let dir = tempfile::Builder::new().prefix("nova-remux-").tempdir()?;
        let dir_path = dir.path().to_path_buf();
        let master_path = dir_path.join("master.m3u8");

        let mut cmd = Command::new(ffmpeg_bin());
        cmd.kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .args([
                "-hide_banner",
                "-loglevel",
                "warning",
                "-analyzeduration",
                "1000000",
                "-probesize",
                "1000000",
                "-fflags",
                "nobuffer",
                "-flags",
                "low_delay",
                // Reconnect aggressively - IPTV CDNs flap.
                "-reconnect",
                "1",
                "-reconnect_streamed",
                "1",
                "-reconnect_delay_max",
                "5",
                "-i",
                upstream_url,
            ]);

        // Video: copy if possible, transcode only when the codec is exotic.
        match route {
            PlaybackRoute::TranscodeHls => {
                cmd.args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-crf",
                    "23",
                    "-pix_fmt",
                    "yuv420p",
                ]);
            }
            _ => {
                cmd.args(["-c:v", "copy"]);
            }
        }

        // Audio: always transcode to stereo AAC. Cheap on Apple Silicon and
        // sidesteps WebKit's missing AC-3/EAC-3/DTS support.
        cmd.args([
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ac",
            "2",
            "-f",
            "hls",
            "-hls_time",
            "1",
            "-hls_list_size",
            "0",
            "-hls_segment_type",
            "fmp4",
            "-hls_flags",
            "independent_segments+delete_segments+temp_file",
            "-hls_base_url",
            "local/",
            "-hls_segment_filename",
        ])
        .arg(dir_path.join("seg_%05d.m4s"))
        .arg(&master_path);

        let child = cmd.spawn()?;
        let mut session = Self { child, dir };
        session.await_ready(&master_path, startup_timeout).await?;
        Ok(session)
    }

    /// Block until `master.m3u8` exists (i.e. ffmpeg wrote the manifest
    /// header). Polls with a short backoff; the first segment is the
    /// long pole on remote sources. If ffmpeg dies before the manifest
    /// appears we detect it through `try_wait` and surface [`RemuxError::EarlyExit`].
    async fn await_ready(
        &mut self,
        master_path: &PathBuf,
        deadline: Duration,
    ) -> Result<(), RemuxError> {
        let started = Instant::now();
        loop {
            if fs::metadata(master_path).await.is_ok() {
                return Ok(());
            }
            if let Some(status) = self.child.try_wait()? {
                return Err(RemuxError::EarlyExit(status.code()));
            }
            if started.elapsed() >= deadline {
                return Err(RemuxError::StartupTimeout(deadline));
            }
            tokio::time::sleep(Duration::from_millis(80)).await;
        }
    }

    /// Path to the directory ffmpeg writes into. The proxy serves files
    /// from this directory directly.
    pub fn dir(&self) -> &std::path::Path {
        self.dir.path()
    }
}

impl Drop for RemuxSession {
    fn drop(&mut self) {
        // `kill_on_drop` on the Command builder ensures the child is killed
        // when `self.child` drops. The TempDir cleans the directory.
        tracing::debug!(dir = ?self.dir.path(), "remux session dropped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Smoke test that ffmpeg is at least on PATH; the full lifecycle is
    // exercised by the hls-proxy integration tests.
    #[tokio::test]
    async fn ffmpeg_binary_resolves() {
        let out = Command::new(ffmpeg_bin()).arg("-version").output().await;
        // If ffmpeg isn't installed on the test machine, skip - CI installs it.
        if out.is_err() {
            eprintln!("ffmpeg not available on PATH; skipping");
            return;
        }
        assert!(out.unwrap().status.success());
    }
}
