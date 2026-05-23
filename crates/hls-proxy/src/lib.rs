//! Local HLS reverse proxy built on Axum.
//!
//! Phase 5 ships a passthrough proxy. Future phases add:
//! - Phase 6: source-language VTT injection into the master playlist.
//! - Phase 7: translated VTT tracks served alongside the manifest.
//! - Phase 9/10: rolling-window VTTs for live captions.
//!
//! Why a proxy at all in Phase 5: subsequent phases need a stable point
//! to rewrite manifests + serve generated VTTs from. Standing the proxy
//! up now means the player URL contract doesn't change later.
//!
//! See `docs/reference/hls-proxy.md` for the manifest rewriting rules
//! that future phases will implement here.

use std::time::Duration;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP client error: {0}")]
    Client(#[from] reqwest::Error),
}

// ── Handle ────────────────────────────────────────────────────────────────────

/// Handle returned by [`start`]. Holding it keeps the proxy alive; dropping
/// it signals graceful shutdown.
pub struct ProxyHandle {
    pub port: u16,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ProxyHandle {
    /// Send the shutdown signal explicitly. The proxy will stop accepting
    /// new connections and drain in-flight requests.
    pub fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

// ── Server boot ───────────────────────────────────────────────────────────────

/// Boot the proxy on `127.0.0.1:0` (OS picks a free port). Returns once
/// the listener is bound — the actual serve loop runs in a spawned task.
pub async fn start() -> Result<ProxyHandle, ProxyError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        // No total timeout — HLS segments can take a while on slow links and
        // the player handles its own buffer-underrun behaviour.
        .user_agent("Mozilla/5.0 (compatible; nova-iptv/1.0)")
        .build()?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/proxy", get(proxy))
        .with_state(client);

    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = rx.await;
        });
        if let Err(e) = serve.await {
            tracing::error!(error = %e, "hls-proxy serve loop terminated");
        }
    });

    tracing::info!(port, "hls-proxy listening on 127.0.0.1");

    Ok(ProxyHandle {
        port,
        shutdown: Some(tx),
    })
}

// ── Routes ────────────────────────────────────────────────────────────────────

async fn health() -> &'static str {
    "ok"
}

#[derive(Debug, Deserialize)]
struct ProxyParams {
    /// Upstream URL to fetch. Caller URL-encodes it.
    url: String,
}

/// Streams the upstream response back to the player. For Phase 5 the body
/// is passed through verbatim; future phases will rewrite manifest bodies
/// before forwarding while keeping segment responses untouched.
async fn proxy(
    State(client): State<reqwest::Client>,
    Query(params): Query<ProxyParams>,
) -> Response {
    if !is_safe_upstream(&params.url) {
        return (StatusCode::BAD_REQUEST, "invalid upstream URL").into_response();
    }

    let upstream = match client.get(&params.url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, url = %params.url, "upstream fetch failed");
            return (StatusCode::BAD_GATEWAY, "upstream fetch failed").into_response();
        }
    };

    let status = upstream.status();
    let mut forwarded_headers = HeaderMap::new();
    for name in &[
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CACHE_CONTROL,
        HeaderName::from_static("accept-ranges"),
    ] {
        if let Some(v) = upstream.headers().get(name) {
            forwarded_headers.insert(name.clone(), v.clone());
        }
    }
    // Always allow CORS — the Tauri webview makes cross-origin requests to
    // 127.0.0.1 from `tauri://localhost` (macOS) / `https://tauri.localhost`
    // (Windows) and needs an explicit OK.
    forwarded_headers.insert(
        HeaderName::from_static("access-control-allow-origin"),
        HeaderValue::from_static("*"),
    );

    let body = Body::from_stream(upstream.bytes_stream());
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = forwarded_headers;
    response
}

/// Reject anything that isn't an `http://` or `https://` URL — defence in
/// depth against `file://` and other schemes being passed through. Also
/// rejects loopback to prevent the proxy being used to scan the local
/// network from a hostile playlist.
fn is_safe_upstream(url: &str) -> bool {
    let parsed = match reqwest::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    if let Some(host) = parsed.host_str() {
        let lower = host.to_ascii_lowercase();
        if lower == "localhost" || lower == "127.0.0.1" || lower == "::1" {
            return false;
        }
    } else {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_scheme() {
        assert!(!is_safe_upstream("file:///etc/passwd"));
        assert!(!is_safe_upstream("ftp://example.com/x"));
    }

    #[test]
    fn rejects_loopback() {
        assert!(!is_safe_upstream("http://127.0.0.1/proxy?url=foo"));
        assert!(!is_safe_upstream("http://localhost:8080/x"));
    }

    #[test]
    fn accepts_normal_urls() {
        assert!(is_safe_upstream("http://example.com/stream.m3u8"));
        assert!(is_safe_upstream(
            "https://provider.com:2082/live/u/p/1.m3u8"
        ));
    }

    #[tokio::test]
    async fn health_endpoint_responds() {
        let handle = start().await.unwrap();
        let url = format!("http://127.0.0.1:{}/health", handle.port);
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
        assert_eq!(body, "ok");
        handle.shutdown();
    }
}
