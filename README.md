# Nova

A local-first desktop IPTV streaming platform with on-device AI transcription and translation.

Import any M3U playlist and watch live channels, VOD, and series with Netflix-class UX.
Transcribe VOD content on your Mac (whisper.cpp, Apple Neural Engine). Translate to any
language via NLLB-200 — all on device, no cloud required. A signature "translation card"
overlay makes watching content in any language effortless.

**Status**: 0.2.0-alpha.0 development, starting the production architecture migration.
Provider import, browsing, favorites/history, native playback, local captions,
translation and search exist. New catalog contracts are not yet connected to imports.
See [implementation progress](docs/implementation/progress.md) for verified checkpoints
and outstanding E2E/device/release gates; the historical architecture brief contains
older plans and is not a current feature-status ledger.

---

## Requirements

| Tool | Version | Install |
|---|---|---|
| [mise](https://mise.jdx.dev) | bootstrap tool | [Official installation](https://mise.jdx.dev/getting-started.html) |
| Rust | 1.95.0 | via mise |
| Node.js | 22.22.3 | via mise |
| pnpm | 9.15.4 | via mise |
| just | 1.51.0 | via mise |

> macOS 14+ on Apple Silicon is the primary target. Intel Mac and Windows are planned.

---

## Onboarding

```bash
# 1. Clone
git clone https://github.com/YoussefSah05/nova.git
cd nova

# 2. Install selected toolchains and fetch locked packages (mise required)
bash scripts/bootstrap.sh

# 3. Open the dev window
just dev
```

`bash scripts/bootstrap.sh` (also `just setup`) performs explicit development setup:
1. Installs pinned Rust, Node, pnpm, and `just` via `mise install`.
2. Installs JavaScript packages via `pnpm install --frozen-lockfile`.
3. Fetches Rust dependencies via `cargo fetch --locked`.
4. Runs development-toolchain verification. It does not install media libraries or models.

Native mpv/FFmpeg/Whisper dependencies still require separate setup; a reproducible
native SDK and clean-machine certification are pending. See [STACK.md](STACK.md)
before trying playback. `pnpm doctor` reports tool drift and known readiness gaps;
it never silently installs tools or opens user databases.

`just dev` compiles the Rust backend and opens a Tauri window with the Vite dev server.
Hot module replacement is active for the React frontend.

---

## All `just` commands

```
just setup        Install toolchain + fetch all dependencies
just dev          Start Tauri dev window (hot reload)
just build        Build production .app / .dmg
just test         Run all tests (Rust + TypeScript)
just lint         Lint everything (cargo clippy + ESLint, zero warnings)
just format       Auto-format everything (rustfmt + Prettier)
just typecheck    TypeScript type-check only
just clean        Remove all build artifacts
just check        Fast cargo check (no artifacts produced)
just test-rust    Rust tests only
just test-ts      TypeScript tests only
just stack        Show selected versions and manifest consistency
just doctor       Check actual development tool versions
just verify-environment  Fail on manifest/toolchain drift
just test-environment    Run environment-tooling regression tests
```

---

## Playlist Import

Nova supports direct M3U playlists and Xtream-style `get.php` URLs. Xtream URLs are routed
through the JSON API so live channels, VOD, and series can be fetched in parallel instead of
waiting on one large generated playlist.

Import progress is streamed to the UI with `playlist:import:progress`. The payload includes
the current stage (`connecting`, `receiving`, `persisting`, `complete`, or `failed`), parsed and
inserted counts, live/movie/series counts, elapsed time, skipped/failed counts, and any warning
or error. Initial import completion is tied to fetch, parse, and database persistence only;
slow optional enrichment should run later as background work.

The provider validation probe for the current large test playlist completed successfully twice:
`42,212` entries total (`9,099` live, `27,998` movies, `5,115` series) in about `2.1-2.4s`.
Use the probe before declaring import changes complete:

```bash
NOVA_PROBE_URL='<playlist url>' cargo run -p nova-xtream --example probe
```

## Browsing Imported Content

After an import completes, the Home view reloads sources and lets users browse all persisted
content for the selected playlist. The UI shows total counts for Live, Movies, and Series,
supports source selection, search, live country filtering, category filtering, and paginated
loading for large Live/Movie lists. The first page size is 200 rows; use **Load more** to fetch
the next batch instead of blocking the UI on tens of thousands of records.

Series are shown as summaries grouped by title, with episode detail available after selecting
a series. Series category metadata is stored on new imports; previously imported profile DBs
gain the column through migration, but old rows only receive categories after re-import.

---

## Project structure

```
nova/
├── apps/desktop/          Tauri app — the only deliverable
│   ├── src/               React 19 frontend (TypeScript, Tailwind 4)
│   └── src-tauri/         Rust backend (Tauri 2 commands, state)
├── crates/                Rust workspace members
│   ├── core/              Domain types, traits, errors
│   ├── m3u/               M3U parser
│   ├── xtream/            Xtream Codes API client
│   ├── epg/               XMLTV parser + normalizer
│   ├── hls-proxy/         Local HLS server (Axum)
│   ├── transcribe/        whisper.cpp FFI + streaming ASR
│   ├── translate/         NLLB / DeepL / Google translation backends
│   ├── media/             FFmpeg subprocess wrapper
│   ├── storage/           SQLite + migrations + repositories
│   ├── jobs/              Background job scheduler
│   ├── search/            FTS5 query builder
│   └── mcp/               Dev-only MCP server (NOVA_DEV=1)
├── packages/
│   ├── ui/                Shared React component library
│   └── types/             Generated TypeScript bindings (ts-rs output)
├── scripts/               Setup scripts (FFmpeg, Whisper, NLLB)
├── docs/                  IPC contract, translation pipeline, design tokens
└── .github/workflows/     CI (Rust + TypeScript checks on every push)
```

---

## Tech stack

| Layer | Choice |
|---|---|
| Desktop shell | Tauri 2 |
| Frontend | React 19, TypeScript 5.6 (strict), Vite 6 |
| Styling | Tailwind CSS 4, shadcn/ui |
| Video player | Vidstack + hls.js |
| Backend language | Rust 1.84, edition 2021 |
| Async runtime | Tokio 1 |
| Database | SQLite via sqlx 0.8 (WAL mode, FTS5) |
| HTTP | Axum 0.8 (proxy), reqwest 0.12 (client) |
| Transcription | whisper.cpp with Core ML backend |
| Translation | NLLB-200 distilled via CTranslate2 (Metal) |
| Package manager | pnpm 9 |
| Task runner | just |

---

## Architecture

See [`ARCHITECTURE.md`](./ARCHITECTURE.md) for the full design: data model, streaming pipelines,
transcription and translation detail, player UX spec, phase-by-phase implementation order,
and the definition of done for each phase.

---

## License

MIT — see [`LICENSE`](./LICENSE).
