//! Headless import probe — the validation loop for the import pipeline.
//!
//! Reproduces `add_source`'s routing (detect → Xtream JSON API or plain M3U)
//! and drains the resulting channel with a per-receive timeout, so a stalled
//! provider is reported as a failure with timing instead of hanging forever.
//!
//! Usage:
//!   NOVA_PROBE_URL='<playlist url>' cargo run -p nova-xtream --example probe
//!
//! The URL carries credentials — pass it via the env var, never commit it.

use std::time::{Duration, Instant};

use nova_core::MediaKind;

/// Abort if no entry arrives within this window. Longer than the Xtream HTTP
/// read timeout (30s) so a real HTTP error surfaces before this trips.
const RECV_TIMEOUT: Duration = Duration::from_secs(45);

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let url = std::env::var("NOVA_PROBE_URL")
        .ok()
        .or_else(|| std::env::args().nth(1));
    let Some(url) = url else {
        eprintln!("error: set NOVA_PROBE_URL or pass the playlist URL as an argument");
        std::process::exit(2);
    };

    let is_xtream = nova_xtream::detect(&url);
    println!(
        "probe: routing as {}",
        if is_xtream {
            "Xtream JSON API"
        } else {
            "plain M3U"
        }
    );

    let start = Instant::now();
    let mut rx = if is_xtream {
        match nova_xtream::XtreamCredentials::from_url(&url) {
            Some(creds) => nova_xtream::fetch(&creds).await,
            None => {
                eprintln!("error: URL looks like Xtream but credentials could not be parsed");
                std::process::exit(2);
            }
        }
    } else {
        nova_m3u::parse(&url).await.entries
    };

    let (mut live, mut movie, mut series) = (0u64, 0u64, 0u64);
    let mut errors: Vec<String> = Vec::new();

    loop {
        match tokio::time::timeout(RECV_TIMEOUT, rx.recv()).await {
            Ok(Some(Ok(entry))) => match entry.kind {
                MediaKind::Live => live += 1,
                MediaKind::Movie => movie += 1,
                MediaKind::Series => series += 1,
            },
            Ok(Some(Err(e))) => errors.push(e),
            Ok(None) => break, // channel closed — import finished
            Err(_) => {
                eprintln!(
                    "\nFAIL: HANG — no entry for {}s (counts so far: live={live} movie={movie} series={series})",
                    RECV_TIMEOUT.as_secs()
                );
                std::process::exit(1);
            }
        }
    }

    let total = live + movie + series;
    let elapsed = start.elapsed();
    println!("\nprobe summary ({:.1}s elapsed):", elapsed.as_secs_f64());
    println!("  live   : {live}");
    println!("  movie  : {movie}");
    println!("  series : {series}");
    println!("  total  : {total}");
    if !errors.is_empty() {
        println!("  errors : {}", errors.len());
        for e in &errors {
            println!("    - {e}");
        }
    }

    if !errors.is_empty() {
        eprintln!("FAIL: import reported {} error(s)", errors.len());
        std::process::exit(1);
    }
    if total == 0 {
        eprintln!("FAIL: import completed but produced 0 entries");
        std::process::exit(1);
    }
    println!("OK: import completed cleanly");
}
