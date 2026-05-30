//! Local HLS reverse proxy built on Axum.
//!
//! Phase 5 surface:
//! - Passthrough `/proxy?url=...` route (HLS segments after manifest rewriting
//!   and direct-file streams come through here).
//! - Session-based `/stream/{token}/master.m3u8` + `/stream/{token}/local/...`
//!   routes. Sessions are created via [`SessionRegistry::register`]; the
//!   proxy routes the upstream and either rewrites the live manifest or
//!   serves files from an ffmpeg-fed temp dir.
//! - `/captions/{token}/...` stub routes that 404 today and will be populated
//!   by Phase 6 (source captions) and Phase 7 (translated captions) without
//!   touching the player.
//!
//! Why a proxy at all in Phase 5: subsequent phases need a stable point to
//! rewrite manifests + serve generated VTTs from. Standing the proxy up now
//! means the player URL contract doesn't change later.
//!
//! See `docs/reference/hls-proxy.md` for the manifest rewriting rules.

mod manifest;
mod routes;
mod session;

use std::time::Duration;

use axum::routing::get;
use axum::Router;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub use manifest::{inject_subtitles, rewrite_manifest, SubtitleTrack};
pub use nova_media::PlaybackRoute;
pub use session::{PlaybackKind, SessionError, SessionInfo, SessionRegistry};

// -- Error ---------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP client error: {0}")]
    Client(#[from] reqwest::Error),
}

// -- Handle --------------------------------------------------------------------

/// Handle returned by [`start`]. Holding it keeps the proxy alive; dropping
/// it signals graceful shutdown.
pub struct ProxyHandle {
    pub port: u16,
    sessions: SessionRegistry,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ProxyHandle {
    /// Origin string the proxy uses in rewritten URLs.
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Access the registry so callers can create / release sessions. This is
    /// what the Tauri `resolve_stream` command holds onto.
    pub fn sessions(&self) -> &SessionRegistry {
        &self.sessions
    }

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

// -- Server boot ---------------------------------------------------------------

/// Boot the proxy on `127.0.0.1:0` (OS picks a free port). Returns once
/// the listener is bound - the actual serve loop runs in a spawned task.
pub async fn start() -> Result<ProxyHandle, ProxyError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent("Mozilla/5.0 (compatible; nova-iptv/1.0)")
        .build()?;

    let sessions = SessionRegistry::new();
    let state = routes::AppState {
        client,
        sessions: sessions.clone(),
        origin: format!("http://127.0.0.1:{port}"),
    };

    let app = Router::new()
        .route("/health", get(routes::health))
        .route("/proxy", get(routes::proxy))
        .route("/stream/{token}/master.m3u8", get(routes::stream_master))
        .route("/stream/{token}/local/{file}", get(routes::stream_local))
        // Both "source-only" and "translated" caption URLs share the same
        // {spec} segment. Spec is `<lang>.vtt` for source captions and
        // `<src>-to-<tgt>.vtt` for translated; the handler parses the form.
        // Axum forbids two `{}` captures in one segment, so the split lives
        // inside the handler.
        .route("/captions/{token}/{spec}", get(routes::captions_stub))
        .with_state(state);

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
        sessions,
        shutdown: Some(tx),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    use crate::routes::{is_hls_manifest_response, is_safe_upstream};

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

    #[test]
    fn detects_hls_manifest_proxy_responses() {
        assert!(is_hls_manifest_response(
            "https://provider.com/path/master.m3u8?token=abc",
            None,
        ));
        assert!(is_hls_manifest_response(
            "https://provider.com/path/master",
            Some(&HeaderValue::from_static("application/vnd.apple.mpegurl")),
        ));
        assert!(!is_hls_manifest_response(
            "https://provider.com/path/segment.ts",
            Some(&HeaderValue::from_static("video/mp2t")),
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

    #[tokio::test]
    async fn captions_stub_returns_404() {
        let handle = start().await.unwrap();
        for path in &[
            format!("http://127.0.0.1:{}/captions/abc/en.vtt", handle.port),
            format!("http://127.0.0.1:{}/captions/abc/en-to-fr.vtt", handle.port),
        ] {
            let res = reqwest::get(path).await.unwrap();
            assert_eq!(res.status(), 404);
        }
        handle.shutdown();
    }
}
