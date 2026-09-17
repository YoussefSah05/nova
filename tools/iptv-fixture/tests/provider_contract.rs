use nova_core::{MediaKind, ParsedEntry};
use nova_iptv_fixture::{FixtureServer, Scenario};
use nova_xtream::XtreamCredentials;
use std::time::Duration;
use tokio::time::timeout;

async fn drain(
    mut rx: tokio::sync::mpsc::Receiver<Result<ParsedEntry, String>>,
) -> (Vec<ParsedEntry>, Vec<String>) {
    timeout(Duration::from_secs(10), async move {
        let mut entries = Vec::new();
        let mut errors = Vec::new();
        while let Some(item) = rx.recv().await {
            match item {
                Ok(entry) => entries.push(entry),
                Err(error) => errors.push(error),
            }
        }
        (entries, errors)
    })
    .await
    .expect("fixture import must close within the deadline")
}

fn credentials(server: &FixtureServer) -> XtreamCredentials {
    XtreamCredentials {
        base_url: server.base_url().into(),
        username: "fixture".into(),
        password: "fixture".into(),
    }
}

#[tokio::test]
async fn fixture_http_is_loopback_scoped_and_has_bounded_shutdown() {
    let server = FixtureServer::start(Scenario::Normal).await.unwrap();
    assert!(server.base_url().starts_with("http://127.0.0.1:"));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(format!("{}/health", server.base_url()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "nova-iptv-fixture-v1"
    );
    assert_eq!(
        client
            .get(format!(
                "{}/player_api.php?action=get_live_streams",
                server.base_url()
            ))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{}/not-a-file", server.base_url()))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn xmltv_http_fixture_preserves_arabic_and_equivalent_utc_times() {
    let server = FixtureServer::start(Scenario::Normal).await.unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let bytes = client
        .get(format!("{}/xmltv.php", server.base_url()))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let programs = nova_epg::parse(std::io::Cursor::new(bytes))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(programs.len(), 2);
    assert_eq!(programs[0].start_utc, programs[1].start_utc);
    assert_eq!(programs[1].title, "أخبار الرياضة");
    assert_eq!(programs[1].channel_tvg_id, "sports2.test");
}

#[tokio::test]
async fn m3u_html_login_response_errors_and_closes_instead_of_looking_empty() {
    let server = FixtureServer::start(Scenario::PlaylistHtml).await.unwrap();
    let playlist = nova_m3u::parse(&format!("{}/playlist.m3u", server.base_url())).await;
    let (entries, errors) = drain(playlist.entries).await;
    assert!(entries.is_empty());
    assert!(!errors.is_empty());
}

fn counts(entries: &[ParsedEntry]) -> (usize, usize, usize) {
    (
        entries.iter().filter(|e| e.kind == MediaKind::Live).count(),
        entries
            .iter()
            .filter(|e| e.kind == MediaKind::Movie)
            .count(),
        entries
            .iter()
            .filter(|e| e.kind == MediaKind::Series)
            .count(),
    )
}

#[tokio::test]
async fn m3u_real_http_client_preserves_names_groups_and_numbered_channels() {
    let server = FixtureServer::start(Scenario::Normal).await.unwrap();
    let playlist = nova_m3u::parse(&format!("{}/playlist.m3u", server.base_url())).await;
    let (entries, errors) = drain(playlist.entries).await;
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(counts(&entries), (2, 5, 1));
    assert_eq!(entries[0].name, "FR| Example Sports 1 FHD");
    assert_eq!(entries[1].name, "AR| رياضة 2 HD");
    assert_eq!(entries[2].group_title, "FR MOVIES");
    assert_eq!(entries[7].season, Some(1));
    assert_eq!(entries[7].episode, Some(2));
    let metadata = timeout(Duration::from_secs(1), playlist.metadata)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        metadata.url_tvg,
        Some(format!("{}/xmltv.php", server.base_url()))
    );
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn xtream_parallel_endpoints_keep_raw_names_and_duplicate_upstream_occurrences() {
    let server = FixtureServer::start(Scenario::Normal).await.unwrap();
    let (entries, errors) = drain(nova_xtream::fetch(&credentials(&server)).await).await;
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(counts(&entries), (2, 5, 1));
    assert!(entries.iter().any(|e| e.name == "AR| Dune Part Two (2024)"));
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.stream_url.contains("/201."))
            .count(),
        2
    );
    let series = entries
        .iter()
        .find(|e| e.kind == MediaKind::Series)
        .unwrap();
    assert_eq!(series.stream_url, "xtream://series/301");
    // Characterize legacy sentinel behavior; it is not canonical episode numbering.
    assert_eq!((series.season, series.episode), (Some(0), Some(0)));
}

#[tokio::test]
async fn failed_vod_endpoint_reports_error_and_other_scopes_finish() {
    let server = FixtureServer::start(Scenario::VodUnavailable)
        .await
        .unwrap();
    let (entries, errors) = drain(nova_xtream::fetch(&credentials(&server)).await).await;
    assert_eq!(counts(&entries), (2, 0, 1));
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("503"));
}

#[tokio::test]
async fn malformed_vod_is_not_reported_as_successful_empty_scope() {
    let server = FixtureServer::start(Scenario::VodMalformed).await.unwrap();
    let (entries, errors) = drain(nova_xtream::fetch(&credentials(&server)).await).await;
    assert_eq!(counts(&entries), (2, 0, 1));
    assert!(!errors.is_empty());
}

#[tokio::test]
async fn valid_empty_vod_is_distinct_from_failed_endpoint() {
    let server = FixtureServer::start(Scenario::VodEmpty).await.unwrap();
    let (entries, errors) = drain(nova_xtream::fetch(&credentials(&server)).await).await;
    assert_eq!(counts(&entries), (2, 0, 1));
    assert!(errors.is_empty());
}

#[tokio::test]
async fn lazy_episode_fetch_orders_specials_and_mixed_numeric_ids() {
    let server = FixtureServer::start(Scenario::Normal).await.unwrap();
    let episodes = timeout(
        Duration::from_secs(10),
        nova_xtream::fetch_series_info(&credentials(&server), 301),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        episodes
            .iter()
            .map(|e| (e.season, e.episode))
            .collect::<Vec<_>>(),
        vec![(0, 1), (1, 1), (1, 2)]
    );
    assert_eq!(episodes[2].title.as_deref(), Some("Deuxième / الثاني"));
    assert!(episodes[2].stream_url.ends_with("/401.mkv"));
}
