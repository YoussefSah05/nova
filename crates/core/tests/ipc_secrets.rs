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
        ("file:///Users/me/Playlists/family.m3u", "family.m3u"),
        ("/Users/me/Playlists/family.m3u", "family.m3u"),
        ("", ""),
    ];
    for (input, expected) in cases {
        assert_eq!(display_source_url(input), expected, "input {input:?}");
    }
}
