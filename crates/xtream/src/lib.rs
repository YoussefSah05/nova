//! Xtream Codes JSON API client.
//!
//! Uses the `/player_api.php` endpoints instead of downloading the full M3U,
//! which lets live channels, VOD and series be fetched **in parallel** so the
//! total import time drops from 30–120 s to ~5–10 s.
//!
//! Each endpoint is streamed via `chunk()` → blocking `serde_json::Deserializer`
//! so entries are emitted as bytes arrive — no buffering of the full response.

use nova_core::{MediaKind, ParsedEntry};
use serde::de::{DeserializeOwned, SeqAccess, Visitor};
use serde::Deserialize;
use std::time::Instant;
use tokio::sync::mpsc;

// ── Public types ──────────────────────────────────────────────────────────────

/// Credentials parsed from an Xtream `/get.php?username=…&password=…` URL.
#[derive(Debug, Clone)]
pub struct XtreamCredentials {
    pub base_url: String,
    pub username: String,
    pub password: String,
}

impl XtreamCredentials {
    /// Parse credentials from a `/get.php` URL.
    /// Returns `None` if the URL does not contain the required query params.
    pub fn from_url(url: &str) -> Option<Self> {
        let parsed = reqwest::Url::parse(url).ok()?;
        let mut username = None;
        let mut password = None;
        for (k, v) in parsed.query_pairs() {
            match k.as_ref() {
                "username" => username = Some(v.into_owned()),
                "password" => password = Some(v.into_owned()),
                _ => {}
            }
        }
        let base_url = format!("{}://{}", parsed.scheme(), parsed.host_str()?);
        let base_url = if let Some(port) = parsed.port() {
            format!("{base_url}:{port}")
        } else {
            base_url
        };
        Some(XtreamCredentials {
            base_url,
            username: username?,
            password: password?,
        })
    }

    fn api_url(&self, action: &str) -> String {
        format!(
            "{}/player_api.php?username={}&password={}&action={}",
            self.base_url, self.username, self.password, action
        )
    }

    fn live_stream_url(&self, id: i64) -> String {
        format!(
            "{}/live/{}/{}/{}.m3u8",
            self.base_url, self.username, self.password, id
        )
    }

    fn vod_stream_url(&self, id: i64, ext: &str) -> String {
        let ext = if ext.is_empty() { "mp4" } else { ext };
        format!(
            "{}/movie/{}/{}/{}.{}",
            self.base_url, self.username, self.password, id, ext
        )
    }

    /// Returns the XMLTV EPG feed URL for this provider. Every Xtream provider
    /// exposes `/xmltv.php` using the same credentials as the JSON API.
    pub fn xmltv_url(&self) -> String {
        format!(
            "{}/xmltv.php?username={}&password={}",
            self.base_url, self.username, self.password
        )
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Returns `true` when the URL looks like an Xtream Codes API endpoint.
pub fn detect(url: &str) -> bool {
    url.contains("/get.php?") && url.contains("username=") && url.contains("password=")
}

/// Fetch all content from an Xtream provider via the JSON API.
///
/// Live channels, VOD and series are fetched in three **independent** parallel
/// tasks. Each task streams the HTTP body and emits entries as they are parsed —
/// the consumer sees entries from whichever endpoint responds first without
/// waiting for the others to finish.
pub async fn fetch(creds: &XtreamCredentials) -> mpsc::Receiver<Result<ParsedEntry, String>> {
    let (tx, rx) = mpsc::channel(512);
    let creds = creds.clone();

    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            // Stall detector: abort an endpoint if no bytes arrive for 30s.
            // Without this a provider that accepts the connection but never
            // streams (common on get_series) hangs the whole import forever.
            .read_timeout(std::time::Duration::from_secs(30))
            // Total-request backstop, mirroring the M3U path's curl --max-time.
            .timeout(std::time::Duration::from_secs(300))
            .user_agent("Mozilla/5.0 (compatible; nova-iptv/1.0)")
            .no_gzip()
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Err(reqwest_error_summary(&e).to_string())).await;
                return;
            }
        };

        let (tx1, tx2, tx3) = (tx.clone(), tx.clone(), tx.clone());
        let (c1, c2, c3) = (creds.clone(), creds.clone(), creds.clone());
        let (cl1, cl2, cl3) = (client.clone(), client.clone(), client.clone());

        tracing::info!(base_url = %creds.base_url, "Xtream fetch starting (3 parallel endpoints)");

        let live_h = tokio::spawn(async move {
            let url = c1.api_url("get_live_streams");
            tracing::info!(
                action = "get_live_streams",
                "Xtream endpoint request starting"
            );
            fetch_stream::<LiveStreamJson, _>(
                &cl1,
                url,
                tx1,
                move |s| to_live_entry(s, &c1),
                "live",
            )
            .await;
        });
        let vod_h = tokio::spawn(async move {
            let url = c2.api_url("get_vod_streams");
            tracing::info!(
                action = "get_vod_streams",
                "Xtream endpoint request starting"
            );
            fetch_stream::<VodStreamJson, _>(&cl2, url, tx2, move |s| to_vod_entry(s, &c2), "vod")
                .await;
        });
        let series_h = tokio::spawn(async move {
            let url = c3.api_url("get_series");
            tracing::info!(action = "get_series", "Xtream endpoint request starting");
            fetch_stream::<SeriesJson, _>(
                &cl3,
                url,
                tx3,
                move |s| to_series_entry(s, &c3),
                "series",
            )
            .await;
        });

        let _ = tokio::join!(live_h, vod_h, series_h);
    });

    rx
}

// ── Streaming HTTP → JSON → entries ──────────────────────────────────────────

/// Stream a JSON array from `url`, emitting one `ParsedEntry` per element as
/// bytes arrive — without buffering the full response.
///
/// Internally: the async task pumps `resp.chunk()` bytes into a bounded
/// `tokio::sync::mpsc` channel; a `spawn_blocking` worker reads from that
/// channel via `ChunkedReader` (a `std::io::Read` impl) and drives
/// `serde_json::Deserializer::deserialize_seq`, which calls `next_element()`
/// one at a time. Each element is converted and sent to `out_tx` via
/// `blocking_send`.
async fn fetch_stream<T, F>(
    client: &reqwest::Client,
    url: String,
    out_tx: mpsc::Sender<Result<ParsedEntry, String>>,
    convert: F,
    label: &'static str,
) where
    T: DeserializeOwned + Send + 'static,
    F: Fn(T) -> ParsedEntry + Send + 'static,
{
    let started_at = Instant::now();
    let mut resp = match client
        .get(&url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            let _ = out_tx
                .send(Err(format!("{label}: HTTP {}", r.status())))
                .await;
            return;
        }
        Err(e) => {
            let _ = out_tx
                .send(Err(format!("{label}: {}", reqwest_error_summary(&e))))
                .await;
            return;
        }
    };

    tracing::debug!("{label}: response received, streaming JSON array");

    // Byte channel: async producer (chunk loop) → blocking consumer (parser).
    let (byte_tx, byte_rx) = mpsc::channel::<Vec<u8>>(8);

    // Blocking task: deserialise the JSON array element by element.
    let out_tx_parse = out_tx.clone();
    let parse_handle = tokio::task::spawn_blocking(move || {
        let reader = ChunkedReader::new(byte_rx);
        parse_json_array::<T, F, _>(reader, convert, |entry| {
            out_tx_parse.blocking_send(Ok(entry)).is_ok()
        })
    });

    // Pump response chunks into the byte channel.
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if byte_tx.send(chunk.to_vec()).await.is_err() {
                    break; // parser stopped (receiver dropped or channel closed)
                }
            }
            Ok(None) => break, // EOF
            Err(e) => {
                let _ = out_tx
                    .send(Err(format!(
                        "{label}: stream error: {}",
                        reqwest_error_summary(&e)
                    )))
                    .await;
                break;
            }
        }
    }
    drop(byte_tx); // Signal EOF to ChunkedReader

    // Surface any JSON parse error.
    match parse_handle.await {
        Ok(Ok(count)) => {
            tracing::info!(
                label,
                count,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "Xtream endpoint complete"
            );
        }
        Ok(Err(e)) => {
            let _ = out_tx.send(Err(format!("{label}: parse error: {e}"))).await;
        }
        Err(e) => {
            let _ = out_tx
                .send(Err(format!("{label}: internal error: {e}")))
                .await;
        }
    }
}

fn reqwest_error_summary(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "request timed out"
    } else if e.is_connect() {
        "connection failed"
    } else if e.is_decode() {
        "invalid response body"
    } else if e.is_body() {
        "response body failed"
    } else {
        "request failed"
    }
}

// ── Sync I/O bridge: tokio mpsc → std::io::Read ───────────────────────────────

/// Wraps a `tokio::sync::mpsc::Receiver<Vec<u8>>` as a `std::io::Read`.
/// Intended for use inside `tokio::task::spawn_blocking`.
struct ChunkedReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl ChunkedReader {
    fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
        }
    }
}

impl std::io::Read for ChunkedReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        // Refill buffer when exhausted.
        while self.pos >= self.buf.len() {
            match self.rx.blocking_recv() {
                Some(chunk) => {
                    self.buf = chunk;
                    self.pos = 0;
                }
                None => return Ok(0), // channel closed = EOF
            }
        }
        let n = (self.buf.len() - self.pos).min(out.len());
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

// ── Streaming JSON array parser ───────────────────────────────────────────────

/// Parse a JSON array from `reader`, calling `send(entry)` for each element.
/// Returns when the array is exhausted or `send` returns `false` (consumer gone).
fn parse_json_array<T, F, G>(reader: impl std::io::Read, convert: F, send: G) -> Result<u64, String>
where
    T: DeserializeOwned,
    F: Fn(T) -> ParsedEntry,
    G: FnMut(ParsedEntry) -> bool,
{
    struct ArrayVisitor<T, F, G> {
        convert: F,
        send: G,
        _t: std::marker::PhantomData<T>,
    }

    impl<'de, T, F, G> Visitor<'de> for ArrayVisitor<T, F, G>
    where
        T: serde::Deserialize<'de>,
        F: Fn(T) -> ParsedEntry,
        G: FnMut(ParsedEntry) -> bool,
    {
        type Value = u64;
        fn expecting(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
            fmt.write_str("a JSON array")
        }
        fn visit_seq<A: SeqAccess<'de>>(mut self, mut seq: A) -> Result<u64, A::Error> {
            let mut count = 0;
            while let Some(item) = seq.next_element::<T>()? {
                if !(self.send)((self.convert)(item)) {
                    break; // consumer gone
                }
                count += 1;
            }
            Ok(count)
        }
    }

    let mut de = serde_json::Deserializer::from_reader(reader);
    serde::Deserializer::deserialize_seq(
        &mut de,
        ArrayVisitor {
            convert,
            send,
            _t: std::marker::PhantomData,
        },
    )
    .map_err(|e| e.to_string())
}

// ── Internal JSON structs ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LiveStreamJson {
    #[serde(default)]
    stream_id: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    stream_icon: Option<String>,
    #[serde(default)]
    epg_channel_id: Option<String>,
    #[serde(default)]
    category_name: Option<String>,
}

#[derive(Deserialize)]
struct VodStreamJson {
    #[serde(default)]
    stream_id: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    stream_icon: Option<String>,
    #[serde(default)]
    category_name: Option<String>,
    #[serde(default)]
    container_extension: Option<String>,
}

#[derive(Deserialize)]
struct SeriesJson {
    #[serde(default)]
    series_id: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    cover: Option<String>,
    #[serde(default)]
    category_name: Option<String>,
}

// ── Entry converters ──────────────────────────────────────────────────────────

fn to_live_entry(s: LiveStreamJson, creds: &XtreamCredentials) -> ParsedEntry {
    let (country_prefix, clean_title) = strip_country_prefix(&s.name);
    let logo_url = s
        .stream_icon
        .as_deref()
        .filter(|u| has_image_extension(u))
        .map(str::to_owned);
    let group_title = s.category_name.unwrap_or_default();
    let category = extract_category(&group_title);
    ParsedEntry {
        kind: MediaKind::Live,
        name: s.name,
        group_title,
        tvg_id: s.epg_channel_id,
        logo_url,
        stream_url: creds.live_stream_url(s.stream_id),
        country_prefix,
        clean_title,
        year: None,
        series_title: None,
        season: None,
        episode: None,
        category: Some(category),
    }
}

fn to_vod_entry(s: VodStreamJson, creds: &XtreamCredentials) -> ParsedEntry {
    let (country_prefix, stripped) = strip_country_prefix(&s.name);
    let (clean_title, year) = extract_year(&stripped);
    let logo_url = s
        .stream_icon
        .as_deref()
        .filter(|u| has_image_extension(u))
        .map(str::to_owned);
    let ext = s.container_extension.as_deref().unwrap_or("mp4");
    let group_title = s.category_name.unwrap_or_default();
    let category = extract_category(&group_title);
    ParsedEntry {
        kind: MediaKind::Movie,
        name: s.name,
        group_title,
        tvg_id: None,
        logo_url,
        stream_url: creds.vod_stream_url(s.stream_id, ext),
        country_prefix,
        clean_title,
        year,
        series_title: None,
        season: None,
        episode: None,
        category: Some(category),
    }
}

fn to_series_entry(s: SeriesJson, _creds: &XtreamCredentials) -> ParsedEntry {
    let (country_prefix, clean_title) = strip_country_prefix(&s.name);
    let logo_url = s
        .cover
        .as_deref()
        .filter(|u| has_image_extension(u))
        .map(str::to_owned);
    let group_title = s.category_name.unwrap_or_default();
    let category = extract_category(&group_title);
    let stream_url = format!("xtream://series/{}", s.series_id);
    ParsedEntry {
        kind: MediaKind::Series,
        name: s.name,
        group_title,
        tvg_id: None,
        logo_url,
        stream_url,
        country_prefix,
        clean_title: clean_title.clone(),
        year: None,
        series_title: Some(clean_title),
        season: Some(1),
        episode: Some(1),
        category: Some(category),
    }
}

// ── Local helpers ─────────────────────────────────────────────────────────────

fn has_image_extension(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url);
    let lower = path.to_lowercase();
    lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".png")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
}

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

fn extract_category(group_title: &str) -> String {
    if let Some(pipe_pos) = group_title.find("| ") {
        group_title[pipe_pos + 2..].trim().to_string()
    } else {
        group_title.trim().to_string()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_xtream_get_php() {
        assert!(detect(
            "http://example.com:8080/get.php?username=foo&password=bar&type=m3u_plus"
        ));
    }

    #[test]
    fn does_not_detect_plain_m3u() {
        assert!(!detect("http://example.com/playlist.m3u"));
    }

    #[test]
    fn credentials_parsed() {
        let creds = XtreamCredentials::from_url(
            "http://provider.example.com:2082/get.php?username=user123&password=pass456&type=m3u_plus",
        )
        .unwrap();
        assert_eq!(creds.base_url, "http://provider.example.com:2082");
        assert_eq!(creds.username, "user123");
        assert_eq!(creds.password, "pass456");
    }

    #[test]
    fn live_stream_url_correct() {
        let creds = XtreamCredentials {
            base_url: "http://host:8080".into(),
            username: "u".into(),
            password: "p".into(),
        };
        assert_eq!(
            creds.live_stream_url(42),
            "http://host:8080/live/u/p/42.m3u8"
        );
    }

    #[test]
    fn xmltv_url_correct() {
        let creds = XtreamCredentials {
            base_url: "http://host:8080".into(),
            username: "u".into(),
            password: "p".into(),
        };
        assert_eq!(
            creds.xmltv_url(),
            "http://host:8080/xmltv.php?username=u&password=p"
        );
    }

    #[test]
    fn vod_stream_url_correct() {
        let creds = XtreamCredentials {
            base_url: "http://host:8080".into(),
            username: "u".into(),
            password: "p".into(),
        };
        assert_eq!(
            creds.vod_stream_url(99, "mp4"),
            "http://host:8080/movie/u/p/99.mp4"
        );
    }

    #[test]
    fn live_entry_converted() {
        let creds = XtreamCredentials {
            base_url: "http://h:80".into(),
            username: "u".into(),
            password: "p".into(),
        };
        let s = LiveStreamJson {
            stream_id: 1,
            name: "FR - NOVO 19".into(),
            stream_icon: Some("http://logo.net/92.png".into()),
            epg_channel_id: Some("novo19.fr".into()),
            category_name: Some("FRANCE GENERAL".into()),
        };
        let e = to_live_entry(s, &creds);
        assert_eq!(e.kind, MediaKind::Live);
        assert_eq!(e.stream_url, "http://h:80/live/u/p/1.m3u8");
        assert_eq!(e.category.as_deref(), Some("FRANCE GENERAL"));
    }

    #[test]
    fn vod_entry_year_stripped() {
        let creds = XtreamCredentials {
            base_url: "http://h:80".into(),
            username: "u".into(),
            password: "p".into(),
        };
        let s = VodStreamJson {
            stream_id: 10,
            name: "Beverly Hills Cop: Axel F (2024)".into(),
            stream_icon: Some("https://image.tmdb.org/t/p/w600/abc.jpg".into()),
            category_name: Some("GERMANY MOVIES".into()),
            container_extension: Some("mp4".into()),
        };
        let e = to_vod_entry(s, &creds);
        assert_eq!(e.kind, MediaKind::Movie);
        assert_eq!(e.clean_title, "Beverly Hills Cop: Axel F");
        assert_eq!(e.year, Some(2024));
        assert!(e.logo_url.is_some());
    }

    #[test]
    fn series_entry_placeholder_url() {
        let creds = XtreamCredentials {
            base_url: "http://h:80".into(),
            username: "u".into(),
            password: "p".into(),
        };
        let s = SeriesJson {
            series_id: 55,
            name: "Mon Comeback".into(),
            cover: None,
            category_name: Some("HBO".into()),
        };
        let e = to_series_entry(s, &creds);
        assert_eq!(e.kind, MediaKind::Series);
        assert_eq!(e.stream_url, "xtream://series/55");
        assert_eq!(e.series_title.as_deref(), Some("Mon Comeback"));
    }

    #[test]
    fn parse_json_array_streams_elements() {
        #[derive(serde::Deserialize)]
        struct Item {
            id: u32,
        }

        let json = b"[{\"id\":1},{\"id\":2},{\"id\":3}]";
        let mut ids = Vec::new();
        let count = parse_json_array::<Item, _, _>(
            json.as_slice(),
            |item| ParsedEntry {
                kind: MediaKind::Live,
                name: item.id.to_string(),
                group_title: String::new(),
                tvg_id: None,
                logo_url: None,
                stream_url: String::new(),
                country_prefix: None,
                clean_title: String::new(),
                year: None,
                series_title: None,
                season: None,
                episode: None,
                category: None,
            },
            |entry| {
                ids.push(entry.name.parse::<u32>().unwrap());
                true
            },
        )
        .unwrap();
        assert_eq!(count, 3);
        assert_eq!(ids, [1, 2, 3]);
    }
}
