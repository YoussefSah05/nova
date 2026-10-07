//! Engine contracts for on-device intelligence, independent of any backend.
//!
//! Pipelines (live captions, translation) talk to these traits, never to a
//! concrete engine, so a model can be swapped by editing the manifest and a
//! cloud provider can later slot in behind the same contract. Every call is
//! blocking by design: inference is CPU/GPU/ANE work that belongs on a
//! dedicated worker thread, never on the async executor.
//!
//! Cloud providers are a seam only. This build ships no cloud client: the
//! cloud engines below refuse every request with `ProviderNotConfigured`
//! and send nothing anywhere.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use thiserror::Error;

use crate::model_manifest::Capability;

/// Where an engine runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    /// On this Mac, from an internal or external model library.
    Local,
    /// A remote service. Reserved: no cloud provider exists in this build.
    Cloud,
}

/// The providers a capability may use, most preferred first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRoute {
    pub capability: Capability,
    pub preferred: Vec<ProviderKind>,
}

impl CapabilityRoute {
    /// The only route this build offers.
    pub fn local_only(capability: Capability) -> Self {
        Self {
            capability,
            preferred: vec![ProviderKind::Local],
        }
    }

    /// The first preferred provider that is ready, if any.
    pub fn pick(&self, ready: impl Fn(ProviderKind) -> bool) -> Option<ProviderKind> {
        self.preferred.iter().copied().find(|&kind| ready(kind))
    }
}

/// Settings a future cloud provider will need. The credential is a reference
/// into the system keychain, never the secret itself, so it can be persisted
/// and displayed safely.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CloudProviderConfig {
    pub endpoint: Option<String>,
    pub credential_ref: Option<String>,
    /// Off by default; turning it on has no effect until a client exists.
    pub enabled: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngineError {
    #[error("this provider is not configured")]
    ProviderNotConfigured,
    #[error("the engine does not support this language or language pair")]
    UnsupportedLanguage,
    #[error("the audio format is not supported")]
    UnsupportedAudio,
    #[error("the engine failed to load its model")]
    LoadFailed,
    #[error("the engine failed during inference")]
    InferenceFailed,
}

/// Audio every speech engine accepts: mono f32 PCM at this rate.
pub const ASR_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrSessionConfig {
    /// ISO 639-1 code, or None to let the engine detect the language.
    pub language: Option<String>,
    pub sample_rate: u32,
    /// How much new audio the engine waits for between decodes. Smaller is
    /// faster to first text and costs more compute.
    pub step_ms: u32,
}

/// A span of recognised speech. Times are milliseconds from the start of
/// the session's audio, so a caller can map them onto media time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsrUpdate {
    /// The current best guess for the uncommitted tail. It may be rewritten,
    /// so it is shown dimmed and never persisted or translated.
    Provisional(AsrSegment),
    /// Final text. Committed segments never change once emitted.
    Committed(Vec<AsrSegment>),
}

/// The hand-off between the audio tap and a speech session. The tap pushes
/// from its own thread; the session drains on the inference worker. The lock
/// is held only to copy samples, never during inference, so pushing audio
/// never waits for a decode. Bounded: when the worker falls behind, the
/// oldest audio is dropped and counted (live captions skip ahead rather
/// than drift further behind).
#[derive(Debug, Clone)]
pub struct PcmQueue {
    inner: Arc<Mutex<PcmQueueInner>>,
}

#[derive(Debug)]
struct PcmQueueInner {
    samples: VecDeque<f32>,
    capacity: usize,
    dropped: u64,
}

impl PcmQueue {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(PcmQueueInner {
                samples: VecDeque::with_capacity(capacity),
                capacity: capacity.max(1),
                dropped: 0,
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PcmQueueInner> {
        // A panic while copying samples leaves the queue structurally valid.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn push(&self, samples: &[f32]) {
        let mut q = self.lock();
        let capacity = q.capacity;
        let keep = samples.len().min(capacity);
        let overflow = (q.samples.len() + keep).saturating_sub(capacity);
        q.samples.drain(..overflow);
        q.dropped += (overflow + samples.len() - keep) as u64;
        q.samples.extend(&samples[samples.len() - keep..]);
    }

    /// Move everything queued into `out`.
    pub fn drain_into(&self, out: &mut Vec<f32>) {
        out.extend(self.lock().samples.drain(..));
    }

    pub fn clear(&self) {
        self.lock().samples.clear();
    }

    /// Samples dropped because the consumer fell behind, since creation.
    pub fn dropped(&self) -> u64 {
        self.lock().dropped
    }
}

pub trait SpeechToTextEngine: Send + Sync {
    fn provider(&self) -> ProviderKind;
    /// Open a streaming session reading from `audio`. The model stays
    /// resident across sessions, so opening one is cheap compared with
    /// loading the engine.
    fn open_session(
        &self,
        config: AsrSessionConfig,
        audio: PcmQueue,
    ) -> Result<Box<dyn AsrSession>, EngineError>;
}

/// Driven by one inference worker thread.
pub trait AsrSession: Send {
    /// Decode if enough new audio has arrived; returns any new updates.
    fn poll(&mut self) -> Result<Vec<AsrUpdate>, EngineError>;
    /// End of stream or stop: commit everything still pending.
    fn flush(&mut self) -> Result<Vec<AsrUpdate>, EngineError>;
    /// A seek or channel change: discard pending audio and the provisional
    /// tail without committing them. `restart_ms` is the stream time the next
    /// audio starts at.
    fn reset(&mut self, restart_ms: u64);
}

/// Committed lines to translate, with the lines just before them as context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslateBatch<'a> {
    pub source: &'a str,
    pub target: &'a str,
    /// The preceding lines as (source, translation), for engines that use
    /// context.
    pub context: &'a [(String, String)],
    pub segments: &'a [String],
}

pub trait Translator: Send + Sync {
    fn provider(&self) -> ProviderKind;
    fn supports(&self, source: &str, target: &str) -> bool;
    /// One output line per input segment, in order.
    fn translate(&self, batch: TranslateBatch<'_>) -> Result<Vec<String>, EngineError>;
}

/// The cloud speech-to-text seam. Refuses every request until a real client
/// is added; it holds its configuration so Settings can show it.
#[derive(Debug, Clone, Default)]
pub struct CloudSpeechToText {
    pub config: CloudProviderConfig,
}

impl SpeechToTextEngine for CloudSpeechToText {
    fn provider(&self) -> ProviderKind {
        ProviderKind::Cloud
    }

    fn open_session(
        &self,
        _: AsrSessionConfig,
        _: PcmQueue,
    ) -> Result<Box<dyn AsrSession>, EngineError> {
        Err(EngineError::ProviderNotConfigured)
    }
}

/// The cloud translation seam; see [`CloudSpeechToText`].
#[derive(Debug, Clone, Default)]
pub struct CloudTranslator {
    pub config: CloudProviderConfig,
}

impl Translator for CloudTranslator {
    fn provider(&self) -> ProviderKind {
        ProviderKind::Cloud
    }

    fn supports(&self, _: &str, _: &str) -> bool {
        false
    }

    fn translate(&self, _: TranslateBatch<'_>) -> Result<Vec<String>, EngineError> {
        Err(EngineError::ProviderNotConfigured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Even fully filled in and enabled, the cloud seam refuses work. This
    /// crate has no network dependency, so refusing is all it can do.
    fn enabled_config() -> CloudProviderConfig {
        CloudProviderConfig {
            endpoint: Some("https://example.invalid/v1".into()),
            credential_ref: Some("keychain:nova-cloud".into()),
            enabled: true,
        }
    }

    #[test]
    fn cloud_speech_refuses_without_contacting_anything() {
        let engine = CloudSpeechToText {
            config: enabled_config(),
        };
        let config = AsrSessionConfig {
            language: Some("en".into()),
            sample_rate: ASR_SAMPLE_RATE,
            step_ms: 1000,
        };
        assert_eq!(engine.provider(), ProviderKind::Cloud);
        assert!(matches!(
            engine.open_session(config, PcmQueue::with_capacity(16)),
            Err(EngineError::ProviderNotConfigured)
        ));
    }

    #[test]
    fn cloud_translation_refuses_without_contacting_anything() {
        let engine = CloudTranslator {
            config: enabled_config(),
        };
        assert!(!engine.supports("en", "fr"));
        let segments = vec!["hello".to_string()];
        let batch = TranslateBatch {
            source: "en",
            target: "fr",
            context: &[],
            segments: &segments,
        };
        assert_eq!(
            engine.translate(batch),
            Err(EngineError::ProviderNotConfigured)
        );
    }

    #[test]
    fn queue_drops_the_oldest_audio_when_full() {
        let q = PcmQueue::with_capacity(4);
        q.push(&[1.0, 2.0, 3.0]);
        q.push(&[4.0, 5.0]);
        let mut out = Vec::new();
        q.drain_into(&mut out);
        assert_eq!(out, [2.0, 3.0, 4.0, 5.0]);
        assert_eq!(q.dropped(), 1);
        q.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        out.clear();
        q.drain_into(&mut out);
        assert_eq!(out, [3.0, 4.0, 5.0, 6.0]);
        assert_eq!(q.dropped(), 3);
        q.push(&[7.0]);
        q.clear();
        out.clear();
        q.drain_into(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn default_cloud_config_is_disabled() {
        assert!(!CloudProviderConfig::default().enabled);
    }

    #[test]
    fn route_picks_the_first_ready_provider() {
        let local = CapabilityRoute::local_only(Capability::Transcription);
        assert_eq!(local.pick(|_| true), Some(ProviderKind::Local));
        assert_eq!(local.pick(|_| false), None);

        let hybrid = CapabilityRoute {
            capability: Capability::Translation,
            preferred: vec![ProviderKind::Local, ProviderKind::Cloud],
        };
        assert_eq!(
            hybrid.pick(|kind| kind == ProviderKind::Cloud),
            Some(ProviderKind::Cloud)
        );
    }
}
