# Deterministic provider tests

The `nova-iptv-fixture` workspace tool provides a loopback-only Axum server with
compiled-in M3U, Xtream-shaped JSON and XMLTV from `fixtures/iptv/v1`. It depends
only on already-locked workspace packages. Adapter crates are dev-dependencies;
the desktop application has no dependency on the fixture tool.

```sh
just test-fixtures
just fixture-provider
cargo run -p nova-iptv-fixture --locked -- vod-unavailable
```

The CLI prints its ephemeral URL; Ctrl+C requests bounded graceful shutdown. For
automated tests, `FixtureServer::start` returns only after binding and exposes the
base URL directly. Tests have a whole-import deadline, drain channels until closure,
and compare counts/metadata independent of parallel endpoint completion order.
Explicit `shutdown` waits with a deadline; Drop signals graceful shutdown on failure.

Coverage: actual M3U curl HTTP parsing, Xtream parallel endpoint parsing and lazy
episodes, XMLTV UTF-8/timezone handling, duplicate upstream occurrences, invalid
HTML/JSON, 503 versus valid empty endpoint, fixture authentication and unknown routes.
Unknown raw provider fields are retained in fixture files but not in legacy
ParsedEntry; new lossless SourceRecord adapters remain future work.

This is service/adapter integration coverage. It does not launch Tauri, touch a
profile, exercise catalog publication, play media or certify a real provider.
The production import-validation skill's real-provider probe/app gates remain
required before any production import change is called validated. The fixture
exposed an M3U UTF-8 slicing panic; the narrow parser correction and its remaining
validation gates are recorded in [the regression note](implementation/m3u-utf8-regression.md).
CI's existing `cargo test --workspace --locked` includes these tests;
runner localhost binding and curl are required. Remote CI results are still unverified.

Research: the pinned [Axum 0.8.9 serve contract](https://docs.rs/axum/0.8.9/axum/serve/struct.Serve.html)
supports graceful shutdown. Adopt a bound listener plus shutdown signal and harness
deadline; avoid fixed port/sleep-based readiness or a public provider dependency.
No Docker or Python environment is required for this native Rust fixture service.

The encrypted file-store prototype is tested with
`cargo test -p nova-storage --locked`: fixture-byte replay after reopen, tamper and
cross-profile rejection, quotas, concurrent commits, private permissions and two
abrupt subprocess exits at staging/commit boundaries. The ignored subprocess helper
is explicitly executed by those boundary tests. These are process-restart tests,
not a physical power-loss or app E2E certification.

Next gates: Keychain/retention integration and adapter evidence completeness;
then real macOS app E2E using an isolated test profile. Playable media, segmented
fault injection, sleep/wake, caption quality and AirPlay still need distinct coverage.
