//! Local HLS reverse proxy built on Axum.
//!
//! Surface (every route is scoped to a session token; no upstream URL is ever
//! in a locator, and nothing a session's manifests didn't name is fetched):
//! - `/stream/{token}/master.m3u8`: the session's manifest, its URIs rewritten
//!   to `/stream/{token}/r/{id}`; or the remux's local manifest.
//! - `/stream/{token}/r/{id}`: a segment, key or sub-playlist the session's
//!   manifests named (sub-playlists rewritten the same way).
//! - `/stream/{token}/file`: a direct-file session's bytes (Range forwarded).
//! - `/stream/{token}/local/{file}`: files ffmpeg writes for a remux.
//!   Sessions are created via [`SessionRegistry::register`].
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
mod resources;
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

// -- DNS -----------------------------------------------------------------------

/// Resolves upstream hosts, dropping addresses on this machine.
struct NoLocalResolver;

impl reqwest::dns::Resolve for NoLocalResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| !routes::is_local_ip(addr.ip()))
                .collect();
            if addrs.is_empty() {
                return Err("upstream host resolves only to this machine".into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

// -- Server boot ---------------------------------------------------------------

/// Boot the proxy on `127.0.0.1:0` (OS picks a free port). Returns once
/// the listener is bound - the actual serve loop runs in a spawned task.
pub async fn start() -> Result<ProxyHandle, ProxyError> {
    start_with(false).await
}

/// `allow_loopback_upstream` lets tests serve an upstream on 127.0.0.1; the
/// app never allows it (a manifest must not reach local services).
async fn start_with(allow_loopback_upstream: bool) -> Result<ProxyHandle, ProxyError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many upstream redirects")
            } else if !allow_loopback_upstream && !routes::is_safe_upstream(attempt.url().as_str())
            {
                attempt.error("blocked local upstream redirect")
            } else {
                attempt.follow()
            }
        }))
        .user_agent("Mozilla/5.0 (compatible; nova-iptv/1.0)");
    // Names are checked where they resolve: a manifest naming a host that
    // points at 127.0.0.1 (`*.nip.io`, `localtest.me`) can't reach local
    // services either.
    let client = if allow_loopback_upstream {
        client
    } else {
        client.dns_resolver(std::sync::Arc::new(NoLocalResolver))
    }
    .build()?;

    let sessions = SessionRegistry::new();
    let state = routes::AppState {
        client,
        sessions: sessions.clone(),
        origin: format!("http://127.0.0.1:{port}"),
        allow_loopback_upstream,
    };

    let app = Router::new()
        .route("/health", get(routes::health))
        .route("/stream/{token}/master.m3u8", get(routes::stream_master))
        .route("/stream/{token}/r/{id}", get(routes::stream_resource))
        .route("/stream/{token}/file", get(routes::stream_file))
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
        assert!(!is_safe_upstream("http://127.0.0.1/stream/t/file"));
        assert!(!is_safe_upstream("http://localhost:8080/x"));
        for url in [
            "http://[::1]/x",
            "http://127.2.3.4/x",
            "http://localhost./x",
            "http://a.localhost/x",
            "http://0.0.0.0/x",
            "http://[::ffff:127.0.0.1]/x",
        ] {
            assert!(!is_safe_upstream(url));
        }
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

    /// A fake provider on 127.0.0.1 whose paths carry "credentials".
    async fn upstream() -> String {
        use axum::http::{header, StatusCode};
        use axum::response::IntoResponse;
        let app = Router::new()
            .route(
                "/live/alice/s3cret/master.m3u8",
                get(|| async {
                    "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nchunks.m3u8\n".into_response()
                }),
            )
            .route(
                "/live/alice/s3cret/chunks.m3u8",
                get(|| async {
                    "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:6,\nseg1.ts\n"
                        .into_response()
                }),
            )
            .route("/live/alice/s3cret/seg1.ts", get(|| async { "SEGMENT" }))
            // What ffprobe calls HLS without saying so: no `.m3u8`, served
            // as `text/plain`, reached through a redirect to another path.
            .route(
                "/go/abc",
                get(|| async {
                    (
                        StatusCode::FOUND,
                        [(header::LOCATION, "/play/alice/s3cret/stream")],
                    )
                        .into_response()
                }),
            )
            .route(
                "/play/alice/s3cret/stream",
                get(|| async {
                    "\u{feff}#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6,\npart1\n"
                        .into_response()
                }),
            )
            .route("/play/alice/s3cret/init.mp4", get(|| async { "INIT" }))
            .route("/play/alice/s3cret/part1", get(|| async { "PART" }))
            .route("/live/alice/s3cret/key.bin", get(|| async { "KEY" }))
            .route(
                "/movie/alice/s3cret/1.mp4",
                get(|headers: axum::http::HeaderMap| async move {
                    match headers.get(header::RANGE) {
                        Some(_) => (StatusCode::PARTIAL_CONTENT, "MP").into_response(),
                        None => "MP4BYTES".into_response(),
                    }
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn text(url: &str) -> (u16, String) {
        let res = reqwest::get(url).await.unwrap();
        (res.status().as_u16(), res.text().await.unwrap())
    }

    /// The URIs a manifest points at (non-tag lines and `URI="…"`).
    fn uris(manifest: &str) -> Vec<String> {
        manifest
            .lines()
            .filter_map(|line| match line.find("URI=\"") {
                Some(at) => line[at + 5..].split('"').next().map(str::to_string),
                None if !line.starts_with('#') && !line.is_empty() => Some(line.to_string()),
                None => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_session_only_exposes_opaque_urls_of_its_own_manifests() {
        let provider = upstream().await;
        let proxy = start_with(true).await.unwrap();
        let origin = proxy.origin();
        let live = proxy
            .sessions()
            .register(
                format!("{provider}/live/alice/s3cret/master.m3u8"),
                true,
                Some(PlaybackRoute::HlsPassthrough),
            )
            .await
            .unwrap();

        let (status, master) = text(&format!("{origin}/stream/{}/master.m3u8", live.token)).await;
        assert_eq!(status, 200);
        let variant = &uris(&master)[0];
        assert!(variant.starts_with(&format!("{origin}/stream/{}/r/", live.token)));
        let (_, chunks) = text(variant).await;
        let links = uris(&chunks);
        assert_eq!(links.len(), 2, "{chunks}");
        let mut bodies = Vec::new();
        for link in &links {
            bodies.push(text(link).await.1);
        }
        assert_eq!(bodies, ["KEY", "SEGMENT"]);
        for manifest in [&master, &chunks] {
            assert!(
                !manifest.contains("alice") && !manifest.contains("s3cret"),
                "{manifest}"
            );
            assert!(!manifest.contains(&provider), "{manifest}");
        }

        // No open relay: the old route is gone, an unknown id is refused,
        // and one session's ids mean nothing under another token.
        let relay = format!("{origin}/proxy?url={provider}/live/alice/s3cret/seg1.ts");
        assert_eq!(text(&relay).await.0, 404);
        assert_eq!(
            text(&format!(
                "{origin}/stream/{}/r/0000000000000000",
                live.token
            ))
            .await
            .0,
            404
        );
        let other = proxy
            .sessions()
            .register(
                format!("{provider}/live/alice/s3cret/master.m3u8"),
                true,
                Some(PlaybackRoute::HlsPassthrough),
            )
            .await
            .unwrap();
        let foreign = links[1].replace(&live.token, &other.token);
        assert_eq!(text(&foreign).await.0, 404);

        // A direct file plays from its session, with Range forwarded.
        let file = proxy
            .sessions()
            .register(
                format!("{provider}/movie/alice/s3cret/1.mp4"),
                false,
                Some(PlaybackRoute::FilePassthrough),
            )
            .await
            .unwrap();
        let file_url = format!("{origin}/stream/{}/file", file.token);
        assert_eq!(text(&file_url).await, (200, "MP4BYTES".to_string()));
        let ranged = reqwest::Client::new()
            .get(&file_url)
            .header("Range", "bytes=0-1")
            .send()
            .await
            .unwrap();
        assert_eq!(ranged.status().as_u16(), 206);
        assert_eq!(
            text(&format!("{origin}/stream/{}/file", live.token))
                .await
                .0,
            400,
            "an HLS session has no file"
        );

        // Released sessions answer nothing.
        proxy.sessions().release(&live.token).await;
        assert_eq!(text(variant).await.0, 404);
        proxy.shutdown();
    }

    #[tokio::test]
    async fn a_playlist_is_rewritten_whatever_it_is_called_and_wherever_it_moved() {
        let provider = upstream().await;
        let proxy = start_with(true).await.unwrap();
        let session = proxy
            .sessions()
            .register(
                format!("{provider}/go/abc"),
                true,
                Some(PlaybackRoute::HlsPassthrough),
            )
            .await
            .unwrap();
        let origin = proxy.origin();
        let (status, master) =
            text(&format!("{origin}/stream/{}/master.m3u8", session.token)).await;
        assert_eq!(status, 200);
        assert!(
            !master.contains("alice") && !master.contains("s3cret"),
            "{master}"
        );
        let links = uris(&master);
        assert_eq!(links.len(), 2, "{master}");
        let mut bodies = Vec::new();
        for link in &links {
            bodies.push(text(link).await);
        }
        assert_eq!(
            bodies,
            [(200, "INIT".to_string()), (200, "PART".to_string())],
            "relative URIs resolve against the redirected playlist"
        );
        proxy.shutdown();
    }

    #[tokio::test]
    async fn hosts_that_resolve_to_this_machine_are_refused() {
        use reqwest::dns::Resolve;
        let refused = NoLocalResolver.resolve("localhost".parse().unwrap()).await;
        assert!(refused.is_err());
    }

    #[tokio::test]
    async fn the_app_proxy_never_fetches_local_services() {
        let provider = upstream().await;
        let proxy = start().await.unwrap();
        let live = proxy
            .sessions()
            .register(
                format!("{provider}/live/alice/s3cret/master.m3u8"),
                true,
                Some(PlaybackRoute::HlsPassthrough),
            )
            .await
            .unwrap();
        let url = format!("{}/stream/{}/master.m3u8", proxy.origin(), live.token);
        assert_eq!(text(&url).await.0, 400, "a 127.0.0.1 upstream is refused");
        proxy.shutdown();
    }
}
