//! Per-playback session registry.
//!
//! A session is created when the player opens a media item: the Tauri command
//! resolves the upstream URL from the DB, calls [`SessionRegistry::register`],
//! and hands the resulting [`SessionInfo`] back to the frontend. The proxy
//! then routes `/stream/{token}/...` and `/captions/{token}/...` requests through
//! the matching session. Dropping/releasing a session kills any ffmpeg
//! remuxer and frees the temp dir.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nova_media::{
    probe_stream, url_looks_like_browser_file, url_looks_like_hls_playlist,
    url_looks_like_transport_stream, PlaybackRoute, ProbeError, RemuxError, RemuxSession,
    StreamProbe,
};
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Maximum time we wait for ffprobe before giving up and assuming passthrough.
/// IPTV providers can be slow; a probe budget keeps the player snappy.
const PROBE_DEADLINE: Duration = Duration::from_secs(2);
/// Time we give ffmpeg to write `master.m3u8` before declaring the remux
/// failed.
const REMUX_STARTUP: Duration = Duration::from_secs(8);

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("probe failed: {0}")]
    Probe(#[from] ProbeError),
    #[error("remux failed: {0}")]
    Remux(#[from] RemuxError),
}

/// What the player should do with this session.
///
/// `Hls` - pull `/stream/{token}/master.m3u8` and attach hls.js.
/// `File` - pull the upstream bytes through `/stream/{token}/file`; use
/// the native `<video>` element with byte-range seek.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackKind {
    Hls,
    File,
}

/// Returned by [`SessionRegistry::register`]; included in the IPC
/// `resolve_stream` response so the player knows how to consume the URL.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub token: String,
    pub kind: PlaybackKind,
    pub route: PlaybackRoute,
    /// `true` for VOD/episode, `false` for live. Used to disable the seek bar.
    pub supports_seek: bool,
    pub duration_sec: Option<f64>,
    pub container: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub fallback_reason: Option<String>,
}

/// Internal state for a live session. Held behind an `Arc<Mutex>` so the
/// route handlers can borrow without blocking other sessions.
pub(crate) struct Session {
    pub upstream_url: String,
    pub probe: StreamProbe,
    pub route: PlaybackRoute,
    pub fallback_reason: Option<String>,
    pub remux: Option<RemuxSession>,
    /// Upstream URLs this session's manifests named, by opaque id.
    pub resources: crate::resources::ResourceTable,
}

#[derive(Clone, Default)]
pub struct SessionRegistry {
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<Session>>>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Probe `upstream_url`, optionally start a remuxer, and remember the
    /// session against a fresh token. The caller hands the token + kind to
    /// the player so it can build the manifest URL.
    pub async fn register(
        &self,
        upstream_url: String,
        is_live: bool,
        route_override: Option<PlaybackRoute>,
    ) -> Result<SessionInfo, SessionError> {
        let forced_route = route_override;
        let (probe, mut route, mut fallback_reason) = if let Some(route) = forced_route {
            (
                forced_probe_from_url(&upstream_url, route),
                route,
                Some(format!("player decode fallback forced {route:?}")),
            )
        } else {
            // URL shape is enough for common Xtream/IPTV paths. Avoid blocking
            // startup on ffprobe when a direct route is obvious.
            match fast_probe_from_url(&upstream_url) {
                Some(p) => {
                    let route = p.route;
                    (p, route, None)
                }
                None => match probe_stream(&upstream_url, PROBE_DEADLINE).await {
                    Ok(p) => {
                        let route = p.route;
                        (p, route, None)
                    }
                    Err(e) => {
                        let route = fallback_route_for_url(&upstream_url);
                        // ffprobe's stderr names the input URL (credentials).
                        let reason = nova_core::redact_urls(&format!("probe failed: {e}"));
                        tracing::warn!(
                            error = %e,
                            route = ?route,
                            "probe failed; using fast playback fallback"
                        );
                        (
                            StreamProbe {
                                container: None,
                                format_name: None,
                                video_codec: None,
                                audio_codec: None,
                                duration_sec: None,
                                width: None,
                                height: None,
                                is_playlist_hls: matches!(route, PlaybackRoute::HlsPassthrough),
                                is_transport_stream: matches!(route, PlaybackRoute::RemuxHls),
                                route,
                            },
                            route,
                            Some(reason),
                        )
                    }
                },
            }
        };
        if fallback_reason.is_none() {
            route = probe.route;
        }

        let remux = if route_requires_remux(route) {
            match RemuxSession::start(&upstream_url, route, REMUX_STARTUP).await {
                Ok(session) => Some(session),
                Err(e) => {
                    if forced_route.is_some() {
                        return Err(SessionError::Remux(e));
                    }
                    let fallback_route = fallback_route_for_probe(&upstream_url, &probe);
                    let reason =
                        nova_core::redact_urls(&format!("remux failed for {route:?}: {e}"));
                    tracing::warn!(
                        error = %e,
                        route = ?route,
                        fallback_route = ?fallback_route,
                        "remux failed; using fast playback fallback"
                    );
                    route = fallback_route;
                    fallback_reason = Some(reason);
                    None
                }
            }
        } else {
            None
        };

        let kind = playback_kind_for_route(route);
        let token = Uuid::new_v4().to_string();
        let duration_sec = probe.duration_sec;
        let container = probe.container.clone();
        let video_codec = probe.video_codec.clone();
        let audio_codec = probe.audio_codec.clone();
        let width = probe.width;
        let height = probe.height;
        let session = Arc::new(Mutex::new(Session {
            upstream_url,
            probe,
            route,
            fallback_reason: fallback_reason.clone(),
            remux,
            resources: crate::resources::ResourceTable::new(),
        }));
        self.sessions.lock().await.insert(token.clone(), session);

        Ok(SessionInfo {
            token,
            kind,
            route,
            supports_seek: !is_live,
            duration_sec,
            container,
            video_codec,
            audio_codec,
            width,
            height,
            fallback_reason,
        })
    }

    /// Drop the session keyed by `token`; the contained [`RemuxSession`]'s
    /// `Drop` reaps ffmpeg and clears the temp dir.
    pub async fn release(&self, token: &str) {
        let _ = self.sessions.lock().await.remove(token);
    }

    /// Drop every session (the webview that held their tokens is gone).
    pub async fn release_all(&self) {
        self.sessions.lock().await.clear();
    }

    pub(crate) async fn get(&self, token: &str) -> Option<Arc<Mutex<Session>>> {
        self.sessions.lock().await.get(token).cloned()
    }
}

fn route_requires_remux(route: PlaybackRoute) -> bool {
    matches!(route, PlaybackRoute::RemuxHls | PlaybackRoute::TranscodeHls)
}

fn playback_kind_for_route(route: PlaybackRoute) -> PlaybackKind {
    match route {
        PlaybackRoute::FilePassthrough => PlaybackKind::File,
        PlaybackRoute::HlsPassthrough | PlaybackRoute::RemuxHls | PlaybackRoute::TranscodeHls => {
            PlaybackKind::Hls
        }
    }
}

fn fallback_route_for_url(url: &str) -> PlaybackRoute {
    if url_looks_like_hls_playlist(url) {
        PlaybackRoute::HlsPassthrough
    } else if url_looks_like_transport_stream(url) {
        PlaybackRoute::RemuxHls
    } else {
        PlaybackRoute::FilePassthrough
    }
}

fn fallback_route_for_probe(url: &str, probe: &StreamProbe) -> PlaybackRoute {
    if probe.is_playlist_hls || url_looks_like_hls_playlist(url) {
        PlaybackRoute::HlsPassthrough
    } else {
        PlaybackRoute::FilePassthrough
    }
}

fn fast_probe_from_url(url: &str) -> Option<StreamProbe> {
    let (route, format_name, is_playlist_hls, is_transport_stream) =
        if url_looks_like_hls_playlist(url) {
            // Do not fast-path HLS playlists. The manifest extension tells us
            // nothing about segment codecs; IPTV providers often hide AC-3,
            // E-AC-3, HEVC, or other browser-hostile tracks behind .m3u8.
            return None;
        } else if url_looks_like_transport_stream(url) {
            (
                PlaybackRoute::RemuxHls,
                Some("mpegts".to_string()),
                false,
                true,
            )
        } else if url_looks_like_browser_file(url) {
            (
                PlaybackRoute::FilePassthrough,
                Some("mov,mp4,m4a,3gp,3g2,mj2".to_string()),
                false,
                false,
            )
        } else {
            return None;
        };

    Some(StreamProbe {
        container: None,
        format_name,
        video_codec: None,
        audio_codec: None,
        duration_sec: None,
        width: None,
        height: None,
        is_playlist_hls,
        is_transport_stream,
        route,
    })
}

fn forced_probe_from_url(url: &str, route: PlaybackRoute) -> StreamProbe {
    let (format_name, is_playlist_hls, is_transport_stream) = if url_looks_like_hls_playlist(url) {
        (Some("hls".to_string()), true, false)
    } else if url_looks_like_transport_stream(url) {
        (Some("mpegts".to_string()), false, true)
    } else if url_looks_like_browser_file(url) {
        (Some("mov,mp4,m4a,3gp,3g2,mj2".to_string()), false, false)
    } else {
        (None, false, false)
    };

    StreamProbe {
        container: None,
        format_name,
        video_codec: None,
        audio_codec: None,
        duration_sec: None,
        width: None,
        height: None,
        is_playlist_hls,
        is_transport_stream,
        route,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_probe_does_not_trust_hls_extension() {
        assert!(fast_probe_from_url("https://provider.test/live/1.m3u8").is_none());
    }

    #[test]
    fn forced_probe_preserves_requested_route() {
        let probe = forced_probe_from_url(
            "https://provider.test/live/1.m3u8",
            PlaybackRoute::TranscodeHls,
        );
        assert_eq!(probe.route, PlaybackRoute::TranscodeHls);
        assert!(probe.is_playlist_hls);
    }
}
