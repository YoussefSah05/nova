//! A live transcription session: audio from a [`PcmQueue`], decoded on a
//! sliding buffer at a fixed step, committed by LocalAgreement-2.
//!
//! The settings are the AI22 v1 defaults (`docs/evaluations/ai22-2026-10-07.md`):
//! 1 s step, 15 s buffer, 24 prompt words. A 0.5 s step did not keep up.

use std::sync::Arc;

use nova_core::intelligence::{
    AsrSegment, AsrSession, AsrSessionConfig, AsrUpdate, EngineError, PcmQueue, ASR_SAMPLE_RATE,
};

use super::client::BufferDecoder;
use super::policy::{truncate_repetition, LocalAgreement, Word};
use super::protocol::{DecodeRequest, MAX_REQUEST_SAMPLES};

const SAMPLE_RATE: f64 = ASR_SAMPLE_RATE as f64;
/// Longest buffer before it is trimmed at a committed point.
const MAX_BUFFER_SEC: f64 = 15.0;
const PROMPT_WORDS: usize = 24;
/// Detected language is trusted (and then kept) only from a buffer this
/// long: on a second of audio whisper's detection flips between decodes.
const LANGUAGE_LOCK_SEC: f64 = 3.0;
/// ...and only from a decode with real speech: on music or silence whisper
/// still names a language, usually a wrong one.
const LANGUAGE_LOCK_WORDS: usize = 3;

pub struct StreamingSession {
    decoder: Arc<dyn BufferDecoder>,
    audio: PcmQueue,
    step_sec: f64,
    requested_language: Option<String>,
    language: Option<String>,
    /// Audio not yet trimmed, starting at `buffer_start` (session seconds).
    buffer: Vec<f32>,
    buffer_start: f64,
    decoded_until: f64,
    /// Stream time, in ms, that session second 0 corresponds to.
    origin_ms: u64,
    policy: LocalAgreement,
    /// `audio.dropped()` already accounted for as a gap.
    dropped_seen: u64,
}

impl StreamingSession {
    pub fn new(
        decoder: Arc<dyn BufferDecoder>,
        config: AsrSessionConfig,
        audio: PcmQueue,
    ) -> Result<Self, EngineError> {
        if config.sample_rate != ASR_SAMPLE_RATE {
            return Err(EngineError::UnsupportedAudio);
        }
        Ok(Self {
            decoder,
            step_sec: (config.step_ms.max(100) as f64) / 1000.0,
            language: config.language.clone(),
            requested_language: config.language,
            buffer: Vec::new(),
            buffer_start: 0.0,
            decoded_until: 0.0,
            origin_ms: 0,
            policy: LocalAgreement::default(),
            dropped_seen: audio.dropped(),
            audio,
        })
    }

    fn received_until(&self) -> f64 {
        self.buffer_start + self.buffer.len() as f64 / SAMPLE_RATE
    }

    fn segment(&self, words: &[Word]) -> Option<AsrSegment> {
        let (first, last) = (words.first()?, words.last()?);
        let ms = |sec: f64| self.origin_ms + (sec.max(0.0) * 1000.0).round() as u64;
        Some(AsrSegment {
            start_ms: ms(first.start),
            end_ms: ms(last.end),
            text: words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        })
    }

    fn drop_audio_before(&mut self, start: f64) {
        let cut = ((start - self.buffer_start) * SAMPLE_RATE).round() as usize;
        let cut = cut.min(self.buffer.len());
        self.buffer.drain(..cut);
        self.buffer_start += cut as f64 / SAMPLE_RATE;
    }

    /// Decode the whole buffer once and advance the policy.
    fn step(&mut self) -> Result<Vec<AsrUpdate>, EngineError> {
        // Never send more than the encoder window: a worker that fell far
        // behind skips ahead instead of failing every step.
        let overflow = self.buffer.len().saturating_sub(MAX_REQUEST_SAMPLES);
        if overflow > 0 {
            let start = self.buffer_start + overflow as f64 / SAMPLE_RATE;
            self.drop_audio_before(start);
        }
        let request = DecodeRequest {
            samples: self.buffer.len(),
            offset_sec: self.buffer_start,
            prompt: self.policy.prompt(self.buffer_start, PROMPT_WORDS),
            language: self.language.clone(),
        };
        let decoded = self.decoder.decode(&request, &self.buffer)?;
        self.decoded_until = self.received_until();
        if self.language.is_none()
            && self.buffer.len() as f64 / SAMPLE_RATE >= LANGUAGE_LOCK_SEC
            && decoded.words.len() >= LANGUAGE_LOCK_WORDS
        {
            self.language = decoded.language.clone();
        }

        let mut hypothesis: Vec<Word> = decoded
            .words
            .into_iter()
            .map(|w| Word {
                text: w.text,
                start: w.start,
                end: w.end,
            })
            .collect();
        truncate_repetition(&mut hypothesis);
        let out = self.policy.step(hypothesis, self.buffer_start);

        let mut updates = Vec::with_capacity(2);
        if let Some(segment) = self.segment(&out.committed) {
            updates.push(AsrUpdate::Committed(vec![segment]));
        }
        // An empty provisional update clears the dimmed tail on screen.
        updates.push(AsrUpdate::Provisional(self.segment(&out.provisional).unwrap_or(
            AsrSegment {
                start_ms: self.origin_ms + (self.decoded_until * 1000.0) as u64,
                end_ms: self.origin_ms + (self.decoded_until * 1000.0) as u64,
                text: String::new(),
            },
        )));

        let received = self.received_until();
        if received - self.buffer_start > MAX_BUFFER_SEC {
            // Prefer a committed sentence end; otherwise keep half a second of
            // committed audio, which the text alignment strips again.
            let start = self
                .policy
                .trim_point(self.buffer_start)
                .unwrap_or_else(|| (self.policy.committed_until() - 0.5).max(self.buffer_start))
                .max(received - 2.0 * MAX_BUFFER_SEC);
            self.drop_audio_before(start);
        }
        Ok(updates)
    }
}

impl StreamingSession {
    /// The queue dropped audio because decoding fell behind: what arrives
    /// next starts after a gap. Commit the pending tail and restart the
    /// buffer after the gap, so later timestamps stay on the media clock and
    /// whisper never decodes across a splice.
    fn skip_dropped_audio(&mut self) -> Vec<AsrUpdate> {
        let dropped = self.audio.dropped();
        let gap = dropped.saturating_sub(self.dropped_seen);
        self.dropped_seen = dropped;
        if gap == 0 {
            return Vec::new();
        }
        let resume = self.received_until() + gap as f64 / SAMPLE_RATE;
        self.buffer.clear();
        self.buffer_start = resume;
        self.decoded_until = resume;
        let pending = self.policy.flush();
        self.segment(&pending)
            .map(|segment| vec![AsrUpdate::Committed(vec![segment])])
            .unwrap_or_default()
    }
}

impl AsrSession for StreamingSession {
    fn poll(&mut self) -> Result<Vec<AsrUpdate>, EngineError> {
        let mut updates = self.skip_dropped_audio();
        self.audio.drain_into(&mut self.buffer);
        if self.received_until() - self.decoded_until >= self.step_sec {
            updates.extend(self.step()?);
        }
        Ok(updates)
    }

    fn flush(&mut self) -> Result<Vec<AsrUpdate>, EngineError> {
        self.audio.drain_into(&mut self.buffer);
        let mut updates = if self.received_until() > self.decoded_until && !self.buffer.is_empty() {
            self.step()?
        } else {
            Vec::new()
        };
        updates.retain(|u| matches!(u, AsrUpdate::Committed(_)));
        let rest = self.policy.flush();
        if let Some(segment) = self.segment(&rest) {
            updates.push(AsrUpdate::Committed(vec![segment]));
        }
        Ok(updates)
    }

    fn reset(&mut self, restart_ms: u64) {
        self.audio.clear();
        self.dropped_seen = self.audio.dropped();
        self.buffer.clear();
        self.buffer_start = 0.0;
        self.decoded_until = 0.0;
        self.origin_ms = restart_ms;
        self.policy = LocalAgreement::default();
        // A channel change may change the spoken language.
        self.language = self.requested_language.clone();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::streaming::protocol::{Accel, Decoded, WireWord};

    /// Replays scripted hypotheses and records what it was asked.
    struct Scripted {
        hypotheses: Mutex<Vec<Vec<(&'static str, f64)>>>,
        requests: Mutex<Vec<DecodeRequest>>,
        detected: Option<&'static str>,
    }

    impl Scripted {
        fn new(hypotheses: Vec<Vec<(&'static str, f64)>>) -> Arc<Self> {
            Arc::new(Self {
                hypotheses: Mutex::new(hypotheses.into_iter().rev().collect()),
                requests: Mutex::new(Vec::new()),
                detected: Some("fr"),
            })
        }
    }

    impl BufferDecoder for Scripted {
        fn decode(&self, request: &DecodeRequest, pcm: &[f32]) -> Result<Decoded, EngineError> {
            assert_eq!(pcm.len(), request.samples);
            self.requests.lock().unwrap().push(request.clone());
            let words = self.hypotheses.lock().unwrap().pop().unwrap_or_default();
            Ok(Decoded {
                words: words
                    .into_iter()
                    .map(|(text, start)| WireWord {
                        text: text.into(),
                        start,
                        end: start + 0.3,
                    })
                    .collect(),
                language: self.detected.map(str::to_string),
                accel: Accel::Metal,
                decode_ms: 1,
            })
        }
    }

    fn config(language: Option<&str>) -> AsrSessionConfig {
        AsrSessionConfig {
            language: language.map(str::to_string),
            sample_rate: ASR_SAMPLE_RATE,
            step_ms: 1000,
        }
    }

    fn seconds(s: f64) -> Vec<f32> {
        vec![0.0; (s * SAMPLE_RATE) as usize]
    }

    fn texts(updates: &[AsrUpdate]) -> Vec<String> {
        updates
            .iter()
            .map(|u| match u {
                AsrUpdate::Committed(s) => format!("C:{}", s[0].text),
                AsrUpdate::Provisional(s) => format!("P:{}", s.text),
            })
            .collect()
    }

    #[test]
    fn decodes_once_per_step_and_commits_on_agreement() {
        let decoder = Scripted::new(vec![
            vec![("bonjour", 0.1)],
            vec![("bonjour", 0.1), ("à", 0.6)],
            vec![("bonjour", 0.1), ("à", 0.6), ("tous", 1.2)],
        ]);
        let audio = PcmQueue::with_capacity(16_000 * 60);
        let mut session = StreamingSession::new(decoder.clone(), config(None), audio.clone()).unwrap();

        audio.push(&seconds(0.5));
        assert!(session.poll().unwrap().is_empty(), "less than one step of audio");
        audio.push(&seconds(0.5));
        assert_eq!(texts(&session.poll().unwrap()), ["P:bonjour"]);
        audio.push(&seconds(1.0));
        assert_eq!(texts(&session.poll().unwrap()), ["C:bonjour", "P:à"]);
        audio.push(&seconds(1.0));
        assert_eq!(texts(&session.poll().unwrap()), ["C:à", "P:tous"]);
        assert_eq!(texts(&session.flush().unwrap()), ["C:tous"]);
        assert_eq!(decoder.requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn detected_language_is_kept_once_the_buffer_is_long_enough() {
        let speech = vec![("il", 0.1), ("fait", 0.4), ("beau", 0.7)];
        let decoder = Scripted::new(vec![speech.clone(), speech.clone(), speech.clone(), speech]);
        let audio = PcmQueue::with_capacity(16_000 * 60);
        let mut session = StreamingSession::new(decoder.clone(), config(None), audio.clone()).unwrap();
        for _ in 0..4 {
            audio.push(&seconds(1.0));
            session.poll().unwrap();
        }
        let languages: Vec<Option<String>> = decoder
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.language.clone())
            .collect();
        assert_eq!(languages, [None, None, None, Some("fr".into())]);
    }

    #[test]
    fn reset_discards_pending_text_and_restarts_the_clock() {
        let decoder = Scripted::new(vec![
            vec![("old", 0.1)],
            vec![("new", 0.2)],
            vec![("new", 0.2)],
        ]);
        let audio = PcmQueue::with_capacity(16_000 * 60);
        let mut session =
            StreamingSession::new(decoder.clone(), config(Some("en")), audio.clone()).unwrap();
        audio.push(&seconds(1.0));
        session.poll().unwrap();
        audio.push(&seconds(0.4));
        session.reset(60_000);
        assert!(session.flush().unwrap().is_empty(), "nothing from before the seek");
        audio.push(&seconds(1.0));
        session.poll().unwrap();
        audio.push(&seconds(1.0));
        let updates = session.poll().unwrap();
        let AsrUpdate::Committed(segments) = &updates[0] else {
            panic!("expected a commit");
        };
        assert_eq!(segments[0].text, "new");
        assert_eq!(segments[0].start_ms, 60_200, "times are in stream ms after the seek");
        let first_after_reset = &decoder.requests.lock().unwrap()[1];
        assert_eq!(first_after_reset.offset_sec, 0.0);
        assert_eq!(first_after_reset.samples, 16_000);
    }

    #[test]
    fn the_buffer_is_trimmed_at_a_committed_sentence_end() {
        let sentence = vec![("one.", 0.2), ("two", 15.5)];
        let decoder = Scripted::new(vec![sentence.clone(), sentence]);
        let audio = PcmQueue::with_capacity(16_000 * 60);
        let mut session =
            StreamingSession::new(decoder.clone(), config(Some("en")), audio.clone()).unwrap();
        audio.push(&seconds(8.0));
        session.poll().unwrap();
        audio.push(&seconds(8.0));
        session.poll().unwrap();
        // "one." committed (ends 0.5 s) and the buffer passed 15 s: trimmed there.
        audio.push(&seconds(1.0));
        session.poll().unwrap();
        let last = decoder.requests.lock().unwrap().last().cloned().unwrap();
        assert!((last.offset_sec - 0.5).abs() < 1e-3, "{}", last.offset_sec);
        assert_eq!(last.prompt, "one.");
    }

    #[test]
    fn dropped_audio_becomes_a_gap_not_a_time_shift() {
        let decoder = Scripted::new(vec![
            vec![("avant", 0.2)],
            vec![("après", 0.3)],
        ]);
        // Room for 2 s: pushing 3 s drops the oldest second.
        let audio = PcmQueue::with_capacity(32_000);
        let mut session =
            StreamingSession::new(decoder.clone(), config(Some("fr")), audio.clone()).unwrap();
        audio.push(&seconds(1.0));
        session.poll().unwrap();
        audio.push(&seconds(3.0));
        let updates = session.poll().unwrap();
        assert_eq!(texts(&updates)[0], "C:avant", "pending text committed at the gap");
        let last = decoder.requests.lock().unwrap().last().cloned().unwrap();
        assert!((last.offset_sec - 2.0).abs() < 1e-6, "resumes after the 1 s gap: {}", last.offset_sec);
        assert_eq!(last.samples, 32_000);
    }

    #[test]
    fn language_is_not_locked_from_a_decode_without_speech() {
        let decoder = Scripted::new(vec![vec![], vec![], vec![], vec![]]);
        let audio = PcmQueue::with_capacity(16_000 * 60);
        let mut session = StreamingSession::new(decoder.clone(), config(None), audio.clone()).unwrap();
        for _ in 0..4 {
            audio.push(&seconds(1.0));
            session.poll().unwrap();
        }
        assert!(decoder.requests.lock().unwrap().iter().all(|r| r.language.is_none()));
    }

    #[test]
    fn other_sample_rates_are_refused() {
        let mut config = config(None);
        config.sample_rate = 48_000;
        assert!(matches!(
            StreamingSession::new(Scripted::new(vec![]), config, PcmQueue::with_capacity(1)),
            Err(EngineError::UnsupportedAudio)
        ));
    }
}
