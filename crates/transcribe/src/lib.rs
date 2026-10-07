//! On-device speech-to-text for Nova (Phase 6).
//!
//! Batch transcription drives `whisper-cli` (whisper.cpp's CLI, built with
//! Core ML) as a subprocess — the same external-binary pattern used for
//! ffmpeg and mpv. Live transcription keeps whisper.cpp resident in a worker
//! process of Nova's own binary (ADR-0011). This crate owns four concerns:
//!
//! - [`model`] — describe, locate, and first-run-download the GGML weights +
//!   prebuilt Core ML encoder.
//! - [`whisper`] — invoke `whisper-cli` and parse its JSON output into
//!   [`nova_core::RawTranscript`].
//! - [`vtt`] — render segments to WebVTT with the `whisper-config` post-rules.
//! - [`streaming`] — the resident worker and the live streaming policy.
//!
//! Audio extraction (16 kHz mono WAV) lives in `nova_media::extract_audio_16k_mono`;
//! orchestration (resolve URL → extract → ensure model → transcribe → persist)
//! lives in the desktop job handler so this crate stays free of Tauri/storage.

pub mod model;
pub mod streaming;
pub mod vtt;
pub mod whisper;

pub use model::{
    default_model, ensure_model, is_installed, model_paths, ModelDownloadProgress, ModelError,
    ModelPaths, WhisperModel,
};
pub use vtt::segments_to_vtt;
pub use whisper::{
    parse_whisper_json, shift_transcript, transcribe_wav, transcribe_wav_cancellable, whisper_bin,
    TranscribeOptions, WhisperError,
};
