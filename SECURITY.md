# Security and privacy

## Reporting a vulnerability

Please report vulnerabilities privately through
[GitHub security advisories](https://github.com/YoussefSah05/nova/security/advisories/new)
rather than a public issue. Include steps to reproduce and the affected version or commit.
Nova is a personal, alpha-stage project; reports are handled on a best-effort basis.

## What Nova protects

| Asset | Protection |
|---|---|
| Provider URLs and credentials | Kept in the Rust backend. The UI receives opaque IDs; playback goes through a loopback, session-scoped proxy that is not an open relay and refuses local/DNS upstreams. |
| TMDB API key | Write-only from the UI; never returned over IPC. |
| Logs and errors | URLs are reduced to `scheme://host` before logging or display. |
| Webview | Strict Content Security Policy; typed, token-checked IPC commands. |
| Models | Pinned revisions and SHA-256 checksums; mismatched files are refused, never loaded. |
| Your data | Per-profile SQLite databases in the app-data folder; no telemetry, no accounts, no cloud inference. |

## Network access

Nova contacts your playlist provider, TMDB (only when enabled with your own key), and Hugging
Face or GitHub (only when you download a model or install the Whisper runtime).

## Known gaps

Tracked in the [implementation status](docs/nova-implementation-status.md) under M25: provider
artwork URLs still reach the webview, `Debug`-formatted URLs and panics can bypass log
redaction, and release builds are not yet signed or notarised.
