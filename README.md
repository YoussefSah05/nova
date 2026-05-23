# Nova

A local-first desktop IPTV streaming platform with on-device AI transcription and translation.

Import any M3U playlist and watch live channels, VOD, and series with Netflix-class UX.
Transcribe VOD content on your Mac (whisper.cpp, Apple Neural Engine). Translate to any
language via NLLB-200 — all on device, no cloud required. A signature "translation card"
overlay makes watching content in any language effortless.

**Status**: Phase 2 — M3U/Xtream import complete and validated. Phase 3 browse foundation
is implemented for imported content; favourites and playback-facing shelves are still pending.
See `ARCHITECTURE.md` for the full roadmap.

---

## Requirements

| Tool | Version | Install |
|---|---|---|
| [mise](https://mise.jdx.dev) | latest | `curl https://mise.run \| sh` |
| Rust | 1.84 | via mise |
| Node.js | 22 | via mise |
| pnpm | 9 | via mise |
| just | latest | via mise |

> macOS 14+ on Apple Silicon is the primary target. Intel Mac and Windows are planned.

---

## Onboarding

```bash
# 1. Clone
git clone https://github.com/YoussefSah05/nova.git
cd nova

# 2. Install all toolchain dependencies and fetch packages
just setup

# 3. Open the dev window
just dev
```

`just setup` runs once on a fresh clone:
1. Installs pinned Rust, Node, pnpm, and `just` via `mise install`.
2. Installs all JavaScript packages via `pnpm install`.
3. Fetches all Rust crate dependencies via `cargo fetch`.
4. Runs placeholder scripts for FFmpeg, Whisper, and NLLB models
   (actual downloads deferred to first-run in Phase 6/7).

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
