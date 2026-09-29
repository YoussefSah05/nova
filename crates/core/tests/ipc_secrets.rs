//! Provider credentials and authenticated locators stay Rust-side: the legacy
//! serving DTOs that cross IPC must never serialize them (bootstrap §4, §53).

use nova_core::{display_source_url, Channel, SeriesEpisode, Source, VodItem};
use serde_json::{json, Value};

const USER: &str = "alice_user";
const PASS: &str = "s3cr3t-pass";

fn xtream_stream(kind: &str) -> String {
    format!("http://provider.example:8080/{kind}/{USER}/{PASS}/1234.ts")
}

/// Round-trip through the DTO and return the IPC JSON it would produce.
fn ipc_json<T: serde::de::DeserializeOwned + serde::Serialize>(input: Value) -> String {
    let dto: T = serde_json::from_value(input).expect("fixture deserializes");
    serde_json::to_string(&dto).expect("dto serializes")
}

fn assert_secret_free(json: &str) {
    for secret in [USER, PASS, "streamUrl", "username=", "password="] {
        assert!(!json.contains(secret), "IPC JSON leaks {secret:?}: {json}");
    }
}

#[test]
fn source_serializes_only_a_host_label() {
    let json = ipc_json::<Source>(json!({
        "id": 1, "kind": "xtream", "name": "My provider",
        "url": format!("http://{USER}:{PASS}@provider.example:8080/get.php?username={USER}&password={PASS}&type=m3u_plus"),
        "lastRefreshedAt": null, "autoRefreshHours": 24,
    }));
    assert_secret_free(&json);
    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["displayUrl"], "http://provider.example:8080");
    assert!(
        value.get("url").is_none(),
        "raw url must not cross IPC: {json}"
    );
}

#[test]
fn playable_rows_never_serialize_their_stream_url() {
    let channel = ipc_json::<Channel>(json!({
        "id": 1, "sourceId": 1, "name": "News", "streamUrl": xtream_stream("live"),
        "isFavorite": false, "isInWatchlist": false,
    }));
    let vod = ipc_json::<VodItem>(json!({
        "id": 2, "sourceId": 1, "name": "Film", "streamUrl": xtream_stream("movie"),
        "isFavorite": false, "isInWatchlist": false,
    }));
    let episode = ipc_json::<SeriesEpisode>(json!({
        "id": 3, "sourceId": 1, "seriesTitle": "Show", "season": 1, "episode": 2,
        "streamUrl": xtream_stream("series"),
    }));
    for json in [channel, vod, episode] {
        assert_secret_free(&json);
    }
}

#[test]
fn display_url_drops_user_info_path_and_query() {
    let cases = [
        (
            "http://provider.example:8080/get.php?username=u&password=p",
            "http://provider.example:8080",
        ),
        ("HTTPS://u:p@cdn.example/list.m3u", "https://cdn.example"),
        (
            "http://provider.example?username=u",
            "http://provider.example",
        ),
        (
            "  http://provider.example#frag  ",
            "http://provider.example",
        ),
        (
            "http://u:p@ss@provider.example/get.php",
            "http://provider.example",
        ),
        ("http://[::1]:8080/get.php?password=p", "http://[::1]:8080"),
        // Malformed URLs the webview's `new URL` accepts: stored, then rejected on import.
        (
            "http:provider.example/get.php?username=u&password=p",
            "get.php",
        ),
        ("http:\\\\provider.example\\get.php?password=p", "get.php"),
        ("user:pass@provider.example", "provider.example"),
        ("http://?username=u&password=p", ""),
        ("file:///Users/me/Playlists/family.m3u", "family.m3u"),
        ("/Users/me/Playlists/family.m3u", "family.m3u"),
        ("", ""),
    ];
    for (input, expected) in cases {
        assert_eq!(display_source_url(input), expected, "input {input:?}");
    }
}

#[test]
fn free_text_never_keeps_a_urls_credentials() {
    use nova_core::redact_urls;
    let cases = [
        (
            "ffprobe failed: http://host.tv:8080/live/alice/s3cret/101.ts: Server returned 403 Forbidden",
            "ffprobe failed: http://host.tv:8080/… Server returned 403 Forbidden",
        ),
        (
            "error sending request for url (https://api.themoviedb.org/3/movie/1?api_key=abc123)",
            "error sending request for url (https://api.themoviedb.org/…)",
        ),
        ("fetch http://bob:pw@cdn.example/x failed", "fetch http://cdn.example/… failed"),
        ("rtmp://live.example/app/key and https://ok.example", "rtmp://live.example/… and https://ok.example"),
        ("\"HTTP://Host/get.php?username=u&password=p\"", "\"http://Host/…\""),
        ("HTTP 404 from upstream", "HTTP 404 from upstream"),
        ("weird ://nothing here", "weird ://nothing here"),
    ];
    for (input, expected) in cases {
        let out = redact_urls(input);
        assert_eq!(out, expected, "{input}");
        for secret in [
            "alice",
            "s3cret",
            "abc123",
            "bob",
            "pw@",
            "password=p",
            "/app/key",
        ] {
            assert!(!out.contains(secret), "{secret} survived in {out}");
        }
    }
}
