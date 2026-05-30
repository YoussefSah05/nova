//! Streaming M3U/M3U8 playlist parser.
//!
//! Supports local file paths and HTTP(S) URLs. Entries are classified into
//! Live / Movie / Series based on the stream URL path segment, then enriched
//! with country prefix, year, season/episode fields derived from the name.
//!
//! Parsed entries are emitted one by one through a tokio mpsc channel so the
//! caller can batch-insert them without holding all 100k+ entries in memory.

use std::collections::HashMap;
use std::process::Stdio;

use nova_core::{normalize_category, MediaKind, ParsedEntry};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot};

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum M3uError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP {0}: server rejected the playlist request")]
    BadStatus(u16),
    #[error("server did not respond in time — try again or check the URL")]
    Timeout,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Unified entry type: errors are pre-stringified so callers (commands.rs)
/// don't need to import M3uError.
pub type Entry = Result<ParsedEntry, String>;

/// Playlist-level metadata extracted from the `#EXTM3U` header line.
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    /// `url-tvg` (or `x-tvg-url`) attribute — points to an XMLTV EPG feed.
    pub url_tvg: Option<String>,
}

/// Streaming playlist handle returned by [`parse`].
///
/// `entries` is the per-row stream (existing behaviour). `metadata` is a
/// oneshot that resolves once the `#EXTM3U` header line has been processed —
/// it's how Phase 4 surfaces the `url-tvg` EPG hint without breaking the
/// row-streaming API.
pub struct Playlist {
    pub entries: mpsc::Receiver<Entry>,
    pub metadata: oneshot::Receiver<Metadata>,
}

/// Start parsing an M3U playlist from a URL or local file path.
///
/// Returns a [`Playlist`] immediately. Entries arrive as `Ok(entry)` on the
/// `entries` channel. Any HTTP/IO error is sent as a single terminal `Err(e)`
/// followed by channel close, so callers always see errors rather than a
/// silent empty run.
pub async fn parse(source: &str) -> Playlist {
    let (tx, rx) = mpsc::channel::<Entry>(512);
    let (meta_tx, meta_rx) = oneshot::channel::<Metadata>();
    let source = source.to_string();

    tokio::spawn(async move {
        stream_source(&source, tx, meta_tx).await;
    });

    Playlist {
        entries: rx,
        metadata: meta_rx,
    }
}

// ── Source dispatch ───────────────────────────────────────────────────────────

async fn stream_source(source: &str, tx: mpsc::Sender<Entry>, meta_tx: oneshot::Sender<Metadata>) {
    let mut meta_tx = Some(meta_tx);
    if source.starts_with("http://") || source.starts_with("https://") {
        stream_via_curl(source, tx, &mut meta_tx).await;
    } else {
        match tokio::fs::read_to_string(source).await {
            Ok(text) => process_text(&text, &tx, &mut meta_tx).await,
            Err(e) => {
                let _ = tx.send(Err(M3uError::Io(e).to_string())).await;
            }
        }
    }
    // If we never saw an #EXTM3U header, still close the oneshot with default
    // metadata so the receiver doesn't hang.
    if let Some(meta_tx) = meta_tx.take() {
        let _ = meta_tx.send(Metadata::default());
    }
}

/// Downloads a playlist via the system `curl` binary and emits entries as each
/// line arrives — no full-file buffering.
///
/// Flags used:
/// - `-s`            suppress progress meter
/// - `-L`            follow HTTP redirects
/// - `-N`            disable output buffering (flush every byte to stdout immediately)
/// - `--connect-timeout 15` fail fast when the provider host cannot be reached
/// - `--speed-time 30` / `--speed-limit 1` abort stalled reads
/// - `--max-time 300` hard 5-minute timeout for the entire download
/// - `--compressed`  request gzip, auto-decompress — smaller transfer
/// - `--fail`        exit code 22 on HTTP 4xx/5xx so we report the error
/// - `kill_on_drop`  kills curl if the receiver is dropped (user cancelled)
async fn stream_via_curl(
    url: &str,
    tx: mpsc::Sender<Entry>,
    meta_tx: &mut Option<oneshot::Sender<Metadata>>,
) {
    let started_at = std::time::Instant::now();
    // Use the absolute path so the binary is found even when launched from a
    // GUI context where PATH does not include /usr/bin.
    let curl_bin = if std::path::Path::new("/usr/bin/curl").exists() {
        "/usr/bin/curl"
    } else {
        "curl" // fallback for Linux CI
    };

    let mut child = match tokio::process::Command::new(curl_bin)
        .args([
            "-s", // no progress meter
            "-L", // follow redirects
            "-N", // --no-buffer: flush every byte to stdout immediately
            "--connect-timeout",
            "15",
            "--speed-time",
            "30",
            "--speed-limit",
            "1",
            "--max-time",
            "300",
            "--compressed", // request gzip, auto-decompress
            "--user-agent",
            "Mozilla/5.0 (compatible; nova-iptv/1.0)",
            "--fail", // exit code 22 on HTTP 4xx/5xx
            url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit()) // curl errors appear in the terminal for debugging
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Err(format!("failed to spawn curl: {e}"))).await;
            return;
        }
    };

    let stdout = child.stdout.take().expect("stdout was piped");
    let mut lines = BufReader::new(stdout).lines();
    let mut pending: Option<ExtinfAttrs> = None;
    let mut first_line = true;

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();

                // Validate that the response is actually an M3U playlist.
                // Servers sometimes return an HTML or JSON error body with HTTP 200,
                // which --fail cannot catch. Detecting the header early lets us
                // surface a useful error instead of silently importing 0 entries.
                if first_line {
                    first_line = false;
                    if !trimmed.is_empty()
                        && !trimmed.starts_with("#EXTM3U")
                        && !trimmed.starts_with("#EXTINF")
                    {
                        let preview = trimmed.chars().take(120).collect::<String>();
                        let _ = tx
                            .send(Err(format!(
                                "server did not return a playlist — got: {preview}\n\
                             (check the URL and credentials)"
                            )))
                            .await;
                        let _ = child.kill().await;
                        return;
                    }
                }

                if !process_line(trimmed, &mut pending, &tx, meta_tx).await {
                    let _ = child.kill().await;
                    return;
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = tx.send(Err(format!("curl read error: {e}"))).await;
                return;
            }
        }
    }

    if let Ok(status) = child.wait().await {
        if status.success() {
            tracing::info!(
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "M3U playlist stream complete"
            );
        } else {
            let msg = match status.code().unwrap_or(-1) {
                6 => "could not resolve host — check the URL".to_string(),
                7 => "could not connect to server".to_string(),
                22 => "server returned an HTTP error (wrong credentials or URL?)".to_string(),
                28 => "download timed out after 5 minutes".to_string(),
                35 => "SSL/TLS handshake failed".to_string(),
                c => format!("curl exited with code {c}"),
            };
            let _ = tx.send(Err(msg)).await;
        }
    }
}

// ── Core line-processing ──────────────────────────────────────────────────────

async fn process_text(
    text: &str,
    tx: &mpsc::Sender<Entry>,
    meta_tx: &mut Option<oneshot::Sender<Metadata>>,
) {
    let mut pending: Option<ExtinfAttrs> = None;
    for line in text.lines() {
        if !process_line(line.trim(), &mut pending, tx, meta_tx).await {
            break;
        }
    }
}

/// Returns `false` when the receiver has been dropped (stop signal).
async fn process_line(
    line: &str,
    pending: &mut Option<ExtinfAttrs>,
    tx: &mpsc::Sender<Entry>,
    meta_tx: &mut Option<oneshot::Sender<Metadata>>,
) -> bool {
    if line.is_empty() {
        return true;
    }
    if line.starts_with("#EXTM3U") {
        // Parse header attrs once (e.g. `url-tvg="https://example.com/epg.xml"`).
        if let Some(sender) = meta_tx.take() {
            let meta = parse_header(line);
            let _ = sender.send(meta);
        }
        return true;
    }
    if line.starts_with("#EXTINF") {
        *pending = Some(parse_extinf(line));
    } else if line.starts_with('#') {
        // Other directives — ignore
    } else if let Some(attrs) = pending.take() {
        let entry = classify(attrs, line);
        return tx.send(Ok(entry)).await.is_ok();
    }
    true
}

// ── #EXTM3U header attribute parsing ──────────────────────────────────────────

/// Extracts playlist-level metadata from the `#EXTM3U` header line. Providers
/// embed the XMLTV EPG URL as either `url-tvg="..."` or `x-tvg-url="..."`.
fn parse_header(line: &str) -> Metadata {
    let attrs = parse_kv_attrs(line);
    let url_tvg = attrs
        .get("url-tvg")
        .or_else(|| attrs.get("x-tvg-url"))
        .or_else(|| attrs.get("tvg-url"))
        .cloned()
        .filter(|s| !s.is_empty());
    Metadata { url_tvg }
}

/// Walks a directive line scanning for `key="value"` attribute pairs.
fn parse_kv_attrs(line: &str) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    let mut rest = line;
    while let Some(eq_pos) = rest.find("=\"") {
        let before = &rest[..eq_pos];
        let key_start = before
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let key = before[key_start..].to_lowercase();
        let after_open = &rest[eq_pos + 2..];
        let close = after_open.find('"').unwrap_or(after_open.len());
        let value = after_open[..close].to_string();
        map.insert(key, value);
        rest = &after_open[close.saturating_add(1)..];
    }
    map
}

// ── EXTINF attribute parsing ──────────────────────────────────────────────────

#[derive(Debug)]
struct ExtinfAttrs {
    tvg_id: Option<String>,
    tvg_name: String,
    tvg_logo: Option<String>,
    group_title: Option<String>,
}

fn parse_extinf(line: &str) -> ExtinfAttrs {
    let mut map: HashMap<String, String> = HashMap::new();

    // Walk the line scanning for key="value" pairs
    let mut rest = line;
    while let Some(eq_pos) = rest.find("=\"") {
        // The key is the word immediately before `="`
        let before = &rest[..eq_pos];
        let key_start = before
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let key = before[key_start..].to_lowercase();
        let after_open = &rest[eq_pos + 2..];
        let close = after_open.find('"').unwrap_or(after_open.len());
        let value = after_open[..close].to_string();
        map.insert(key, value);
        rest = &after_open[close.saturating_add(1)..];
    }

    // Display name is everything after the last comma on the line
    let display_name = line
        .rfind(',')
        .map(|i| line[i + 1..].trim().to_string())
        .unwrap_or_default();

    let tvg_name = map
        .remove("tvg-name")
        .filter(|s| !s.is_empty())
        .unwrap_or(display_name);

    let tvg_logo = map.remove("tvg-logo").filter(|s| !s.is_empty());

    // Providers use both "tvg-id" and "tvg-ID"
    let tvg_id = map.remove("tvg-id").filter(|s| !s.is_empty());

    ExtinfAttrs {
        tvg_id,
        tvg_name,
        tvg_logo,
        group_title: map.remove("group-title").filter(|s| !s.is_empty()),
    }
}

fn has_image_extension(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url);
    let lower = path.to_lowercase();
    lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".png")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
}

// ── Classification ────────────────────────────────────────────────────────────

fn classify(attrs: ExtinfAttrs, stream_url: &str) -> ParsedEntry {
    let kind = detect_kind(stream_url, attrs.group_title.as_deref());

    // Validate logo URL regardless of how attrs were constructed
    let logo_url = attrs.tvg_logo.filter(|u| has_image_extension(u));

    let (country_prefix, stripped) = strip_country_prefix(&attrs.tvg_name);
    let stripped = stripped.trim_start_matches('-').trim().to_string();

    let (clean_title, year, series_title, season, episode) = match kind {
        MediaKind::Movie => {
            let (title, yr) = extract_year(&stripped);
            (title, yr, None, None, None)
        }
        MediaKind::Series => {
            let (base, s, e) = extract_season_episode(&stripped);
            let series = base.trim_end_matches('-').trim().to_string();
            (series.clone(), None, Some(series), s, e)
        }
        MediaKind::Live => (stripped.clone(), None, None, None, None),
    };

    let group_title = attrs.group_title.unwrap_or_default();
    let category = normalize_category(&group_title);

    ParsedEntry {
        kind,
        name: attrs.tvg_name,
        group_title,
        tvg_id: attrs.tvg_id,
        logo_url,
        stream_url: stream_url.to_string(),
        country_prefix,
        clean_title,
        year,
        series_title,
        season,
        episode,
        category: Some(category),
    }
}

fn detect_kind(url: &str, group_title: Option<&str>) -> MediaKind {
    if url.contains("/live/") {
        return MediaKind::Live;
    }
    if url.contains("/movie/") {
        return MediaKind::Movie;
    }
    if url.contains("/series/") {
        return MediaKind::Series;
    }
    if let Some(g) = group_title {
        let g = g.to_uppercase();
        if g.contains("SRS") || g.contains("SERIES") {
            return MediaKind::Series;
        }
        if g.contains("MOVIE") || g.contains("VOD") || g.contains("FILM") {
            return MediaKind::Movie;
        }
    }
    MediaKind::Live
}

/// Strips prefix like `"DE| "`, `"FR| "`, `"EU| "`, `"SRS-FR| "` from a name.
fn strip_country_prefix(name: &str) -> (Option<String>, String) {
    if let Some(pipe_pos) = name.find("| ") {
        let prefix = &name[..pipe_pos];
        let code = prefix.trim_start_matches("SRS-");
        if code.len() >= 2 && code.len() <= 3 && code.chars().all(|c| c.is_ascii_uppercase()) {
            return (Some(code.to_string()), name[pipe_pos + 2..].to_string());
        }
    }
    (None, name.to_string())
}

/// Extracts trailing `(YYYY)` year from a movie title.
fn extract_year(title: &str) -> (String, Option<u16>) {
    if let Some(open) = title.rfind('(') {
        let after = &title[open + 1..];
        if let Some(close) = after.find(')') {
            if let Ok(y) = after[..close].parse::<u16>() {
                return (title[..open].trim().to_string(), Some(y));
            }
        }
    }
    (title.to_string(), None)
}

/// Extracts `S{nn} E{nn}` or `S{nn}E{nn}` suffix from a series episode name.
fn extract_season_episode(title: &str) -> (String, Option<u8>, Option<u8>) {
    if let Some((pos, s, e)) = find_season_episode(title) {
        return (title[..pos].trim().to_string(), Some(s), Some(e));
    }
    (title.to_string(), None, None)
}

fn find_season_episode(title: &str) -> Option<(usize, u8, u8)> {
    let upper = title.to_uppercase();
    let chars: Vec<char> = upper.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == 'S' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            let start = i;
            i += 1;
            let mut s_str = String::new();
            while i < chars.len() && chars[i].is_ascii_digit() {
                s_str.push(chars[i]);
                i += 1;
            }
            // Optional space between S and E
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            if i < chars.len()
                && chars[i] == 'E'
                && i + 1 < chars.len()
                && chars[i + 1].is_ascii_digit()
            {
                i += 1;
                let mut e_str = String::new();
                while i < chars.len() && chars[i].is_ascii_digit() {
                    e_str.push(chars[i]);
                    i += 1;
                }
                if let (Ok(s), Ok(e)) = (s_str.parse::<u8>(), e_str.parse::<u8>()) {
                    return Some((start, s, e));
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_live() -> (ExtinfAttrs, &'static str) {
        (
            ExtinfAttrs {
                tvg_id: Some("novo19.fr".into()),
                tvg_name: "FR - NOVO 19".into(),
                tvg_logo: Some("http://logo.123sat.net/logo2026/logos/92.png".into()),
                group_title: Some("EU| FRANCE GENERAL".into()),
            },
            "http://example.com:2082/live/u/p/15562.m3u8",
        )
    }

    fn make_movie_with_logo() -> (ExtinfAttrs, &'static str) {
        (
            ExtinfAttrs {
                tvg_id: None,
                tvg_name: "DE| Beverly Hills Cop: Axel F (2024)".into(),
                tvg_logo: Some("https://image.tmdb.org/t/p/w600_and_h900_bestv2/zszRKfzjM5jltiq8rk6rasKVpUv.jpg".into()),
                group_title: Some("DE| GERMANY MOVIES".into()),
            },
            "http://example.com:2082/movie/u/p/221233.mp4",
        )
    }

    fn make_movie_no_logo() -> (ExtinfAttrs, &'static str) {
        (
            ExtinfAttrs {
                tvg_id: None,
                tvg_name: "DE| Art of Love (2024)".into(),
                tvg_logo: Some("https://image.tmdb.org/t/p/w600_and_h900_bestv2".into()),
                group_title: Some("DE| GERMANY MOVIES".into()),
            },
            "http://example.com:2082/movie/u/p/200733.mp4",
        )
    }

    fn make_series() -> (ExtinfAttrs, &'static str) {
        (
            ExtinfAttrs {
                tvg_id: None,
                tvg_name: "FR| -Mon Comeback S01 E04".into(),
                tvg_logo: Some(
                    "https://image.tmdb.org/t/p/w300/r7Aexe8gf3Aztpj5NitjbR0mzdP.jpg".into(),
                ),
                group_title: Some("SRS-FR| HBO".into()),
            },
            "http://example.com:2082/series/u/p/293257.mkv",
        )
    }

    #[test]
    fn live_kind_and_no_year() {
        let (attrs, url) = make_live();
        let e = classify(attrs, url);
        assert_eq!(e.kind, MediaKind::Live);
        assert!(e.year.is_none());
        assert!(e.series_title.is_none());
    }

    #[test]
    fn movie_kind_year_country_logo() {
        let (attrs, url) = make_movie_with_logo();
        let e = classify(attrs, url);
        assert_eq!(e.kind, MediaKind::Movie);
        assert_eq!(e.country_prefix.as_deref(), Some("DE"));
        assert_eq!(e.clean_title, "Beverly Hills Cop: Axel F");
        assert_eq!(e.year, Some(2024));
        assert!(e.logo_url.is_some());
    }

    #[test]
    fn incomplete_tmdb_logo_becomes_none() {
        let (attrs, url) = make_movie_no_logo();
        let e = classify(attrs, url);
        assert!(e.logo_url.is_none(), "bare TMDB base URL must be rejected");
    }

    #[test]
    fn series_season_episode_parsed() {
        let (attrs, url) = make_series();
        let e = classify(attrs, url);
        assert_eq!(e.kind, MediaKind::Series);
        assert_eq!(e.country_prefix.as_deref(), Some("FR"));
        assert_eq!(e.series_title.as_deref(), Some("Mon Comeback"));
        assert_eq!(e.season, Some(1));
        assert_eq!(e.episode, Some(4));
    }

    #[test]
    fn series_season2() {
        let e = classify(
            ExtinfAttrs {
                tvg_id: None,
                tvg_name: "FR| -Mon Comeback S02 E01".into(),
                tvg_logo: None,
                group_title: Some("SRS-FR| HBO".into()),
            },
            "http://example.com:2082/series/u/p/293224.mkv",
        );
        assert_eq!(e.season, Some(2));
        assert_eq!(e.episode, Some(1));
    }

    #[test]
    fn extinf_attrs_parsed_correctly() {
        let line = r#"#EXTINF:-1 tvg-ID="novo19.fr" tvg-name="FR - NOVO 19" tvg-logo="http://logo.123sat.net/logo2026/logos/92.png" group-title="EU| FRANCE GENERAL",FR - NOVO 19"#;
        let a = parse_extinf(line);
        assert_eq!(a.tvg_id.as_deref(), Some("novo19.fr"));
        assert_eq!(a.tvg_name, "FR - NOVO 19");
        assert!(a.tvg_logo.is_some());
        assert_eq!(a.group_title.as_deref(), Some("EU| FRANCE GENERAL"));
    }

    #[test]
    fn strip_srs_prefix() {
        let (c, rest) = strip_country_prefix("SRS-FR| HBO");
        assert_eq!(c.as_deref(), Some("FR"));
        assert_eq!(rest, "HBO");
    }

    #[test]
    fn strip_eu_prefix() {
        let (c, rest) = strip_country_prefix("EU| FRANCE GENERAL");
        assert_eq!(c.as_deref(), Some("EU"));
        assert_eq!(rest, "FRANCE GENERAL");
    }

    #[test]
    fn no_prefix_passthrough() {
        let (c, rest) = strip_country_prefix("CNN International");
        assert!(c.is_none());
        assert_eq!(rest, "CNN International");
    }

    #[test]
    fn category_normalized_via_classifier() {
        // Categories now flow through nova_core::normalize_category.
        assert_eq!(normalize_category("DE| GERMANY MOVIES"), "Movies");
        assert_eq!(normalize_category("SRS-FR| HBO"), "HBO");
        assert_eq!(normalize_category("PLAIN"), "Plain");
    }

    #[test]
    fn header_url_tvg_extracted() {
        let line = r#"#EXTM3U url-tvg="http://provider.com/xmltv.php" x-tvg-url="http://other.com/epg.xml""#;
        let m = parse_header(line);
        // Prefer url-tvg over x-tvg-url when both are present.
        assert_eq!(m.url_tvg.as_deref(), Some("http://provider.com/xmltv.php"));
    }

    #[test]
    fn header_falls_back_to_x_tvg_url() {
        let line = r#"#EXTM3U x-tvg-url="http://only.com/epg.xml""#;
        let m = parse_header(line);
        assert_eq!(m.url_tvg.as_deref(), Some("http://only.com/epg.xml"));
    }

    #[test]
    fn header_with_no_attrs_is_empty() {
        let m = parse_header("#EXTM3U");
        assert!(m.url_tvg.is_none());
    }
}
