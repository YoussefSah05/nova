//! FFmpeg subprocess wrapper.
//!
//! Implemented in Phase 6: audio extraction to 16kHz mono PCM for Whisper,
//! thumbnail sprite generation for VOD scrubbing, codec/container probing.
//! FFmpeg binary is bundled at install time via scripts/bundle-ffmpeg.sh.
//! See docs/reference/ffmpeg-recipes.md for canonical FFmpeg invocations.

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
