//! Live speech recognition: a resident whisper.cpp worker process and the
//! LocalAgreement-2 streaming policy chosen by the AI22 benchmark.
//!
//! ```text
//! audio tap ──PcmQueue──▶ StreamingSession ──decode──▶ WhisperWorker ══pipes══▶ ASR worker process
//!                              │                                               (whisper.cpp, Metal → Core ML)
//!                              ▼
//!                  AsrUpdate::{Provisional, Committed}
//! ```
//!
//! See ADR-0011 for why the model runs in a separate process.

pub mod client;
pub mod policy;
pub mod protocol;
pub mod session;
pub mod text;
pub mod worker;

use std::sync::Arc;

use nova_core::intelligence::{
    AsrSession, AsrSessionConfig, EngineError, PcmQueue, ProviderKind, SpeechToTextEngine,
};

pub use client::{BufferDecoder, WhisperWorker, WorkerError, WorkerSpec};
pub use protocol::{Accel, DecodeRequest, Decoded, TimingModels, TimingReport};
pub use session::StreamingSession;
pub use worker::segments_of;

/// The local whisper.cpp speech engine: one resident worker, any number of
/// sessions (decodes are serialized per model).
pub struct WhisperStreamingEngine {
    worker: Arc<WhisperWorker>,
}

impl WhisperStreamingEngine {
    /// Start the worker and wait until it can decode.
    pub fn start(spec: WorkerSpec) -> Result<Self, WorkerError> {
        Ok(Self {
            worker: Arc::new(WhisperWorker::start(spec)?),
        })
    }

    /// The encoder the latest decode ran on (Metal until Core ML is ready).
    pub fn accel(&self) -> Option<Accel> {
        self.worker.accel()
    }

    /// One decode on the resident worker (blocking). A worker that dies or
    /// hangs is replaced on the next call; this call fails.
    pub fn decode(
        &self,
        request: &protocol::DecodeRequest,
        pcm: &[f32],
    ) -> Result<protocol::Decoded, EngineError> {
        self.worker.decode(request, pcm)
    }

    /// Stop the worker process for good (model unloaded or evicted).
    /// Sessions still holding the engine then fail their steps.
    pub fn shutdown(&self) {
        self.worker.shutdown();
    }
}

impl SpeechToTextEngine for WhisperStreamingEngine {
    fn provider(&self) -> ProviderKind {
        ProviderKind::Local
    }

    fn open_session(
        &self,
        config: AsrSessionConfig,
        audio: PcmQueue,
    ) -> Result<Box<dyn AsrSession>, EngineError> {
        let decoder: Arc<dyn BufferDecoder> = self.worker.clone();
        Ok(Box::new(StreamingSession::new(decoder, config, audio)?))
    }
}
