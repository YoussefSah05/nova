//! HTTP routes.
//!
//! - `GET /health` - liveness.
//! - `GET /proxy?url=...` - Phase 5 passthrough (HLS segments after manifest
//!   rewriting come through here). Also used by the player directly when
//!   `resolve_stream` returns `PlaybackKind::File`.
//! - `GET /stream/{token}/master.m3u8` - manifest endpoint. Behaviour depends
//!   on the session route: passthrough-HLS rewrites the upstream
//!   manifest; remux serves the file ffmpeg is writing into the temp dir.
//! - `GET /stream/{token}/local/{file}` - file inside the remux session's
//!   temp dir (segments).
//! - `GET /captions/{token}/{lang}.vtt` - source-language captions. 404 in
//!   Phase 5 until `crates/transcribe/` populates these in Phase 6.
//! - `GET /captions/{token}/{src}-to-{tgt}.vtt` - translated captions. 404
//!   in Phase 5 until `crates/translate/` populates these in Phase 7.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

use crate::manifest::rewrite_manifest;
use crate::session::SessionRegistry;
use nova_media::PlaybackRoute;

#[derive(Clone)]
pub(crate) struct AppState {
    pub client: reqwest::Client,
    pub sessions: SessionRegistry,
    pub origin: String,
}

pub(crate) async fn health() -> &'static str {
    "ok"
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProxyParams {
    pub url: String,
}

pub(crate) async fn proxy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ProxyParams>,
) -> Response {
    if !is_safe_upstream(&params.url) {
        return (StatusCode::BAD_REQUEST, "invalid upstream URL").into_response();
    }
    forward(&state.client, &params.url, &state.origin, &headers).await
}

pub(crate) async fn stream_master(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    let session_arc = match state.sessions.get(&token).await {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "unknown session").into_response(),
    };
    let session = session_arc.lock().await;
    let route = session.route;
    let fallback_reason = session.fallback_reason.clone();
    let is_transport_stream = session.probe.is_transport_stream;

    if let Some(remux) = &session.remux {
        let master = remux.dir().join("master.m3u8");
        return serve_local_file(&master, "application/vnd.apple.mpegurl").await;
    }

    match route {
        PlaybackRoute::HlsPassthrough => {
            let upstream = session.upstream_url.clone();
            drop(session);
            fetch_and_rewrite_manifest(&state, &upstream, &headers).await
        }
        PlaybackRoute::FilePassthrough => {
            tracing::debug!(
                route = ?route,
                is_transport_stream,
                fallback_reason = fallback_reason.as_deref(),
                "master playlist requested for direct-file session"
            );
            // The player should have routed direct files through /proxy?url=
            // (PlaybackKind::File). If we got here the frontend is confused;
            // serve a synthetic 400 rather than streaming an MP4 disguised
            // as a manifest.
            (
                StatusCode::BAD_REQUEST,
                "session is a direct file; use the file URL instead",
            )
                .into_response()
        }
        PlaybackRoute::RemuxHls | PlaybackRoute::TranscodeHls => {
            tracing::warn!(
                route = ?route,
                fallback_reason = fallback_reason.as_deref(),
                "remux route had no active remux session"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "remux session missing").into_response()
        }
    }
}

pub(crate) async fn stream_local(
    State(state): State<AppState>,
    Path((token, file)): Path<(String, String)>,
) -> Response {
    let session_arc = match state.sessions.get(&token).await {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "unknown session").into_response(),
    };
    let session = session_arc.lock().await;
    let Some(remux) = &session.remux else {
        return (StatusCode::NOT_FOUND, "no local files for this session").into_response();
    };
    // Defensive: refuse any path that escapes the temp dir.
    if file.contains('/') || file.contains('\\') || file.contains("..") {
        return (StatusCode::BAD_REQUEST, "bad filename").into_response();
    }
    let path = remux.dir().join(&file);
    let mime = if file.ends_with(".m3u8") {
        "application/vnd.apple.mpegurl"
    } else if file.ends_with(".m4s") || file.ends_with(".mp4") {
        "video/iso.segment"
    } else {
        "application/octet-stream"
    };
    serve_local_file(&path, mime).await
}

/// 404 for every caption URL in Phase 5. Phase 6 will swap this for a
/// handler that returns the VTT file generated by `crates/transcribe/`;
/// Phase 7 will overload it for the translated variant.
pub(crate) async fn captions_stub() -> Response {
    (StatusCode::NOT_FOUND, "captions arrive in Phase 6").into_response()
}

// -- Helpers -----------------------------------------------------------------

async fn fetch_and_rewrite_manifest(
    state: &AppState,
    upstream_url: &str,
    headers: &HeaderMap,
) -> Response {
    let mut req = state.client.get(upstream_url);
    if let Some(range) = headers.get(header::RANGE) {
        req = req.header(header::RANGE, range);
    }
    let res = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "manifest fetch failed");
            return (StatusCode::BAD_GATEWAY, "manifest fetch failed").into_response();
        }
    };
    let status = res.status();
    let body = match res.text().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "manifest body read failed");
            return (StatusCode::BAD_GATEWAY, "manifest body read failed").into_response();
        }
    };
    let rewritten = rewrite_manifest(&body, upstream_url, &state.origin);
    let mut response = Response::new(Body::from(rewritten));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.apple.mpegurl"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    add_cors(response.headers_mut());
    response
}

async fn forward(
    client: &reqwest::Client,
    url: &str,
    proxy_origin: &str,
    in_headers: &HeaderMap,
) -> Response {
    let mut req = client.get(url);
    // Forward Range so byte-seek on direct MP4 works.
    if let Some(range) = in_headers.get(header::RANGE) {
        req = req.header(header::RANGE, range);
    }
    let upstream = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "upstream fetch failed");
            return (StatusCode::BAD_GATEWAY, "upstream fetch failed").into_response();
        }
    };

    let status = upstream.status();
    let content_type = upstream.headers().get(header::CONTENT_TYPE).cloned();
    if is_hls_manifest_response(url, content_type.as_ref()) {
        let body = match upstream.text().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e.without_url(), "manifest proxy body read failed");
                return (StatusCode::BAD_GATEWAY, "manifest body read failed").into_response();
            }
        };
        let rewritten = rewrite_manifest(&body, url, proxy_origin);
        let mut response = Response::new(Body::from(rewritten));
        *response.status_mut() = status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/vnd.apple.mpegurl"),
        );
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        add_cors(response.headers_mut());
        return response;
    }

    let mut forwarded = HeaderMap::new();
    for name in &[
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::CACHE_CONTROL,
        HeaderName::from_static("accept-ranges"),
    ] {
        if let Some(v) = upstream.headers().get(name) {
            forwarded.insert(name.clone(), v.clone());
        }
    }
    add_cors(&mut forwarded);

    let body = Body::from_stream(upstream.bytes_stream());
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = forwarded;
    response
}

async fn serve_local_file(path: &std::path::Path, mime: &str) -> Response {
    let file = match File::open(path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, path = ?path, "local file open failed");
            return (StatusCode::NOT_FOUND, "file not found").into_response();
        }
    };
    let meta = match file.metadata().await {
        Ok(m) => m,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "stat failed").into_response(),
    };
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&meta.len().to_string()).unwrap(),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    add_cors(&mut headers);
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    *response.headers_mut() = headers;
    response
}

fn add_cors(headers: &mut HeaderMap) {
    headers.insert(
        HeaderName::from_static("access-control-allow-origin"),
        HeaderValue::from_static("*"),
    );
}

pub(crate) fn is_safe_upstream(url: &str) -> bool {
    let parsed = match reqwest::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return false;
    }
    let literal = lower.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = literal.parse::<std::net::IpAddr>() {
        if ip.is_loopback() || ip.is_unspecified() {
            return false;
        }
        if let std::net::IpAddr::V6(ip) = ip {
            if ip
                .to_ipv4_mapped()
                .is_some_and(|v4| v4.is_loopback() || v4.is_unspecified())
            {
                return false;
            }
        }
    }
    true
}

pub(crate) fn is_hls_manifest_response(url: &str, content_type: Option<&HeaderValue>) -> bool {
    let lower_url = url.to_ascii_lowercase();
    if lower_url.contains(".m3u8") {
        return true;
    }

    content_type
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            let lower = v.to_ascii_lowercase();
            lower.contains("mpegurl") || lower.contains("application/vnd.apple.mpegurl")
        })
        .unwrap_or(false)
}
