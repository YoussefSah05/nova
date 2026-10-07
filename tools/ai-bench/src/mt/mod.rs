//! Translation benchmark: engines behind one trait, an English pivot for
//! pairs without a direct model, and the FLORES-200 runner.

pub mod apple;
pub mod llama;
pub mod seq2seq;

use std::fs;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::asr::run::percentile;
use crate::metrics::{bleu, chrf_pp};

/// Translates one subtitle segment; `context` holds the previous source
/// lines (used by engines that benefit from it, ignored by the others).
pub trait SegmentTranslator {
    fn translate(&mut self, text: &str, context: &[String]) -> Result<String>;
}

impl SegmentTranslator for seq2seq::Seq2Seq {
    fn translate(&mut self, text: &str, _context: &[String]) -> Result<String> {
        seq2seq::Seq2Seq::translate(self, text)
    }
}

/// Source → English → target, for pairs without a direct model.
pub struct Pivot {
    pub first: Box<dyn SegmentTranslator>,
    pub second: Box<dyn SegmentTranslator>,
}

impl SegmentTranslator for Pivot {
    fn translate(&mut self, text: &str, context: &[String]) -> Result<String> {
        let english = self.first.translate(text, context)?;
        self.second.translate(&english, &[])
    }
}

/// FLORES-200 file stem for a two-letter language.
pub fn flores_code(lang: &str) -> &'static str {
    match lang {
        "en" => "eng_Latn",
        "fr" => "fra_Latn",
        "ar" => "arb_Arab",
        _ => "unknown",
    }
}

#[derive(Debug, Serialize)]
pub struct DirectionReport {
    pub source: String,
    pub target: String,
    pub segments: usize,
    pub chrf_pp: f64,
    pub bleu: f64,
    pub latency_ms_p50: f64,
    pub latency_ms_p95: f64,
    /// First few outputs for inspection.
    pub samples: Vec<(String, String, String)>,
}

/// Translate the first `limit` FLORES devtest sentences of a direction.
pub fn run_direction(
    engine: &mut dyn SegmentTranslator,
    flores_devtest: &Path,
    source: &str,
    target: &str,
    limit: usize,
    context_lines: usize,
) -> Result<DirectionReport> {
    // The scored sentences, plus three after them used only to warm up
    // (kernels, caches) so no scored sentence is ever seen twice.
    let read = |lang: &str| -> Result<Vec<String>> {
        let path = flores_devtest.join(format!("{}.devtest", flores_code(lang)));
        Ok(fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?
            .lines()
            .take(limit + 3)
            .map(str::to_string)
            .collect())
    };
    let (mut src, mut refs) = (read(source)?, read(target)?);
    let warmup: Vec<String> = src.split_off(limit.min(src.len()));
    refs.truncate(src.len());
    for line in &warmup {
        engine.translate(line, &[])?;
    }
    let mut hyps = Vec::with_capacity(src.len());
    let mut latencies = Vec::with_capacity(src.len());
    for (i, line) in src.iter().enumerate() {
        let context = &src[i.saturating_sub(context_lines)..i];
        let began = Instant::now();
        let out = engine.translate(line, context)?;
        latencies.push(began.elapsed().as_secs_f64() * 1000.0);
        hyps.push(out);
    }
    Ok(DirectionReport {
        source: source.into(),
        target: target.into(),
        segments: src.len(),
        chrf_pp: chrf_pp(&hyps, &refs),
        bleu: bleu(&hyps, &refs),
        latency_ms_p50: percentile(&latencies, 50.0),
        latency_ms_p95: percentile(&latencies, 95.0),
        samples: src
            .iter()
            .zip(&hyps)
            .zip(&refs)
            .take(3)
            .map(|((s, h), r)| (s.clone(), h.clone(), r.clone()))
            .collect(),
    })
}

#[derive(Debug, Serialize)]
pub struct MtReport {
    pub candidate: String,
    pub engine: String,
    pub load_sec: f64,
    pub peak_rss_mb: f64,
    pub directions: Vec<DirectionReport>,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Upper;
    impl SegmentTranslator for Upper {
        fn translate(&mut self, text: &str, _: &[String]) -> Result<String> {
            Ok(text.to_uppercase())
        }
    }
    struct Suffix;
    impl SegmentTranslator for Suffix {
        fn translate(&mut self, text: &str, _: &[String]) -> Result<String> {
            Ok(format!("{text}!"))
        }
    }

    #[test]
    fn pivot_chains_both_legs() {
        let mut p = Pivot {
            first: Box::new(Upper),
            second: Box::new(Suffix),
        };
        assert_eq!(p.translate("salut", &[]).unwrap(), "SALUT!");
    }

    #[test]
    fn runs_a_direction_over_flores_files() {
        let dir = std::env::temp_dir().join(format!("nova-flores-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("eng_Latn.devtest"), "one\ntwo\nthree\nfour\n").unwrap();
        fs::write(dir.join("fra_Latn.devtest"), "ONE\nTWO\nTHREE\nFOUR\n").unwrap();
        let r = run_direction(&mut Upper, &dir, "en", "fr", 3, 2).unwrap();
        assert_eq!(r.segments, 3);
        assert!((r.chrf_pp - 100.0).abs() < 1e-9);
        fs::remove_dir_all(dir).unwrap();
    }
}
