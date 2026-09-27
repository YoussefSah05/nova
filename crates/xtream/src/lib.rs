//! Xtream Codes JSON API client.
//!
//! Uses the `/player_api.php` endpoints instead of downloading the full M3U,
//! which lets live channels, VOD and series be fetched **in parallel** so the
//! total import time drops from 30–120 s to ~5–10 s.
//!
//! Each endpoint is streamed via `chunk()` → blocking `serde_json::Deserializer`
//! so entries are emitted as bytes arrive — no buffering of the full response.

use nova_core::clean::{parse_source_title, TitleKind, PARSER_VERSION};
use nova_core::{normalize_category, MediaKind, ParsedEntry};
use serde::de::{DeserializeOwned, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::Value;
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
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }
        let base_url = parsed.origin().ascii_serialization();
        Some(XtreamCredentials {
            base_url,
            username: username?,
            password: password?,
        })
    }

    fn api_url(&self, action: &str) -> String {
        format!(
            "{}/player_api.php?username={}&password={}&action={}",
            self.base_url,
            url_component(&self.username),
            url_component(&self.password),
            action
        )
    }

    fn live_stream_url(&self, id: i64) -> String {
        format!(
            "{}/live/{}/{}/{}.m3u8",
            self.base_url,
            url_component(&self.username),
            url_component(&self.password),
            id
        )
    }

    fn vod_stream_url(&self, id: i64, ext: &str) -> String {
        let ext = if ext.is_empty() { "mp4" } else { ext };
        format!(
            "{}/movie/{}/{}/{}.{}",
            self.base_url,
            url_component(&self.username),
            url_component(&self.password),
            id,
            url_component(ext)
        )
    }

    /// Stream URL for a single series episode. Xtream serves episodes under
    /// `/series/` keyed by the episode's own stream id (not the series id).
    pub fn series_episode_url(&self, episode_id: &str, ext: &str) -> String {
        let ext = if ext.is_empty() { "mp4" } else { ext };
        format!(
            "{}/series/{}/{}/{}.{}",
            self.base_url,
            url_component(&self.username),
            url_component(&self.password),
            url_component(episode_id),
            url_component(ext)
        )
    }

    /// Returns the XMLTV EPG feed URL for this provider. Every Xtream provider
    /// exposes `/xmltv.php` using the same credentials as the JSON API.
    pub fn xmltv_url(&self) -> String {
        format!(
            "{}/xmltv.php?username={}&password={}",
            self.base_url,
            url_component(&self.username),
            url_component(&self.password)
        )
    }
}

// Encode credentials as individual query values / path segments.
fn url_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Returns `true` when the URL looks like an Xtream Codes API endpoint.
pub fn detect(url: &str) -> bool {
    url.contains("/get.php?") && url.contains("username=") && url.contains("password=")
}

/// One episode of a series, resolved from `get_series_info`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XtreamEpisode {
    pub season: i64,
    pub episode: i64,
    pub title: Option<String>,
    pub stream_url: String,
    pub still_url: Option<String>,
}

/// Fetch the full season/episode list for one series via the `get_series_info`
/// action. Unlike the catalog endpoints this is a single small object response,
/// so we buffer + parse it directly rather than streaming. Called lazily when
/// the user opens a series (see `list_series_episodes`).
pub async fn fetch_series_info(
    creds: &XtreamCredentials,
    series_id: i64,
) -> Result<Vec<XtreamEpisode>, String> {
    let url = format!(
        "{}&series_id={}",
        creds.api_url("get_series_info"),
        series_id
    );
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(45))
        .user_agent("Mozilla/5.0 (compatible; nova-iptv/1.0)")
        .build()
        .map_err(|e| reqwest_error_summary(&e).to_string())?;

    let resp = client
        .get(&url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|e| reqwest_error_summary(&e).to_string())?;
    if !resp.status().is_success() {
        return Err(format!("get_series_info: HTTP {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| reqwest_error_summary(&e).to_string())?;
    Ok(parse_series_info(&body, creds))
}

/// Pure parser for a `get_series_info` body. The `episodes` field is normally an
/// object keyed by season number (`{"1":[…],"2":[…]}`); some providers send a
/// flat array, and ids/numbers arrive as either strings or numbers — all
/// tolerated here. Factored out so it can be unit-tested without the network.
fn parse_series_info(body: &Value, creds: &XtreamCredentials) -> Vec<XtreamEpisode> {
    let mut out = Vec::new();
    let episodes = &body["episodes"];
    if let Some(map) = episodes.as_object() {
        for (season_key, arr) in map {
            let season_from_key = season_key.parse::<i64>().ok();
            if let Some(list) = arr.as_array() {
                for ep in list {
                    push_episode(ep, season_from_key, creds, &mut out);
                }
            }
        }
    } else if let Some(list) = episodes.as_array() {
        for ep in list {
            push_episode(ep, None, creds, &mut out);
        }
    }
    out.sort_by(|a, b| a.season.cmp(&b.season).then(a.episode.cmp(&b.episode)));
    out
}

fn push_episode(
    ep: &Value,
    season_from_key: Option<i64>,
    creds: &XtreamCredentials,
    out: &mut Vec<XtreamEpisode>,
) {
    let Some(id) = value_as_string(&ep["id"]) else {
        return;
    };
    let ext = ep["container_extension"].as_str().unwrap_or("mp4");
    let episode = value_as_i64(&ep["episode_num"]).unwrap_or(0);
    let season = season_from_key
        .or_else(|| value_as_i64(&ep["season"]))
        .unwrap_or(1);
    let title = ep["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let still_url = ep["info"]["movie_image"]
        .as_str()
        .filter(|s| has_image_extension(s))
        .map(str::to_owned);
    out.push(XtreamEpisode {
        season,
        episode,
        title,
        stream_url: creds.series_episode_url(&id, ext),
        still_url,
    });
}

fn value_as_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn value_as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
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
    let count = serde::Deserializer::deserialize_seq(
        &mut de,
        ArrayVisitor {
            convert,
            send,
            _t: std::marker::PhantomData,
        },
    )
    .map_err(json_error_summary)?;
    de.end().map_err(json_error_summary)?;
    Ok(count)
}

fn json_error_summary(error: serde_json::Error) -> String {
    // serde can quote invalid field values; never echo provider payloads.
    format!(
        "invalid provider JSON at line {} column {}",
        error.line(),
        error.column()
    )
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
    /// Panels send a number, a numeric string, `""` or `null`.
    #[serde(default)]
    tmdb: Option<serde_json::Value>,
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
    let parsed = parse_source_title(&s.name, TitleKind::Live);
    let (quality_hints, language_hints) =
        (parsed.quality_hints.clone(), parsed.language_hints.clone());
    let (country_prefix, clean_title) = (parsed.country_prefix, parsed.title);
    let logo_url = s
        .stream_icon
        .as_deref()
        .filter(|u| has_image_extension(u))
        .map(str::to_owned);
    let group_title = s.category_name.unwrap_or_default();
    let category = normalize_category(&group_title);
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
        upstream_id: None,
        provider_tmdb_id: None,
        quality_hints,
        language_hints,
        parser_version: Some(PARSER_VERSION),
    }
}

fn to_vod_entry(s: VodStreamJson, creds: &XtreamCredentials) -> ParsedEntry {
    let parsed = parse_source_title(&s.name, TitleKind::Movie);
    let (quality_hints, language_hints) =
        (parsed.quality_hints.clone(), parsed.language_hints.clone());
    let (country_prefix, clean_title, year) = (parsed.country_prefix, parsed.title, parsed.year);
    let logo_url = s
        .stream_icon
        .as_deref()
        .filter(|u| has_image_extension(u))
        .map(str::to_owned);
    let ext = s.container_extension.as_deref().unwrap_or("mp4");
    let group_title = s.category_name.unwrap_or_default();
    let category = normalize_category(&group_title);
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
        upstream_id: Some(format!("xtream:vod:{}", s.stream_id)),
        provider_tmdb_id: s.tmdb.as_ref().and_then(parse_tmdb_id),
        quality_hints,
        language_hints,
        parser_version: Some(PARSER_VERSION),
    }
}

fn to_series_entry(s: SeriesJson, _creds: &XtreamCredentials) -> ParsedEntry {
    let parsed = parse_source_title(&s.name, TitleKind::Series);
    let (quality_hints, language_hints) =
        (parsed.quality_hints.clone(), parsed.language_hints.clone());
    let (country_prefix, clean_title) = (parsed.country_prefix, parsed.title);
    let logo_url = s
        .cover
        .as_deref()
        .filter(|u| has_image_extension(u))
        .map(str::to_owned);
    let group_title = s.category_name.unwrap_or_default();
    let category = normalize_category(&group_title);
    // Catalog sentinel: one row per series carrying the series_id in its URL.
    // Real seasons/episodes are fetched lazily via `get_series_info` the first
    // time the user opens the series (see `list_series_episodes`). season/episode
    // 0 marks it unfetched so series listings exclude it from episode counts.
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
        season: Some(0),
        episode: Some(0),
        category: Some(category),
        upstream_id: None,
        provider_tmdb_id: None,
        quality_hints,
        language_hints,
        parser_version: Some(PARSER_VERSION),
    }
}

// ── Local helpers ─────────────────────────────────────────────────────────────

/// A positive TMDB ID from a provider's loosely typed `tmdb` field. Zero,
/// empty and non-numeric values (common placeholders) are no evidence.
fn parse_tmdb_id(value: &serde_json::Value) -> Option<i64> {
    let id = match value {
        serde_json::Value::Number(n) => n.as_i64()?,
        serde_json::Value::String(s) => s.trim().parse::<i64>().ok()?,
        _ => return None,
    };
    (1..=999_999_999).contains(&id).then_some(id)
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
        // "FRANCE GENERAL" normalizes to the canonical Entertainment genre.
        assert_eq!(e.category.as_deref(), Some("Entertainment"));
    }

    #[test]
    fn vod_names_with_spaced_prefixes_and_in_title_numbers() {
        let creds = XtreamCredentials {
            base_url: "http://h:80".into(),
            username: "u".into(),
            password: "p".into(),
        };
        let vod = |name: &str| VodStreamJson {
            stream_id: 1,
            name: name.into(),
            stream_icon: None,
            category_name: None,
            container_extension: None,
            tmdb: None,
        };
        let e = to_vod_entry(vod("FR | Blade Runner 2049 (2017) 4K"), &creds);
        assert_eq!(e.clean_title, "Blade Runner 2049");
        assert_eq!(
            (e.year, e.country_prefix.as_deref()),
            (Some(2017), Some("FR"))
        );
        assert_eq!(e.name, "FR | Blade Runner 2049 (2017) 4K", "raw kept");
        let e = to_vod_entry(vod("UK: The Office (US)"), &creds);
        assert_eq!(e.clean_title, "The Office (US)");
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
            tmdb: Some(serde_json::json!("280180")),
        };
        let e = to_vod_entry(s, &creds);
        assert_eq!(e.kind, MediaKind::Movie);
        assert_eq!(e.clean_title, "Beverly Hills Cop: Axel F");
        assert_eq!(e.year, Some(2024));
        assert!(e.logo_url.is_some());
        assert_eq!(e.provider_tmdb_id, Some(280180));
    }

    #[test]
    fn provider_tmdb_ids_accept_only_positive_numbers() {
        use serde_json::json;
        assert_eq!(parse_tmdb_id(&json!(603)), Some(603));
        assert_eq!(parse_tmdb_id(&json!(" 603 ")), Some(603));
        for placeholder in [
            json!(0),
            json!("0"),
            json!(""),
            json!(null),
            json!(-5),
            json!("tt0133093"),
            json!(1.5),
        ] {
            assert_eq!(parse_tmdb_id(&placeholder), None, "{placeholder}");
        }
        let parsed: VodStreamJson =
            serde_json::from_value(json!({ "stream_id": 1, "name": "X", "tmdb": "" })).unwrap();
        assert_eq!(parsed.tmdb.as_ref().and_then(parse_tmdb_id), None);
        let parsed: VodStreamJson =
            serde_json::from_value(json!({ "stream_id": 1, "name": "X" })).unwrap();
        assert!(parsed.tmdb.is_none());
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

    fn test_creds() -> XtreamCredentials {
        XtreamCredentials {
            base_url: "http://h:80".into(),
            username: "u".into(),
            password: "p".into(),
        }
    }

    #[test]
    fn series_episode_url_correct() {
        let creds = test_creds();
        assert_eq!(
            creds.series_episode_url("293257", "mkv"),
            "http://h:80/series/u/p/293257.mkv"
        );
        // Empty extension falls back to mp4.
        assert_eq!(
            creds.series_episode_url("1", ""),
            "http://h:80/series/u/p/1.mp4"
        );
    }

    #[test]
    fn parse_series_info_object_keyed_seasons() {
        let body = serde_json::json!({
            "episodes": {
                "1": [
                    { "id": "101", "episode_num": 1, "title": "Pilot",
                      "container_extension": "mkv",
                      "info": { "movie_image": "http://img/1.jpg" } },
                    { "id": 102, "episode_num": "2", "title": "",
                      "container_extension": "mp4" }
                ],
                "2": [
                    { "id": "201", "episode_num": 1, "container_extension": "mkv" }
                ]
            }
        });
        let eps = parse_series_info(&body, &test_creds());
        assert_eq!(eps.len(), 3);
        // Sorted by season then episode.
        assert_eq!((eps[0].season, eps[0].episode), (1, 1));
        assert_eq!(eps[0].title.as_deref(), Some("Pilot"));
        assert_eq!(eps[0].stream_url, "http://h:80/series/u/p/101.mkv");
        assert_eq!(eps[0].still_url.as_deref(), Some("http://img/1.jpg"));
        // string episode_num + numeric id tolerated; empty title -> None.
        assert_eq!((eps[1].season, eps[1].episode), (1, 2));
        assert_eq!(eps[1].title, None);
        assert_eq!(eps[1].stream_url, "http://h:80/series/u/p/102.mp4");
        assert_eq!((eps[2].season, eps[2].episode), (2, 1));
    }

    #[test]
    fn parse_series_info_handles_empty_or_missing() {
        // Some providers send an empty array instead of an object.
        let body = serde_json::json!({ "episodes": [] });
        assert!(parse_series_info(&body, &test_creds()).is_empty());
        // Missing episodes key entirely.
        let body = serde_json::json!({ "info": {} });
        assert!(parse_series_info(&body, &test_creds()).is_empty());
    }

    #[test]
    fn parse_series_info_season_from_episode_field_when_flat_array() {
        let body = serde_json::json!({
            "episodes": [
                { "id": "9", "episode_num": 3, "season": 4, "container_extension": "ts" }
            ]
        });
        let eps = parse_series_info(&body, &test_creds());
        assert_eq!(eps.len(), 1);
        assert_eq!((eps[0].season, eps[0].episode), (4, 3));
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
                upstream_id: None,
                provider_tmdb_id: None,
                quality_hints: Vec::new(),
                language_hints: Vec::new(),
                parser_version: None,
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
    #[test]
    fn credentials_preserve_reserved_characters_as_single_components() {
        let c = XtreamCredentials {
            base_url: "https://example.invalid".into(),
            username: "fixture +&".into(),
            password: "fixture/#?%".into(),
        };
        for value in [c.api_url("get_live_streams"), c.xmltv_url()] {
            let url = reqwest::Url::parse(&value).unwrap();
            let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
            assert!(pairs.get("username").is_some_and(|v| v == &c.username));
            assert!(pairs.get("password").is_some_and(|v| v == &c.password));
            assert!(url.fragment().is_none());
        }
        for value in [
            c.live_stream_url(1),
            c.vod_stream_url(1, "mp4"),
            c.series_episode_url("1", "mp4"),
        ] {
            let url = reqwest::Url::parse(&value).unwrap();
            assert_eq!(url.path_segments().unwrap().count(), 4);
            assert!(url.query().is_none() && url.fragment().is_none());
        }
    }

    #[test]
    fn streaming_parser_rejects_trailing_data_without_echoing_values() {
        for input in [r#"[] trailing"#, r#"["sensitive-fixture-value"]"#] {
            let result =
                parse_json_array::<u32, _, _>(input.as_bytes(), |_| unreachable!(), |_| true);
            assert!(result.is_err());
            assert!(!result.unwrap_err().contains("sensitive-fixture-value"));
        }
    }
}
