//! Nova AI benchmark harness (AI22). See `README.md`.

mod asr;
mod metrics;
mod mt;
mod report;
mod rss;
mod text;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use asr::run::{run_language, CandidateReport, RunOptions};
use asr::whisper::{WhisperEngine, WhisperTuning};
use asr::StreamConfig;

#[derive(Parser)]
#[command(about = "Benchmark Nova's local ASR and translation candidates")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Speech recognition: offline quality and streaming latency per language.
    Asr {
        /// Report label, e.g. `whisper-turbo-q5-coreml`.
        #[arg(long)]
        name: String,
        /// `whisper` (in-process whisper.cpp) or `apple` (SpeechTranscriber helper).
        #[arg(long, default_value = "whisper")]
        engine: String,
        /// ggml model file (whisper engine).
        #[arg(long)]
        model: Option<PathBuf>,
        /// Path to `nova-apple-helper` (apple engine).
        #[arg(long)]
        helper: Option<PathBuf>,
        /// Apple `.fastResults` low-latency reporting.
        #[arg(long)]
        fast: bool,
        /// Dataset directory holding one sub-directory per language.
        #[arg(long)]
        data: PathBuf,
        /// Language sub-directories to run, e.g. `en_us,fr_fr,ar_eg`.
        #[arg(long, value_delimiter = ',')]
        langs: Vec<String>,
        #[arg(long, default_value_t = 40)]
        offline_limit: usize,
        #[arg(long, default_value_t = 6)]
        streams: usize,
        #[arg(long, default_value_t = 90.0)]
        stream_sec: f64,
        #[arg(long, default_value_t = 1.0)]
        step: f64,
        #[arg(long, default_value_t = 15.0)]
        max_buffer: f64,
        #[arg(long, default_value_t = 4)]
        threads: i32,
        /// Size the encoder context to the buffer (faster short decodes).
        #[arg(long)]
        adaptive_ctx: bool,
        /// Smallest adaptive encoder context in seconds.
        #[arg(long, default_value_t = 10.0)]
        ctx_floor: f64,
        /// Disable temperature-fallback re-decodes.
        #[arg(long)]
        no_fallback: bool,
        /// JSON report path.
        #[arg(long)]
        out: PathBuf,
    },
    /// Translation: quality (chrF++, BLEU) and per-segment latency on FLORES-200.
    Mt {
        #[arg(long)]
        name: String,
        /// `opus` (Marian per pair, English pivot when missing), `nllb`, `llama`,
        /// or `apple` (`--models` is then the helper binary).
        #[arg(long)]
        engine: String,
        /// opus: directory of `opus-mt-<src>-<tgt>` models; nllb: model dir; llama: GGUF file.
        #[arg(long)]
        models: PathBuf,
        /// Directions like `en-fr,fr-ar`.
        #[arg(long, value_delimiter = ',')]
        pairs: Vec<String>,
        /// FLORES-200 `devtest` directory.
        #[arg(long)]
        flores: PathBuf,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Previous source lines given as context (LLM engines).
        #[arg(long, default_value_t = 2)]
        context: usize,
        #[arg(long, default_value_t = 4)]
        threads: usize,
        #[arg(long)]
        out: PathBuf,
    },
    /// Merge candidate JSON reports into one Markdown summary with gates.
    Report {
        /// Candidate JSON reports (asr and/or mt).
        reports: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Translate lines from stdin with one ONNX model (smoke test).
    MtSmoke {
        /// Model directory (config.json, tokenizer.json, onnx/).
        #[arg(long)]
        model: PathBuf,
        /// `marian`, or `nllb:<src>:<tgt>` with two-letter codes.
        #[arg(long)]
        family: String,
        #[arg(long, default_value_t = 4)]
        threads: usize,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Asr {
            name,
            engine,
            model,
            helper,
            fast,
            data,
            langs,
            offline_limit,
            streams,
            stream_sec,
            step,
            max_buffer,
            threads,
            adaptive_ctx,
            ctx_floor,
            no_fallback,
            out,
        } => {
            let tuning = WhisperTuning {
                adaptive_ctx,
                ctx_floor_sec: ctx_floor,
                no_fallback,
            };
            let opts = RunOptions {
                offline_limit,
                streams,
                stream_sec,
                stream: StreamConfig {
                    step_sec: step,
                    max_buffer_sec: max_buffer,
                    prompt_words: 24,
                    language: None,
                },
            };
            let mut languages = Vec::new();
            let (engine_label, load_sec, model_path) = match engine.as_str() {
                "whisper" => {
                    let model = model.context("--model is required for the whisper engine")?;
                    let mut w = WhisperEngine::load(&model, threads, tuning)?;
                    eprintln!("{name}: loaded in {:.2}s", w.load_sec);
                    for lang in &langs {
                        languages.push(run_language(&mut w, &data.join(lang), &opts)?);
                    }
                    (
                        "whisper.cpp (whisper-rs, resident)".to_string(),
                        w.load_sec,
                        model,
                    )
                }
                "apple" => {
                    let helper = helper.context("--helper is required for the apple engine")?;
                    for lang in &langs {
                        let mut a =
                            asr::apple::AppleSpeech::new(helper.clone(), apple_locale(lang), fast)?;
                        languages.push(run_language(&mut a, &data.join(lang), &opts)?);
                    }
                    let label = if fast {
                        "Apple SpeechTranscriber (fastResults)"
                    } else {
                        "Apple SpeechTranscriber"
                    };
                    (label.to_string(), 0.0, helper)
                }
                other => anyhow::bail!("unknown engine `{other}`"),
            };
            let report = CandidateReport {
                candidate: name,
                engine: engine_label,
                coreml: cfg!(feature = "coreml"),
                tuning,
                load_sec,
                model_path,
                dataset: data,
                step_sec: step,
                peak_rss_mb: rss::peak_rss_mb(),
                languages,
            };
            std::fs::write(&out, serde_json::to_string_pretty(&report)?)
                .with_context(|| format!("writing {}", out.display()))?;
            eprintln!("report: {}", out.display());
        }
        Command::Mt {
            name,
            engine,
            models,
            pairs,
            flores,
            limit,
            context,
            threads,
            out,
        } => {
            let report = run_mt(
                &name, &engine, &models, &pairs, &flores, limit, context, threads,
            )?;
            std::fs::write(&out, serde_json::to_string_pretty(&report)?)
                .with_context(|| format!("writing {}", out.display()))?;
            eprintln!("report: {}", out.display());
        }
        Command::Report { reports, out } => {
            let values = reports
                .iter()
                .map(|p| -> Result<serde_json::Value> {
                    Ok(serde_json::from_str(
                        &std::fs::read_to_string(p)
                            .with_context(|| format!("reading {}", p.display()))?,
                    )?)
                })
                .collect::<Result<Vec<_>>>()?;
            std::fs::write(&out, report::render(&values))?;
            eprintln!("summary: {}", out.display());
        }
        Command::MtSmoke {
            model,
            family,
            threads,
        } => {
            let family = parse_family(&family)?;
            let mut engine = mt::seq2seq::Seq2Seq::load(&model, family, threads)?;
            eprintln!("loaded in {:.2}s", engine.load_sec);
            for line in std::io::stdin().lines() {
                let line = line?;
                let began = std::time::Instant::now();
                let out = engine.translate(&line)?;
                println!("{:>6.0} ms  {out}", began.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }
    Ok(())
}

/// Apple locale for a dataset language directory: `en_us` → `en-US`.
fn apple_locale(dir: &str) -> String {
    match dir.split_once('_') {
        Some((lang, region)) => format!("{lang}-{}", region.to_uppercase()),
        None => dir.to_string(),
    }
}

fn split_pair(pair: &str) -> Result<(String, String)> {
    let (a, b) = pair
        .split_once('-')
        .with_context(|| format!("pair `{pair}` must look like en-fr"))?;
    Ok((a.to_string(), b.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn run_mt(
    name: &str,
    engine: &str,
    models: &std::path::Path,
    pairs: &[String],
    flores: &std::path::Path,
    limit: usize,
    context: usize,
    threads: usize,
) -> Result<mt::MtReport> {
    use mt::seq2seq::{Family, Seq2Seq};
    use mt::{run_direction, Pivot, SegmentTranslator};
    let mut directions = Vec::new();
    let mut load_sec = 0.0;
    match engine {
        "opus" => {
            // One Marian model per direction; English pivot when missing.
            let load = |a: &str, b: &str, load_sec: &mut f64| -> Result<Option<Seq2Seq>> {
                let dir = models.join(format!("opus-mt-{a}-{b}"));
                if !dir.exists() {
                    return Ok(None);
                }
                let mut m = Seq2Seq::load(&dir, Family::Marian, threads)?;
                *load_sec += m.load_sec;
                // Multi-target Marian models (e.g. opus-mt-en-ar) need the
                // target tag first; single-target models have no such token.
                if b == "ar" && m.has_token(">>ara<<") {
                    m.set_source_prefix(">>ara<<")?;
                    eprintln!("opus-mt-{a}-{b}: using target tag >>ara<<");
                }
                Ok(Some(m))
            };
            for pair in pairs {
                let (a, b) = split_pair(pair)?;
                let mut engine: Box<dyn SegmentTranslator> = match load(&a, &b, &mut load_sec)? {
                    Some(m) => Box::new(m),
                    None => {
                        let first =
                            load(&a, "en", &mut load_sec)?.context("no pivot model to English")?;
                        let second = load("en", &b, &mut load_sec)?
                            .context("no pivot model from English")?;
                        eprintln!("{pair}: no direct model, pivoting through English");
                        Box::new(Pivot {
                            first: Box::new(first),
                            second: Box::new(second),
                        })
                    }
                };
                directions.push(run_direction(
                    engine.as_mut(),
                    flores,
                    &a,
                    &b,
                    limit,
                    context,
                )?);
            }
        }
        "nllb" => {
            let mut m = Seq2Seq::load(
                models,
                Family::Nllb {
                    source: "en".into(),
                    target: "fr".into(),
                },
                threads,
            )?;
            load_sec = m.load_sec;
            for pair in pairs {
                let (a, b) = split_pair(pair)?;
                m.set_direction(&a, &b);
                directions.push(run_direction(&mut m, flores, &a, &b, limit, context)?);
            }
        }
        "llama" => {
            let mut m = mt::llama::LlamaTranslator::start(models)?;
            load_sec = m.load_sec;
            for pair in pairs {
                let (a, b) = split_pair(pair)?;
                m.set_direction(&a, &b);
                directions.push(run_direction(&mut m, flores, &a, &b, limit, context)?);
            }
        }
        "apple" => {
            for pair in pairs {
                let (a, b) = split_pair(pair)?;
                let mut m = mt::apple::AppleTranslator::start(models, &a, &b)?;
                directions.push(run_direction(&mut m, flores, &a, &b, limit, context)?);
            }
        }
        other => anyhow::bail!("unknown engine `{other}`"),
    }
    for d in &directions {
        eprintln!(
            "  {}-{}: chrF++ {:.1}  BLEU {:.1}  p50 {:.0} ms  p95 {:.0} ms",
            d.source, d.target, d.chrf_pp, d.bleu, d.latency_ms_p50, d.latency_ms_p95
        );
    }
    Ok(mt::MtReport {
        candidate: name.into(),
        engine: engine.into(),
        load_sec,
        peak_rss_mb: rss::peak_rss_mb(),
        directions,
    })
}

fn parse_family(spec: &str) -> Result<mt::seq2seq::Family> {
    match spec.split(':').collect::<Vec<_>>().as_slice() {
        ["marian"] => Ok(mt::seq2seq::Family::Marian),
        ["nllb", src, tgt] => Ok(mt::seq2seq::Family::Nllb {
            source: src.to_string(),
            target: tgt.to_string(),
        }),
        _ => anyhow::bail!("family must be `marian` or `nllb:<src>:<tgt>`"),
    }
}
