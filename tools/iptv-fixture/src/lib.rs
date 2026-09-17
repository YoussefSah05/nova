//! Test-only provider emulator. Fixed synthetic files, loopback binding, no upstream I/O.

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use std::{io, time::Duration};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle, time::timeout};

pub const CORPUS_VERSION: u32 = 1;
pub const PLAYLIST: &str = include_str!("../../../fixtures/iptv/v1/playlist.m3u");
pub const LIVE: &str = include_str!("../../../fixtures/iptv/v1/live.json");
pub const VOD: &str = include_str!("../../../fixtures/iptv/v1/vod.json");
pub const SERIES: &str = include_str!("../../../fixtures/iptv/v1/series.json");
pub const EPISODES: &str = include_str!("../../../fixtures/iptv/v1/episodes.json");
pub const GUIDE: &str = include_str!("../../../fixtures/iptv/v1/guide.xml");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Normal,
    VodUnavailable,
    VodMalformed,
    VodEmpty,
    PlaylistHtml,
}

impl Scenario {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "normal" => Some(Self::Normal),
            "vod-unavailable" => Some(Self::VodUnavailable),
            "vod-malformed" => Some(Self::VodMalformed),
            "vod-empty" => Some(Self::VodEmpty),
            "playlist-html" => Some(Self::PlaylistHtml),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct FixtureState {
    base_url: String,
    scenario: Scenario,
}

pub struct FixtureServer {
    base_url: String,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<io::Result<()>>>,
}

impl FixtureServer {
    pub async fn start(scenario: Scenario) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let state = FixtureState {
            base_url: base_url.clone(),
            scenario,
        };
        let app = Router::new()
            .route("/health", get(|| async { "nova-iptv-fixture-v1" }))
            .route("/playlist.m3u", get(playlist))
            .route("/get.php", get(playlist))
            .route("/player_api.php", get(api))
            .route(
                "/xmltv.php",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
                        GUIDE,
                    )
                }),
            )
            .with_state(state);
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
        });
        Ok(Self {
            base_url,
            stop: Some(stop),
            task: Some(task),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Explicit bounded shutdown for harnesses; Drop also signals graceful shutdown.
    pub async fn shutdown(mut self) -> io::Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(mut task) = self.task.take() {
            match timeout(Duration::from_secs(2), &mut task).await {
                Ok(result) => result.map_err(|_| io::Error::other("fixture task failed"))?,
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "fixture shutdown timed out",
                    ))
                }
            }
        } else {
            Ok(())
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

async fn playlist(State(state): State<FixtureState>) -> Response {
    if state.scenario == Scenario::PlaylistHtml {
        return (
            [(header::CONTENT_TYPE, "text/html")],
            "<html>fixture login page</html>",
        )
            .into_response();
    }
    (
        [(header::CONTENT_TYPE, "audio/x-mpegurl; charset=utf-8")],
        PLAYLIST.replace("{{BASE_URL}}", &state.base_url),
    )
        .into_response()
}

#[derive(Deserialize)]
struct ApiQuery {
    username: Option<String>,
    password: Option<String>,
    action: Option<String>,
    series_id: Option<String>,
}

async fn api(State(state): State<FixtureState>, Query(query): Query<ApiQuery>) -> Response {
    if query.username.as_deref() != Some("fixture") || query.password.as_deref() != Some("fixture")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let body = match query.action.as_deref() {
        Some("get_live_streams") => LIVE,
        Some("get_vod_streams") => match state.scenario {
            Scenario::VodUnavailable => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            Scenario::VodMalformed => "{invalid-json",
            Scenario::VodEmpty => "[]",
            _ => VOD,
        },
        Some("get_series") => SERIES,
        Some("get_series_info") if query.series_id.as_deref() == Some("301") => EPISODES,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body,
    )
        .into_response()
}
