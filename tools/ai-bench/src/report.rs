//! `report`: merge candidate JSON reports into one Markdown summary with the
//! hard gates of `docs/implementation/ai-model-evaluation.md` evaluated.

use std::fmt::Write as _;

use serde_json::Value;

/// Hard gates (the rubric). Changing one is a rubric change, not a tweak.
pub const FIRST_TEXT_P50_SEC: f64 = 1.0;
pub const COMMITTED_LAG_P95_SEC: f64 = 3.5;
pub const OFFLINE_RTF: f64 = 0.3;
pub const STREAM_RTF: f64 = 1.0;
pub const ARABIC_WER_FACTOR: f64 = 1.25;
pub const PEAK_RSS_MB: f64 = 2500.0;
pub const MT_P95_MS: f64 = 400.0;

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn num(x: f64, digits: usize) -> String {
    if x.is_nan() {
        "n/a".into()
    } else {
        format!("{x:.digits$}")
    }
}

/// Whether a metric meets its limit. A missing metric (NaN) never does: an
/// unmeasured candidate cannot pass a gate.
fn within(value: f64, limit: f64) -> bool {
    value <= limit
}

fn mark(ok: bool) -> &'static str {
    if ok {
        "pass"
    } else {
        "FAIL"
    }
}

/// Markdown for every report given (ASR reports have `languages`, MT ones
/// `directions`).
pub fn render(reports: &[Value]) -> String {
    let asr: Vec<&Value> = reports
        .iter()
        .filter(|r| r.get("languages").is_some())
        .collect();
    let mt: Vec<&Value> = reports
        .iter()
        .filter(|r| r.get("directions").is_some())
        .collect();
    let mut md = String::new();

    if !asr.is_empty() {
        let _ = writeln!(md, "## Speech recognition\n");
        let _ = writeln!(
            md,
            "Gates (latency is reference-timed from the audio, not engine timestamps): \
             first text after speech onset p50 ≤ {FIRST_TEXT_P50_SEC} s · utterance committed lag p95 ≤ {COMMITTED_LAG_P95_SEC} s · \
             offline RTF ≤ {OFFLINE_RTF} · streaming keeps up (RTF < {STREAM_RTF}, or real-time wall time) · \
             Arabic WER ≤ {ARABIC_WER_FACTOR}× best (needs ≥ 2 candidates) · peak RSS ≤ {PEAK_RSS_MB} MB. \
             RSS covers Nova-side processes only: system speech/translation daemons used by Apple engines are not counted.\n"
        );
        let mut langs: Vec<String> = asr
            .iter()
            .flat_map(|r| r["languages"].as_array().into_iter().flatten())
            .filter_map(|l| l["language"].as_str().map(str::to_string))
            .collect();
        langs.sort();
        langs.dedup();
        for lang in &langs {
            let rows: Vec<(&Value, &Value)> = asr
                .iter()
                .filter_map(|r| {
                    r["languages"]
                        .as_array()?
                        .iter()
                        .find(|l| l["language"].as_str() == Some(lang))
                        .map(|l| (*r, l))
                })
                .collect();
            let arabic_candidates = rows.len();
            let best_offline = rows
                .iter()
                .map(|(_, l)| f(&l["offline"]["quality"]["wer"]))
                .filter(|w| !w.is_nan())
                .fold(f64::INFINITY, f64::min);
            let _ = writeln!(md, "### {lang}\n");
            let _ = writeln!(
                md,
                "| Candidate | Offline WER (95% CI) | CER | Offline RTF | Stream WER | First text after onset p50 | Utterance shown lag p50 / p95 | Utterance committed lag p50 / p95 | Missed utt. | Stream RTF | Rewrites | Load s | RSS MB | Gates |"
            );
            let _ = writeln!(
                md,
                "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|"
            );
            for (r, l) in rows {
                let (o, s) = (&l["offline"], &l["streaming"]);
                // Reference-timed, engine-independent latency (see run.rs).
                let first = f(&s["first_text_latency_p50"]);
                let lag95 = f(&s["utterance_committed_lag_p95"]);
                let off_rtf = f(&o["rtf"]);
                let s_rtf = f(&s["rtf"]);
                let wer = f(&o["quality"]["wer"]);
                let rss = f(&r["peak_rss_mb"]);
                let mut failed = Vec::new();
                let mut notes = Vec::new();
                if !within(first, FIRST_TEXT_P50_SEC) {
                    failed.push("first text");
                }
                if !within(lag95, COMMITTED_LAG_P95_SEC) {
                    failed.push("lag");
                }
                if s["negative_lags"].as_u64().unwrap_or(0) > 0 {
                    notes.push("negative lags: check boundaries");
                }
                if !within(off_rtf, OFFLINE_RTF) {
                    failed.push("offline RTF");
                }
                // Simulated engines: compute must stay under real time.
                // Real-time engines (no separable compute): their measured
                // wall time must have kept pace with the audio.
                let keeps_up = match s["keeps_up"].as_bool() {
                    Some(k) => k,
                    None => within(s_rtf, STREAM_RTF) && s_rtf < STREAM_RTF,
                };
                if !keeps_up {
                    failed.push("keeps up");
                }
                if lang.starts_with("ar") {
                    if arabic_candidates < 2 {
                        notes.push("Arabic WER gate n/a (single candidate)");
                    } else if !within(wer, ARABIC_WER_FACTOR * best_offline) {
                        failed.push("Arabic WER");
                    }
                }
                if !within(rss, PEAK_RSS_MB) {
                    failed.push("RSS");
                }
                let mut gates = if failed.is_empty() {
                    mark(true).to_string()
                } else {
                    format!("{}: {}", mark(false), failed.join(", "))
                };
                if !notes.is_empty() {
                    gates.push_str(&format!(" ({})", notes.join("; ")));
                }
                let ci = match o["quality"]["wer_ci95"].as_array() {
                    Some(v) if v.len() == 2 => {
                        format!(" ({}–{})", num(f(&v[0]), 3), num(f(&v[1]), 3))
                    }
                    _ => String::new(),
                };
                let _ = writeln!(
                    md,
                    "| {} | {}{} | {} | {} | {} | {} s | {} / {} s | {} / {} s | {}/{} | {} | {} | {} | {} | {} |",
                    r["candidate"].as_str().unwrap_or("?"),
                    num(wer, 3),
                    ci,
                    num(f(&o["quality"]["cer"]), 3),
                    num(off_rtf, 3),
                    num(f(&s["quality"]["wer"]), 3),
                    num(first, 2),
                    num(f(&s["utterance_shown_lag_p50"]), 2),
                    num(f(&s["utterance_shown_lag_p95"]), 2),
                    num(f(&s["utterance_committed_lag_p50"]), 2),
                    num(lag95, 2),
                    s["missed_utterances"].as_u64().unwrap_or(0),
                    s["utterances"].as_u64().unwrap_or(0),
                    num(s_rtf, 3),
                    num(f(&s["rewrite_rate"]), 2),
                    num(f(&r["load_sec"]), 2),
                    num(rss, 0),
                    gates
                );
            }
            let _ = writeln!(md);
        }
    }

    if !mt.is_empty() {
        let _ = writeln!(md, "## Translation\n");
        let _ = writeln!(md, "Gate: per-segment latency p95 ≤ {MT_P95_MS} ms · peak RSS ≤ {PEAK_RSS_MB} MB. FLORES-200 devtest.\n");
        let mut dirs: Vec<String> = mt
            .iter()
            .flat_map(|r| r["directions"].as_array().into_iter().flatten())
            .map(|d| {
                format!(
                    "{}-{}",
                    d["source"].as_str().unwrap_or("?"),
                    d["target"].as_str().unwrap_or("?")
                )
            })
            .collect();
        dirs.sort();
        dirs.dedup();
        for dir in &dirs {
            let _ = writeln!(md, "### {dir}\n");
            let _ = writeln!(md, "| Candidate | Engine | chrF++ | BLEU | p50 ms | p95 ms | Load s | RSS MB | Gates |");
            let _ = writeln!(md, "|---|---|---|---|---|---|---|---|---|");
            for r in &mt {
                let Some(d) = r["directions"].as_array().and_then(|a| {
                    a.iter().find(|d| {
                        format!(
                            "{}-{}",
                            d["source"].as_str().unwrap_or(""),
                            d["target"].as_str().unwrap_or("")
                        ) == *dir
                    })
                }) else {
                    continue;
                };
                let p95 = f(&d["latency_ms_p95"]);
                let rss = f(&r["peak_rss_mb"]);
                let mut failed = Vec::new();
                if !within(p95, MT_P95_MS) {
                    failed.push("latency");
                }
                if !within(rss, PEAK_RSS_MB) {
                    failed.push("RSS");
                }
                let gates = if failed.is_empty() {
                    mark(true).to_string()
                } else {
                    format!("{}: {}", mark(false), failed.join(", "))
                };
                let engine = r["engine"].as_str().unwrap_or("?");
                let nc = if engine == "nllb" {
                    " (non-commercial)"
                } else {
                    ""
                };
                let _ = writeln!(
                    md,
                    "| {} | {}{} | {} | {} | {} | {} | {} | {} | {} |",
                    r["candidate"].as_str().unwrap_or("?"),
                    engine,
                    nc,
                    num(f(&d["chrf_pp"]), 1),
                    num(f(&d["bleu"]), 1),
                    num(f(&d["latency_ms_p50"]), 0),
                    num(p95, 0),
                    num(f(&r["load_sec"]), 2),
                    num(rss, 0),
                    gates
                );
            }
            let _ = writeln!(md);
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gates_are_evaluated_per_language_and_direction() {
        let asr = json!({
            "candidate": "fast", "load_sec": 0.4, "peak_rss_mb": 900.0,
            "languages": [{
                "language": "ar_eg",
                "offline": {"quality": {"wer": 0.2, "cer": 0.1}, "rtf": 0.1},
                "streaming": {"quality": {"wer": 0.25}, "first_text_latency_p50": 0.9, "utterance_shown_lag_p50": 0.5,
                              "utterance_shown_lag_p95": 1.0, "utterance_committed_lag_p50": 1.5, "utterance_committed_lag_p95": 3.0,
                              "rtf": 0.5, "rewrite_rate": 0.1, "missed_utterances": 0, "utterances": 10}
            }]
        });
        let slow = json!({
            "candidate": "slow", "load_sec": 1.0, "peak_rss_mb": 3000.0,
            "languages": [{
                "language": "ar_eg",
                "offline": {"quality": {"wer": 0.4, "cer": 0.2}, "rtf": 0.5},
                "streaming": {"quality": {"wer": 0.5}, "first_text_latency_p50": 2.0, "utterance_shown_lag_p50": 1.5,
                              "utterance_shown_lag_p95": 3.0, "utterance_committed_lag_p50": 3.0, "utterance_committed_lag_p95": 6.0,
                              "rtf": 1.2, "rewrite_rate": 0.3, "missed_utterances": 1, "utterances": 10}
            }]
        });
        let mt = json!({
            "candidate": "opus", "engine": "opus", "load_sec": 0.3, "peak_rss_mb": 400.0,
            "directions": [{"source": "en", "target": "fr", "chrf_pp": 67.0, "bleu": 47.0,
                            "latency_ms_p50": 170.0, "latency_ms_p95": 300.0}]
        });
        let md = render(&[asr, slow, mt]);
        assert!(md.contains("| fast |") && md.contains("| pass |"));
        // A real-time engine that fell behind fails "keeps up" despite NaN RTF.
        let apple = json!({
            "candidate": "rt", "load_sec": 0.0, "peak_rss_mb": 100.0,
            "languages": [{"language": "en_us",
                "offline": {"quality": {"wer": 0.05, "cer": 0.02}, "rtf": 0.05},
                "streaming": {"quality": {"wer": 0.05}, "first_text_latency_p50": 0.5,
                    "utterance_committed_lag_p95": 1.0, "rtf": null, "keeps_up": false}}]
        });
        assert!(render(&[apple]).contains("FAIL: keeps up"));
        let single = json!({
            "candidate": "solo", "load_sec": 0.1, "peak_rss_mb": 100.0,
            "languages": [{"language": "ar_eg",
                "offline": {"quality": {"wer": 0.9, "cer": 0.5}, "rtf": 0.05},
                "streaming": {"quality": {"wer": 0.9}, "first_text_latency_p50": 0.5,
                    "utterance_committed_lag_p95": 1.0, "rtf": 0.5}}]
        });
        assert!(render(&[single]).contains("Arabic WER gate n/a"));
        assert!(md.contains("FAIL: first text, lag, offline RTF, keeps up, Arabic WER, RSS"));
        assert!(
            md.contains("### en-fr") && md.contains("| opus | opus | 67.0 | 47.0 | 170 | 300 |")
        );
    }
}
