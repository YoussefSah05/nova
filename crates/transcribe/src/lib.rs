//! whisper.cpp FFI wrapper + streaming ASR.
//!
//! Implemented in Phase 6: batch VOD transcription with word-level timestamps,
//! live streaming ASR with LocalAgreement-2 stabilization.
//! Builds whisper.cpp with WHISPER_COREML=1 for Apple Neural Engine acceleration.
//! See docs/reference/whisper-config.md for model selection and VAD tuning.

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
