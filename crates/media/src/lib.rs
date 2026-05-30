//! FFmpeg subprocess wrapper.
//!
//! Phase 5 surface:
//! - [`probe::probe_stream`] — short ffprobe call that returns a
//!   [`StreamProbe`] used by the HLS proxy to decide passthrough vs remux.
//! - [`remux::RemuxSession`] — long-lived ffmpeg subprocess that turns
//!   arbitrary upstream content into fMP4-HLS in a temp dir. The proxy
//!   serves the resulting manifest/segments verbatim.
//!
//! Future (Phase 6+): audio extraction to 16 kHz mono PCM for Whisper, VOD
//! thumbnail sprite generation. The binary is bundled at install time via
//! `scripts/bundle-ffmpeg.sh`; for now we resolve `ffmpeg`/`ffprobe` from
//! `PATH` (overrideable via `NOVA_FFMPEG_BIN` / `NOVA_FFPROBE_BIN`).
//!
//! See `docs/reference/ffmpeg.md` for canonical FFmpeg invocations.

pub mod probe;
pub mod remux;

pub use probe::{
    probe_stream, url_looks_like_browser_file, url_looks_like_hls_playlist,
    url_looks_like_transport_stream, PlaybackRoute, ProbeError, StreamProbe, StreamProfile,
};
pub use remux::{RemuxError, RemuxSession};

/// Resolve the ffmpeg binary path. Honours `NOVA_FFMPEG_BIN` for dev/test
/// overrides; falls back to the literal name so the OS PATH is searched.
pub fn ffmpeg_bin() -> String {
    std::env::var("NOVA_FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".into())
}

/// Resolve the ffprobe binary path. Honours `NOVA_FFPROBE_BIN`.
pub fn ffprobe_bin() -> String {
    std::env::var("NOVA_FFPROBE_BIN").unwrap_or_else(|_| "ffprobe".into())
}
