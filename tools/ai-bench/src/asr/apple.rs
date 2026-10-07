//! Apple SpeechAnalyzer / SpeechTranscriber (macOS 26) through the Swift
//! helper (`swift-helper/`), out of process. Streaming runs at real time
//! (the analyzer's own pacing is what is being measured); offline runs feed
//! the file as fast as possible.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use super::{AsrCandidate, OfflineResult, StreamConfig, StreamResult, SAMPLE_RATE};
use crate::text::normalize;

#[derive(Deserialize)]
struct Event {
    t: Option<f64>,
    #[serde(default)]
    text: Option<String>,
    /// From the final "done" line.
    wall_sec: Option<f64>,
    #[serde(default)]
    r#final: bool,
    #[serde(default)]
    words: Vec<(String, f64, f64)>,
    error: Option<String>,
}

pub struct AppleSpeech {
    helper: PathBuf,
    /// Dataset language directory → Apple locale, e.g. `en_us` → `en-US`.
    locale: String,
    fast: bool,
    scratch: tempfile_dir::Dir,
}

mod tempfile_dir {
    /// A private scratch directory removed on drop.
    pub struct Dir(pub std::path::PathBuf);
    impl Dir {
        pub fn new() -> std::io::Result<Self> {
            let p = std::env::temp_dir().join(format!("nova-apple-{}", std::process::id()));
            std::fs::create_dir_all(&p)?;
            Ok(Self(p))
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Write 16 kHz mono f32 PCM as a 16-bit WAV.
fn write_wav(path: &Path, pcm: &[f32]) -> Result<()> {
    let mut f = std::fs::File::create(path)?;
    let data_len = (pcm.len() * 2) as u32;
    let rate = SAMPLE_RATE as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in pcm {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        f.write_all(&v.to_le_bytes())?;
    }
    Ok(())
}

impl AppleSpeech {
    pub fn new(helper: PathBuf, locale: String, fast: bool) -> Result<Self> {
        Ok(Self {
            helper,
            locale,
            fast,
            scratch: tempfile_dir::Dir::new()?,
        })
    }

    fn run(&self, wav: &Path, realtime: bool) -> Result<Vec<Event>> {
        let mut cmd = Command::new(&self.helper);
        cmd.arg("asr").arg(wav).arg(&self.locale).arg("--install");
        if realtime {
            cmd.arg("--realtime");
        }
        if self.fast {
            cmd.arg("--fast");
        }
        let out = cmd.output().context("running nova-apple-helper")?;
        let events: Vec<Event> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if let Some(e) = events.iter().find_map(|e| e.error.clone()) {
            bail!("apple helper: {e}");
        }
        Ok(events)
    }

    /// Scored text: the emitted text of final results (not rebuilt from
    /// timed runs, which could miss untimed runs).
    fn final_text(events: &[Event]) -> String {
        events
            .iter()
            .filter(|e| e.r#final)
            .filter_map(|e| e.text.as_deref())
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl AsrCandidate for AppleSpeech {
    fn offline(&mut self, pcm: &[f32], _language: Option<&str>) -> Result<OfflineResult> {
        let wav = self.scratch.0.join("utterance.wav");
        write_wav(&wav, pcm)?;
        let began = Instant::now();
        let events = self.run(&wav, false)?;
        // The helper's own analysis time: process start and asset checks
        // excluded, as whisper's are (it stays resident).
        let compute = events
            .iter()
            .find_map(|e| e.wall_sec)
            .unwrap_or_else(|| began.elapsed().as_secs_f64());
        Ok(OfflineResult {
            audio_sec: pcm.len() as f64 / SAMPLE_RATE as f64,
            compute_sec: compute,
            text: Self::final_text(&events),
            language: None,
        })
    }

    fn stream(&mut self, pcm: &[f32], _cfg: &StreamConfig) -> Result<StreamResult> {
        let wav = self.scratch.0.join("stream.wav");
        write_wav(&wav, pcm)?;
        let events = self.run(&wav, true)?;
        let mut result = StreamResult {
            audio_sec: pcm.len() as f64 / SAMPLE_RATE as f64,
            // Real-time feed: compute is not separable from pacing.
            compute_sec: f64::NAN,
            ..Default::default()
        };
        // First appearance of each word (text + start), then its finalization.
        let audio_sec = pcm.len() as f64 / SAMPLE_RATE as f64;
        // Words finalized after the audio ended are the end-of-input flush:
        // counted, not timed (as for whisper).
        let flush_after = audio_sec + 0.25;
        let mut first: Vec<(String, f64, f64)> = Vec::new();
        let mut shown_prev: Vec<String> = Vec::new();
        for e in &events {
            let Some(t) = e.t else { continue };
            if result.first_text_at.is_none() && !e.words.is_empty() {
                result.first_text_at = Some(t);
            }
            let keys: Vec<String> = e.words.iter().map(|w| normalize(&w.0)).collect();
            for (w, k) in e.words.iter().zip(&keys) {
                if !first
                    .iter()
                    .any(|(fk, fs, _)| fk == k && (fs - w.1).abs() < 0.6)
                {
                    first.push((k.clone(), w.1, t));
                }
            }
            if e.r#final {
                if t > flush_after {
                    result.flushed_words += e.words.len();
                    shown_prev.clear();
                    continue;
                }
                for (w, k) in e.words.iter().zip(&keys) {
                    let shown_at = first
                        .iter()
                        .find(|(fk, fs, _)| fk == k && (fs - w.1).abs() < 0.6)
                        .map_or(t, |f| f.2);
                    result.shown_lag.push(shown_at - w.2);
                    result.committed_lag.push(t - w.2);
                    result.timeline.push(super::TimedWord {
                        start: w.1,
                        end: w.2,
                        shown_at,
                        committed_at: t,
                    });
                }
                result.committed_words += e.words.len();
                shown_prev.clear();
            } else {
                let kept = shown_prev
                    .iter()
                    .zip(&keys)
                    .take_while(|(a, b)| a == b)
                    .count();
                result.retracted_words += shown_prev.len().saturating_sub(kept);
                shown_prev = keys;
            }
        }
        // Kept up if the backlog did not grow: committed lag over the last
        // quarter of the stream is not much worse than over the first quarter
        // (total wall time would also count the cold model preparation).
        let lags = &result.committed_lag;
        if lags.len() >= 8 {
            let q = lags.len() / 4;
            let median = |v: &[f64]| crate::asr::run::percentile(v, 50.0);
            result.keeps_up = Some(median(&lags[lags.len() - q..]) <= median(&lags[..q]) + 2.0);
        }
        result.text = Self::final_text(&events);
        Ok(result)
    }

    fn detects_language(&self) -> bool {
        false
    }
}
