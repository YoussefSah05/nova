# Getting started

This guide takes you from a fresh clone to watching a film with translated captions.

## 1. Requirements

| | |
|---|---|
| Hardware | Apple Silicon Mac (M1 or newer) |
| OS | macOS 14 Sonoma or newer |
| Disk | ~1 GB for the app and build cache, plus ~1.1 GB if you install every model |
| Tools | [mise](https://mise.jdx.dev/getting-started.html), [Homebrew](https://brew.sh), Xcode command-line tools, CMake |

Exact tool versions are pinned in [`mise.toml`](../../mise.toml) and
[`nova-stack.json`](../../nova-stack.json); see [STACK.md](../../STACK.md).

## 2. Install

```bash
git clone https://github.com/YoussefSah05/nova.git
cd nova

# Media libraries: libmpv for playback, FFmpeg for audio extraction and probing
brew install mpv ffmpeg

# Pinned Rust / Node / pnpm / just, locked packages, and Nova's private Whisper runtime
bash scripts/bootstrap.sh
```

`bootstrap.sh` never touches your profiles or models. It builds whisper.cpp 1.8.3 from
checksum-pinned source into Nova's app-data folder; rerun `just install-whisper` to repair it.

## 3. Run

```bash
just dev     # development window with hot reload
just build   # release build: target/release/bundle/macos/Nova.app and a .dmg
```

The release bundle links Homebrew's libmpv, so it runs on the Mac that built it. Signed,
self-contained builds are on the roadmap.

## 4. Add a playlist

On first launch, create a profile, then add a source:

- **M3U URL or file**: any `.m3u` / `.m3u8` playlist.
- **Xtream Codes**: server, username and password. Nova uses the JSON API, which fetches live
  channels, films and series in parallel.

Import streams progress as it parses. A ~42k-entry playlist completes in a few seconds; artwork
and enrichment continue in the background. Programme guides (XMLTV) declared by the playlist are
imported too, and you can add your own guide URL in **Settings → Playlists & guide**.

Optional: add your own [TMDB API key](https://www.themoviedb.org/settings/api) in **Settings → Catalog** to
match films and shows to artwork, cast, ratings and recommendations.

## 5. Install models (optional)

Open **Settings → Models**:

| Card | Enables | Size |
|---|---|---|
| Source-language captions | Captions on any film or episode | ~500 MB |
| Translated captions | English · French · Arabic translation of captions | ~450 MB |
| Catalog intelligence | Semantic search, genres, duplicate grouping | ~120 MB |

Every download is checked against pinned SHA-256 checksums before Nova uses it. Already have the
models on an external drive? Use **External model folders → Choose folder** instead of
downloading.

## 6. Watch with captions

Open a film, press **Play**, then use the **CC** menu (or press `C`) to choose source captions,
a translation, or both. The first caption window takes longer while Core ML prepares Whisper for
your Mac; later windows take a couple of seconds.

## Troubleshooting

| Problem | Fix |
|---|---|
| `whisper-cli failed to start` | Run `just install-whisper`, then restart Nova. |
| Playback shows a black frame | Check `brew list mpv`; reinstall with `brew reinstall mpv`. |
| Captions say "model unavailable" | Download it in Settings, or check it is enabled under model controls. |
| Translation says "unavailable" | Download *Translated captions*; source captions keep working meanwhile. |
| Import fails with HTTP 403 | Many providers block unknown clients or too many connections; close other players using the same account and retry. |
| Tool version drift | `just doctor` reports what differs from the pinned stack. |

Still stuck? Open an issue with the output of `just doctor` and the steps to reproduce. Leave
out playlist URLs and credentials.
