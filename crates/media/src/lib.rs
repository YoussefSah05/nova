//! FFmpeg subprocess wrapper.
//!
//! Phase 5 surface:
//! - [`probe::probe_stream`] — short ffprobe call that returns a
//!   [`StreamProbe`] used by the HLS proxy to decide passthrough vs remux.
//! - [`remux::RemuxSession`] — long-lived ffmpeg subprocess that turns
//!   arbitrary upstream content into fMP4-HLS in a temp dir. The proxy
//!   serves the resulting manifest/segments verbatim.
//!
//! Phase 6 adds [`audio::extract_audio_16k_mono`] — 16 kHz mono WAV extraction
//! for Whisper. Future: VOD thumbnail sprite generation. The binary is bundled
//! at install time via
//! `scripts/bundle-ffmpeg.sh`; for now we resolve `ffmpeg`/`ffprobe` from
//! `PATH` or standard macOS Homebrew locations (overrideable via
//! `NOVA_FFMPEG_BIN` / `NOVA_FFPROBE_BIN`).
//!
//! See `docs/reference/ffmpeg.md` for canonical FFmpeg invocations.

pub mod audio;
pub mod probe;
pub mod remux;

pub use audio::{extract_audio_16k_mono, extract_audio_window, read_wav_16k_mono, AudioError};
pub use probe::{
    probe_stream, url_looks_like_browser_file, url_looks_like_hls_playlist,
    url_looks_like_transport_stream, PlaybackRoute, ProbeError, StreamProbe, StreamProfile,
};
pub use remux::{RemuxError, RemuxSession};

/// Resolve ffmpeg without requiring a GUI launch to inherit the shell PATH.
pub fn ffmpeg_bin() -> String {
    resolve_tool("ffmpeg", "NOVA_FFMPEG_BIN")
}

/// Resolve the ffprobe binary path. Honours `NOVA_FFPROBE_BIN`.
pub fn ffprobe_bin() -> String {
    resolve_tool("ffprobe", "NOVA_FFPROBE_BIN")
}

fn resolve_tool(name: &str, override_key: &str) -> String {
    if let Ok(explicit) = std::env::var(override_key) {
        return explicit; // A broken explicit override must remain diagnosable.
    }
    let directories = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    let directories = directories
        .into_iter()
        .chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into()])
        .collect();
    find_tool(name, directories).unwrap_or_else(|| name.to_string())
}

fn find_tool(name: &str, directories: Vec<std::path::PathBuf>) -> Option<String> {
    directories
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|path| {
            let Ok(metadata) = path.metadata() else {
                return false;
            };
            if !metadata.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(all(test, unix))]
mod tool_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn discovery_skips_non_executable_files_and_respects_directory_order() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let fallback = root.path().join("fallback");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&fallback).unwrap();
        let file = first.join("ffprobe");
        let fallback_file = fallback.join("ffprobe");
        for path in [&file, &fallback_file] {
            std::fs::write(path, "#!/bin/sh\n").unwrap();
        }
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&fallback_file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            find_tool("ffprobe", vec![first.clone(), fallback.clone()]),
            Some(fallback_file.to_string_lossy().into_owned())
        );
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            find_tool("ffprobe", vec![first, fallback]),
            Some(file.to_string_lossy().into_owned())
        );
    }
}
