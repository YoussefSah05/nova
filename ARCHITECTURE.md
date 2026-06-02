# IPTV Streaming Platform — Architecture & Initialization Brief

> Master document for project scaffolding.
> Project codename: **Stream** (placeholder — rename as desired).
> Version 2 — translation pipeline integrated.

---

## 1. Project overview

A local-first desktop IPTV streaming platform with on-device AI transcription **and translation**. Each profile imports its own M3U playlist; the app provides a Netflix-class UX over live channels, VOD, and series, with automatic transcription of VOD content, live captions for channels, and cross-language translation of either — all rendered in a designed "middle-box" overlay when in translation mode.

**Primary target**: macOS 14+ on Apple Silicon (M-series). Build pipeline must produce a signed, notarized `.dmg`.
**Secondary target**: Windows 11 (x64 + arm64). Build pipeline must produce a signed `.msi`. Not day-one; architecture must not preclude.

**Priority languages**: English, French, Arabic. The system must work for all 200+ NLLB-supported languages but defaults, fonts, and QA effort focus on these three.

### Non-goals (explicitly out of scope for v1)
- Multi-device sync, server mode, casting to other devices.
- DRM (Widevine/FairPlay/PlayReady).
- User accounts, payments, social features.
- Mobile.
- Speech-to-speech translation (we display text, not synthesized audio).

### Key constraints
- **Local-first**: every core feature runs on the user's machine. No cloud dependency for transcription or translation by default. Cloud translation backends (DeepL/Google) are opt-in only.
- **Multi-profile**: each profile has its own M3U source, EPG, watch history, transcription cache, translation cache, settings. Profile switch is instant.
- **Premium feel**: video must not stutter; UI must respond in <100ms; idle resource footprint <100MB RAM, <1% CPU when not playing.
- **Hardware-aware**: app probes the machine on first run and tunes defaults (Whisper model, NLLB model, concurrency limits, cache sizes).
- **Translation must feel free**: caching is aggressive. Translating the same VOD twice should be instant the second time.

---

## 2. Locked technology decisions

| Layer | Choice | Version target |
|---|---|---|
| Desktop shell | Tauri | 2.x latest stable |
| Daemon language | Rust | 1.80+, edition 2024 |
| Frontend framework | React | 19.x |
| Frontend language | TypeScript | 5.6+ (strict mode) |
| Build tool | Vite | 6.x |
| UI styling | Tailwind CSS | 4.x |
| Component primitives | shadcn/ui | latest |
| Video player core | **libmpv** (embedded, macOS via CAOpenGLLayer) — superseded Vidstack in Phase 5 | latest stable |
| HLS engine | local Axum proxy + libmpv demuxer; hls.js only on the WebKit fallback path | latest stable |
| Database | SQLite via `sqlx` | 0.8+, `sqlite` feature |
| Search index | SQLite FTS5 (built-in) | — |
| Async runtime | Tokio | 1.x latest |
| HTTP server (proxy) | Axum | 0.8+ |
| HTTP client | reqwest | latest, `rustls-tls` |
| XML parser (EPG) | quick-xml | latest |
| **Transcription** | whisper.cpp with Core ML backend | latest, `WHISPER_COREML=1` |
| **Translation (local)** | NLLB-200 distilled via CTranslate2 | latest, Metal backend |
| **Translation (cloud, optional)** | DeepL API, Google Translate API | — |
| Audio/video processing | FFmpeg | bundled binary, 7.x |
| Package manager (JS) | pnpm | 9+ |
| Toolchain manager | mise | latest |
| Task runner | just | latest |
| Linters | clippy (Rust), ESLint (TS) | — |
| Formatters | rustfmt, Prettier | — |
| Testing (Rust) | built-in + `insta` for snapshots | — |
| Testing (TS) | Vitest + Playwright for E2E | — |

### Deliberately rejected
- **Electron** — too heavy for this workload, baseline RAM unacceptable for premium positioning.
- **Python in the runtime path** — GIL issues, packaging headaches, slow startup. Used only for offline scripts (model conversion, etc.).
- **Node backend** — GC pauses during playback, weaker FFI to whisper.cpp / ctranslate2.
- **Video.js / Plyr** — Video.js is dated; Plyr is too opinionated. Vidstack is purpose-built for modern custom players.
- **PostgreSQL / external DB** — overkill for local-first; extra process to manage.
- **Meilisearch / Elastic** — SQLite FTS5 is plenty for a single-user corpus.
- **SeamlessM4T** — excellent quality but less mature on-device tooling. Revisit later.
- **Per-pair MarianMT models** — N×(N-1) models is bad UX. NLLB's one-model-many-pairs wins.

---

## 3. Repository layout

```
stream/
├── .editorconfig
├── .gitignore
├── .gitattributes
├── mise.toml
├── Justfile
├── README.md
├── ARCHITECTURE.md
├── package.json                 # pnpm workspace root
├── pnpm-workspace.yaml
├── Cargo.toml                   # Rust workspace root
├── rust-toolchain.toml
├── apps/
│   └── desktop/                 # Tauri app — the only deliverable
│       ├── package.json
│       ├── tsconfig.json
│       ├── vite.config.ts
│       ├── tailwind.config.ts
│       ├── index.html
│       ├── src/                 # React frontend
│       │   ├── main.tsx
│       │   ├── App.tsx
│       │   ├── routes/
│       │   ├── features/
│       │   │   ├── player/      # Includes caption renderer (standard + middle-box)
│       │   │   ├── browse/
│       │   │   ├── search/
│       │   │   ├── profile/
│       │   │   ├── transcript/  # Companion panel
│       │   │   └── settings/
│       │   ├── components/
│       │   ├── lib/
│       │   ├── styles/
│       │   └── types/           # Generated from Rust via ts-rs
│       └── src-tauri/
│           ├── Cargo.toml
│           ├── tauri.conf.json
│           ├── build.rs
│           ├── icons/
│           └── src/
│               ├── main.rs
│               ├── commands/
│               ├── state.rs
│               └── ipc.rs
├── crates/
│   ├── core/                    # Domain types, traits, errors
│   ├── m3u/                     # M3U parser
│   ├── xtream/                  # Xtream Codes JSON API client
│   ├── epg/                     # XMLTV parser + normalizer
│   ├── hls-proxy/               # Local HLS server (Axum)
│   ├── transcribe/              # whisper.cpp FFI + streaming
│   ├── translate/               # NLLB / DeepL / Google backends
│   ├── media/                   # FFmpeg wrapper
│   ├── storage/                 # SQLite + migrations + repositories
│   ├── jobs/                    # Background job scheduler
│   ├── search/                  # FTS5 query builder
│   └── mcp/                     # Dev-only MCP server
├── packages/
│   ├── ui/                      # Shared React components, design system
│   └── types/                   # Generated TS bindings (output of ts-rs)
│   ├── ffmpeg-recipes/
│   ├── hls-manifest/
│   ├── whisper-config/
│   ├── translate-config/
│   ├── caption-renderer/
│   ├── frontend-design/
│   ├── xmltv-epg/
│   ├── api-conventions/
│   ├── rust-patterns/
│   ├── sqlite-patterns/
│   └── video-player-ux/
├── scripts/
│   ├── convert-whisper-models.sh    # Whisper → Core ML conversion
│   ├── convert-nllb-models.sh       # NLLB → CTranslate2 conversion
│   ├── bundle-ffmpeg.sh             # Download + verify FFmpeg binary
│   └── seed-dev-data.ts
├── docs/
│   ├── design-tokens.md
│   ├── ipc-contract.md
│   ├── translation-pipeline.md      # Detailed pipeline doc
│   └── decisions/                   # ADRs
└── .github/
    └── workflows/
        ├── ci.yml
        └── release.yml
```

---

## 4. Toolchain pinning

### `mise.toml`
```toml
[tools]
rust = "1.84"
node = "22"
pnpm = "9"
just = "latest"
```

### `rust-toolchain.toml`
```toml
[toolchain]
channel = "1.84"
components = ["rustfmt", "clippy", "rust-analyzer"]
targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"]
```

### `package.json` (root)
- `"packageManager": "pnpm@9.x.y"` to enforce.
- Top-level scripts: `dev`, `build`, `test`, `lint`, `format`, `typecheck`.

### `Cargo.toml` (workspace)
- Members: all of `crates/*` plus `apps/desktop/src-tauri`.
- `resolver = "2"`.
- `[workspace.dependencies]` centralizes all crate versions.
- `[profile.release]`: `lto = "fat"`, `codegen-units = 1`, `strip = true`, `panic = "abort"`.

### `Justfile` (selected)
```just
default:
    @just --list

setup:
    mise install
    pnpm install
    cargo fetch
    ./scripts/bundle-ffmpeg.sh
    ./scripts/convert-whisper-models.sh
    ./scripts/convert-nllb-models.sh

dev:
    pnpm tauri dev

build:
    pnpm tauri build

test:
    cargo test --workspace
    pnpm test

lint:
    cargo clippy --workspace --all-targets -- -D warnings
    pnpm lint

format:
    cargo fmt --all
    pnpm format

typecheck:
    pnpm typecheck

clean:
    cargo clean
    pnpm clean
```

---

## 5. High-level architecture

```
┌────────────────────────────────────────────────────────────────────────┐
│                    Tauri Process (single binary)                       │
│                                                                        │
│  ┌──────────────────────────┐    ┌─────────────────────────────────┐   │
│  │  WKWebView (React)       │◄──►│  Rust Core (in-process)         │   │
│  │                          │    │                                 │   │
│  │  • Browse / Search       │    │  • Tauri commands               │   │
│  │  • Player chrome         │    │  • State store                  │   │
│  │  • Caption renderer      │    │  • Job scheduler                │   │
│  │    (standard / mid-box)  │    │  • IPC events → UI              │   │
│  │  • Profile picker        │    └──────────┬──────────────────────┘   │
│  │  • Transcript panel      │               │                          │
│  └──────────────────────────┘               │                          │
│           │ libmpv / proxy                  ▼                          │
│           ▼                       ┌─────────────────────────┐          │
│  ┌────────────────────┐           │  hls-proxy crate (Axum) │          │
│  │  localhost HLS     │◄──────────┤  • Manifest rewriting   │          │
│  │  127.0.0.1:<port>  │           │  • Subtitle injection   │          │
│  └────────────────────┘           │    (source + targets)   │          │
│                                   │  • Segment caching      │          │
│                                   └────┬────────────────────┘          │
│                                        │                               │
│                                        ▼                               │
│                                ┌──────────────────┐                    │
│                                │  Upstream IPTV   │                    │
│                                │  provider (HLS)  │                    │
│                                └──────────────────┘                    │
│                                                                        │
│  ┌──────────────────────────┐    ┌─────────────────────────────────┐   │
│  │  transcribe crate        │───►│  translate crate                │   │
│  │  • whisper.cpp FFI       │    │  • NLLB-200 via CTranslate2     │   │
│  │  • Core ML / ANE         │    │  • DeepL backend (opt-in)       │   │
│  │  • Streaming ASR         │    │  • Google backend (opt-in)      │   │
│  │  • Source-lang VTT       │    │  • Per-segment caching          │   │
│  └─────────┬────────────────┘    │  • Target-lang VTT output       │   │
│            │                     └────────┬────────────────────────┘   │
│            │                              │                            │
│  ┌─────────▼──────────────────────────────▼──────────────────────┐     │
│  │  storage crate (SQLite + FTS5)                                │     │
│  │  Per-profile DB: ~/Library/Application Support/Stream/...     │     │
│  └───────────────────────────────────────────────────────────────┘     │
└────────────────────────────────────────────────────────────────────────┘
```

### Process model
- **Single process**. Rust core runs in-process inside the Tauri binary.
- **Tokio multi-thread runtime**. UI on main thread (macOS requirement).
- **HLS proxy** binds to `127.0.0.1` on OS-assigned port at startup.

### IPC contract
- **Tauri commands** for request/response: `list_channels`, `play_vod(id, caption_mode)`, `enqueue_transcription`, `enqueue_translation`, `get_transcript`, `search`, `update_settings`, etc.
- **Tauri events** for streaming updates: `epg:refresh:progress`, `transcription:progress`, `transcription:complete`, `translation:progress`, `translation:complete`, `live:caption`, `live:caption:translated`, `playlist:import:progress`, `jobs:queue:changed`. Tauri event names must use alphanumeric characters plus `-`, `/`, `:`, or `_`; do not use dots.
- **Generated TypeScript bindings** via `ts-rs`.
- Contract in `docs/ipc-contract.md`, CI-enforced.

---

## 6. Data model (SQLite)

One DB per profile at `~/Library/Application Support/Stream/profiles/<profile_id>.db` plus top-level `app.db`.

### `app.db` schema
```sql
CREATE TABLE profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    avatar_color TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

### Per-profile schema
```sql
-- Source playlists
CREATE TABLE sources (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,            -- 'm3u' | 'xtream'
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    username TEXT,                 -- encrypted at rest
    password TEXT,
    last_refreshed_at INTEGER,
    auto_refresh_hours INTEGER DEFAULT 24
);

-- Channels (live)
CREATE TABLE channels (
    id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    stream_id TEXT,
    tvg_id TEXT,
    name TEXT NOT NULL,
    logo_url TEXT,
    logo_cached_path TEXT,
    region TEXT,
    country TEXT,
    category TEXT,
    raw_group TEXT,
    url TEXT NOT NULL,
    tvg_shift INTEGER DEFAULT 0,
    catchup_kind TEXT,
    catchup_days INTEGER,
    expected_language TEXT,        -- inferred from country/name, used as Whisper hint
    is_favorite INTEGER DEFAULT 0,
    is_hidden INTEGER DEFAULT 0,
    last_played_at INTEGER,
    position_in_category INTEGER
);
CREATE INDEX idx_channels_category ON channels(country, category);
CREATE INDEX idx_channels_tvg ON channels(tvg_id);

-- VOD
CREATE TABLE vod_items (
    id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    stream_id TEXT,
    name TEXT NOT NULL,
    year INTEGER,
    poster_url TEXT,
    backdrop_url TEXT,
    plot TEXT,
    rating REAL,
    duration_sec INTEGER,
    container TEXT,
    url TEXT NOT NULL,
    category TEXT,
    cast TEXT,
    director TEXT,
    genre TEXT,
    detected_language TEXT,        -- filled after transcription
    added_at INTEGER
);

-- Series
CREATE TABLE series (...);
CREATE TABLE series_seasons (...);
CREATE TABLE series_episodes (...);

-- EPG
CREATE TABLE programs (
    id INTEGER PRIMARY KEY,
    channel_tvg_id TEXT NOT NULL,
    title TEXT NOT NULL,
    sub_title TEXT,
    description TEXT,
    start_utc INTEGER NOT NULL,
    stop_utc INTEGER NOT NULL,
    category TEXT,
    icon_url TEXT
);
CREATE INDEX idx_programs_channel_time ON programs(channel_tvg_id, start_utc);

-- Transcripts (one per media item per source language)
CREATE TABLE transcripts (
    id INTEGER PRIMARY KEY,
    media_kind TEXT NOT NULL,       -- 'vod' | 'episode' | 'live_recording'
    media_id INTEGER NOT NULL,
    language TEXT NOT NULL,         -- BCP-47, source language
    model TEXT NOT NULL,            -- 'large-v3-turbo' etc.
    vtt_path TEXT NOT NULL,
    json_path TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    duration_processed_sec INTEGER,
    UNIQUE(media_kind, media_id, language, model)
);

-- Transcript segments (word-level timestamps included when Whisper provides)
CREATE TABLE transcript_segments (
    id INTEGER PRIMARY KEY,
    transcript_id INTEGER NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,          -- segment ordinal within transcript
    start_sec REAL NOT NULL,
    end_sec REAL NOT NULL,
    text TEXT NOT NULL,
    words_json TEXT,               -- optional: [{start, end, word}]
    speaker_id INTEGER             -- future: diarization
);
CREATE INDEX idx_segments_transcript_seq ON transcript_segments(transcript_id, seq);

-- TRANSLATIONS: cached per (segment, target_lang, backend)
CREATE TABLE transcript_translations (
    id INTEGER PRIMARY KEY,
    segment_id INTEGER NOT NULL REFERENCES transcript_segments(id) ON DELETE CASCADE,
    target_lang TEXT NOT NULL,       -- BCP-47
    text TEXT NOT NULL,
    backend TEXT NOT NULL,           -- 'nllb-200-distilled-600M' | 'nllb-200-distilled-1.3B' | 'deepl' | 'google'
    created_at INTEGER NOT NULL,
    UNIQUE(segment_id, target_lang, backend)
);
CREATE INDEX idx_translations_segment ON transcript_translations(segment_id, target_lang);

-- Generated translated-VTT files cached on disk; this table tracks them for invalidation
CREATE TABLE translated_vtt_files (
    id INTEGER PRIMARY KEY,
    transcript_id INTEGER NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
    target_lang TEXT NOT NULL,
    backend TEXT NOT NULL,
    vtt_path TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(transcript_id, target_lang, backend)
);

-- FTS5 over segments AND translations (single virtual table, content union)
CREATE VIRTUAL TABLE transcript_fts USING fts5(
    text,
    lang UNINDEXED,
    segment_id UNINDEXED,
    kind UNINDEXED,             -- 'source' | 'translation'
    tokenize='unicode61 remove_diacritics 2'
);
-- Triggers populate from transcript_segments AND transcript_translations.

-- Watch history
CREATE TABLE watch_history (
    id INTEGER PRIMARY KEY,
    media_kind TEXT NOT NULL,
    media_id INTEGER NOT NULL,
    position_sec REAL NOT NULL,
    duration_sec REAL,
    watched_at INTEGER NOT NULL,
    completed INTEGER DEFAULT 0,
    caption_mode TEXT,             -- 'off' | 'source' | 'translation:fr' | 'dual:ar:fr'
    PRIMARY KEY -- last-watched per (media_kind, media_id) via dedup query
);

-- Per-content caption preferences (sticky)
CREATE TABLE caption_preferences (
    id INTEGER PRIMARY KEY,
    scope TEXT NOT NULL,           -- 'channel' | 'vod' | 'source_lang'
    scope_key TEXT NOT NULL,       -- channel id, vod id, or BCP-47 source lang
    caption_mode TEXT NOT NULL,    -- as above
    updated_at INTEGER NOT NULL,
    UNIQUE(scope, scope_key)
);

-- Jobs queue
CREATE TABLE jobs (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,             -- 'transcribe_vod' | 'translate_transcript' | 'refresh_epg' | 'refresh_playlist' | 'cache_logo'
    payload TEXT NOT NULL,          -- JSON
    state TEXT NOT NULL,            -- 'queued' | 'running' | 'done' | 'failed' | 'cancelled'
    priority INTEGER DEFAULT 0,
    attempts INTEGER DEFAULT 0,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER
);
CREATE INDEX idx_jobs_state_priority ON jobs(state, priority DESC, created_at);

-- Per-profile settings
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL              -- JSON
);
```

All writes go through repository functions in `crates/storage`. WAL mode, `PRAGMA synchronous=NORMAL`.

---

## 7. Streaming pipeline

### Playlist import flow
1. User pastes M3U URL or Xtream credentials.
2. Xtream-style `get.php` URLs route to `xtream`, which fetches live, VOD, and series JSON endpoints in parallel. Plain playlists use the streaming `m3u` parser.
3. Parsed entries are persisted in batches inside one SQLite transaction. Current batch size is 1,000 entries per media table flush.
4. Progress streams via `playlist:import:progress` with stage, parsed/inserted counts, live/movie/series counts, failed/skipped counts, elapsed milliseconds, and terminal `done=true`.
5. Fetch and stalled-read timeouts prevent indefinite loading. Optional endpoint failures are reported as warnings when at least some entries import successfully.
6. Initial import completion is fetch → parse → persist. Slow enrichment such as logo caching, EPG refresh, stream probing, and expected-language inference must run as background work and must not block the terminal import event.
7. After completion, browse commands expose persisted content by source with counts, search, filters, and paginated Live/Movie loading. The UI must not treat the first returned page as the complete import.

### Playlist import validation
The current large provider test playlist has been validated through the headless probe. It routes as Xtream JSON API and imports `42,212` entries: `9,099` live, `27,998` movies, and `5,115` series. Repeated probe runs completed cleanly in about `2.1-2.4s`.

```bash
NOVA_PROBE_URL='<playlist url>' cargo run -p nova-xtream --example probe
```

### Imported content browse flow

The browse surface is a **Netflix-style hero + rails + grid** view on a **TanStack Query** data layer,
built with **shadcn/ui** primitives (`src/components/ui/*`: button, card, badge, input, tabs,
skeleton — Radix + cva) themed dark via shadcn CSS tokens added to `globals.css` `@theme`
(`--color-background/-foreground/-card/-primary/-muted/-border/-ring/...`, indigo `--primary`). All
catalog reads go through `features/browse/queries.ts` (typed query-key factory + `invoke` fetchers),
so every read is cached, deduped, and prefetchable; the app root provides the `QueryClient`
(`lib/queryClient.ts`, `staleTime` 5 min, no refetch-on-focus). All content normalizes to one
`BrowseItem` so cards/rails/grid stay tab-agnostic.

1. Home loads sources for the active profile and keeps an explicit selected source. Tab badge counts
   come from cached `count_*` queries; the country/genre filter options from cached
   `list_channel_countries` / `list_*_categories`. Country and category filtering are both
   index-backed (`idx_channels_source_country`, `idx_channels_category`).
2. **Browse mode** (no search/filter) → `BrowseRails`: a featured **`BrowseHero`** (spotlights the
   first Recently-Added item with a backdrop; reuses that rail's cached query — no extra IPC) above a
   vertical stack of horizontal rails — Continue Watching (profile-wide), Your Watchlist, Your
   Favourites, Recently Added, then one rail per genre (capped on the landing surface; the chip-bar /
   "See all" opens the grid for the rest). The hero + personal rails + first few genre rails are
   **eager** so the body always shows content immediately; deeper genre `Rail`s lazy-mount via
   IntersectionObserver. Explicit **loading / empty (`count === 0`) / error (retry)** states plus a
   `BrowseBoundary` error boundary mean the content area is never a blank void (the prior regression).
   This replaces the old `Shelves` component, whose per-tab refetch was the tab-switch lag.
3. **Search / filter / see-all** (query typed or a country/category chip active) → `BrowseGrid`: a
   responsive poster grid backed by `useInfiniteQuery` (200/page, auto-loads the next page near the
   end). Column count is derived from container width (ResizeObserver); items are chunked into rows
   and rows are **windowed** with `@tanstack/react-virtual` (`features/browse/VirtualRows.tsx`), so
   only on-screen rows mount. `keepPreviousData` means filter changes don't flash skeletons.
4. **Tab switches are cache reads, not refetches.** On source select, `prefetchSourceOverview` warms
   counts + categories for all three tabs + Continue Watching; on idle, `prefetchPersonalRails` warms
   each tab's personal rails. Combined with the 5-min `staleTime`, switching tabs is instant. Per-tab
   `scrollTop` is remembered for continuity; section swaps get a `.nova-fade-in` crossfade.
5. Favourite / watchlist toggles optimistically patch the item across every cached rail + grid page
   (`patchItemEverywhere`, with rollback on error), then invalidate only the membership rails.
   Refresh / re-import invalidates the whole source subtree.
6. Series cards open the detail view (episodes); live/movie cards open the player.
7. Browse errors surface in the UI and must not leave loading states active indefinitely.

### Catalog enrichment

A planned 4-layer pipeline cleans messy IPTV names + taxonomy so the home renders cleanly:
**(A) rules** (deterministic, offline) → **(B) on-device embeddings** (fastembed-rs: fuzzy de-dup,
zero-shot genre, semantic match) → **(C) opt-in TMDB** (canonical names, posters, overviews,
backdrops; cached, attributed) → **(D) tiny local LLM** (llama.cpp, residual disambiguation only).
Layers are idempotent, incremental, and never destroy raw data — they write a display layer.

Phase A is live: `nova_core::clean::polish_title` (`crates/core/src/clean.rs`) folds decorative
Unicode (`ᴴᴰ`→`HD`, fullwidth→ASCII) and strips release/quality/codec junk tokens
(`HD`/`1080p`/`x265`/`WEB-DL`/`MULTI`/`VOSTFR`…), conservatively. It runs at import in `push_entry`
(both M3U + Xtream) on live `name`, vod `clean_title`, series `series_title`.

Phase B is live (`crates/enrich`, fastembed-rs/`ort`, `multilingual-e5-small`): a post-import
background task (`commands/enrich.rs::run_enrichment`) embeds unenriched rows **in chunks** (emitting
real `embedding x/total` progress), groups true duplicates into a `dedup_group`, and zero-shot fills
missing genres — gated on `enrich_status IS NULL` (migration 0013), idempotent, emitting
`catalog:enrich:progress`. Grouping is gated on an exact rules key (`clean::dedup_key`, which strips
quality junk but **preserves distinguishing numbers** so `Sports 1` ≠ `Sports 2`); embedding cosine is
only a secondary guard. Grouping is **lossless** in the UI — variants ride along on the representative
card (`queries.ts` `collapseDuplicates`) and stay reachable, never deleted.

Phase C is live (opt-in TMDB): the same job, after the embedding stage, matches movies + series to
TMDB (`nova_enrich::tmdb` — async `reqwest` client returning cacheable raw bodies + pure
title+year scoring). The query is normalized for recall (`clean::search_title`) and tried as
movie+year → movie no-year → `/search/multi`, stopping once confident. Results are **tiered**: a
confident match writes the full display layer (`canonical_title`/`poster_url`/`backdrop_url`/
`overview`/`tmdb_genres`, genres mapped through `classify::canonical_genre`); a weak match with a
poster attaches **art only** (keeps the original title) for near-universal coverage; otherwise
`tmdb_status='unmatched'` for a later Phase D residual (migration 0014: columns on `vod_items`, a
`series_meta` sidecar, a `tmdb_cache`). It only runs when `ProfileSettings.tmdb_enabled` and a
user-supplied `tmdb_api_key` are set (offline-first). TMDB ToS requires attribution (surfaced in
Settings/About) and permits the local cache. Phase D (tiny local LLM, llama.cpp) remains planned. The
frontend prefers the enriched display layer (`queries.ts`, `BrowseHero`, `ContentDetail`) and upgrades
content in place via React Query invalidation on the terminal `catalog:enrich:progress` event. Movies
open a `ContentDetail` start screen (overview + Play / preferred-audio / Translate-soon) before
playback.

### Live playback flow
1. User clicks a channel. Frontend calls `play_channel(channel_id, caption_mode)`.
2. Rust returns a localhost proxy URL: `http://127.0.0.1:<port>/live/<channel_id>/index.m3u8`.
3. `hls-proxy` fetches upstream manifest, rewrites segment URLs through the proxy.
4. If `caption_mode != off`, manifest is augmented with `#EXT-X-MEDIA:TYPE=SUBTITLES` tracks:
   - Source language → `/live/<channel_id>/captions.<src>.vtt`
   - Each requested target language → `/live/<channel_id>/captions.<src>-to-<tgt>.vtt`
5. hls.js consumes the manifest. Hardware decode via VideoToolbox is automatic.
6. If live captions are on, `transcribe` crate spawns a streaming ASR task; if translation is on, output flows through `translate` crate before hitting the VTT endpoint.

### VOD playback flow
1. User clicks a VOD item. Frontend calls `play_vod(vod_id, caption_mode)`.
2. Rust resolves what's available:
   - **Transcript exists in source language**: serve it.
   - **Transcript missing**: enqueue `transcribe_vod` job; offer playback without captions while job runs; emit `transcription:progress` events.
   - **Translation requested and cached**: serve cached translated VTT.
   - **Translation requested, not cached**: enqueue `translate_transcript` job (runs after transcription if both pending); when complete, the proxy starts serving the new VTT, and the player picks it up via subtitle track refresh.
3. Proxy returns manifest with all available + requested caption tracks.
4. Player UI's caption picker reflects job state ("French translation — generating, 34%…").

### Catch-up / timeshift (Xtream)
Xtream pattern: `.../streaming/timeshift.php?username=X&password=Y&stream=<id>&start=<YYYY-MM-DD:HH-MM>&duration=<minutes>`. Surfaced in UI on EPG entries within the channel's `catchup_days` window. Transcription/translation works on these recordings the same as VOD.

---

## 8. Transcription pipeline

### Model strategy
- Ship with **no models bundled** (keeps installer small).
- On first run, recommended model: **`large-v3-turbo` quantized to `q5_0`** (~500 MB). Show why, allow skip.
- Convert to Core ML on first run via `whisper.cpp/models/generate-coreml-model.sh`. Takes 30–120s, one-time. UI: "Optimizing transcription for your M3…"
- Cache compiled `.mlmodelc` at `~/Library/Application Support/Stream/models/whisper/`.
- Advanced users can also download `large-v3` (full, ~1.5 GB) for archival-quality.

### Whisper invocation rules (as implemented — `whisper-cli` subprocess)
We drive the Homebrew `whisper-cpp` CLI as a subprocess (same pattern as ffmpeg/mpv), not FFI.
- **No `--task` flag** — transcription is whisper.cpp's default; passing `--task` prints help and
  exits. Cross-language uses the dedicated translation pipeline (§9), never Whisper's translate task.
- `--output-json-full` always (segments **and** per-token offsets → word-level timing for dual-mode
  and transcript-deep-link search).
- `--dtw <preset>` always (e.g. `large.v3.turbo`): DTW-aligned token timestamps. This is the
  timestamp-accuracy lever; without it token offsets are coarse heuristics and cue timing drifts.
- `--prompt "<tail>"` carries the previous window's last words for cross-boundary lexical context.
- `--language <bcp47>` when known; otherwise auto-detect (learned from the first window and persisted).
- `--no-context` for Arabic specifically (reduces hallucinations across silence).
- **VAD is deliberately disabled.** whisper.cpp's `--vad` remaps *segment* offsets to the original
  timeline but leaves *token* DTW offsets in the VAD-compressed timeline, corrupting the word timing
  we depend on. We chunk on whisper's natural pause-aligned segment boundaries instead (see below).

### VOD transcription flow (streaming, playback-anchored)
Rather than transcribing the whole file before showing anything, a **caption session**
(`apps/desktop/.../caption_session.rs`) streams cues from the playhead:
1. `start_caption_session{profileId, mediaKind, mediaId, fromSec, durationSec?, audioTrackIndex?,
   trackLang?}` ensures the model, replays any cached coverage instantly, then runs a
   **segment-aligned sliding cursor**.
2. Per window: `extract_audio_window` (ffmpeg `-ss/-t`, 16 kHz mono WAV, `-map 0:a:<n>` for the
   selected audio track) pulls up to 30s with a ~3s overlap → `transcribe_wav` (DTW + prompt +
   `--max-len`/`--split-on-word` for caption-sized cues) → `shift_transcript` to absolute time.
3. **Overlap stitch (LocalAgreement-style, Phase 6.3):** segments are kept by where they *end*
   (anything past the confirmed frontier `gap_start`), then their already-shown leading words are
   trimmed off — so a segment that spans the boundary in pauseless dialogue keeps its new content
   instead of being dropped (a start-based drop silently loses it). Only **complete** segments are
   committed (the trailing segment within ~1.5s of the hard 30s cut is held for the next window,
   re-read with context and re-admitted by the end-based keep — lossless), and the cursor advances
   to the last complete segment's end so a sentence is never hard-cut. Coverage is tracked as
   `[start,end)` intervals (persisted in `transcript_windows`, migration 0008) so a re-watch
   replays instantly.
4. Cues emit incrementally via `transcription:cues`; WebVTT + rows land in `transcripts` /
   `transcript_segments`. The session prefetches ~90s ahead and covers only what's watched (lazy).
5. One active session at a time (the open player); a new `start` cancels the previous.

### Per-track + refresh-stable cache (Phase 6.4)
A transcript is keyed by **`content_key` (a SHA-256 of the upstream stream URL) + `audio_track` +
`model`** (migration `0010`), not the DB `media_id`. So each audio track of multi-track content is
cached independently — switching the audio track re-targets transcription to that track from the
playhead and a previously-watched track replays instantly — and a playlist refresh that renumbers
`media_id` still resolves the same cached transcript (the row's `media_id` is lazily re-pointed; a
legacy row's empty `content_key` is backfilled on next watch). On-disk VTT/JSON are namespaced
`{kind}-{id}-t{track}-{lang}`.

### Caption appearance (Phase 6.2, per profile)
Caption styling is personalizable like Netflix (font size / family / colour / backdrop), stored in
the profile DB (`caption_style`, migration 0009; `nova_core::CaptionStyle`) via
`get_caption_style` / `set_caption_style`, and applied by the React `CaptionOverlay` (the
`CaptionStylePanel` "Aa" control). Pure presentation — it does not affect the transcript.

### Live transcription flow
1. User toggles captions on a channel.
2. `transcribe::live::Session` started: FFmpeg subprocess pulls channel audio → stdout PCM.
3. Whisper streaming with 5s window, 1s hop, **LocalAgreement-2** stabilization (only emit text agreed across 2 consecutive windows).
4. New stable segments pushed to in-memory ring buffer in proxy; `captions.<src>.vtt` endpoint serves rolling 30s of cues.
5. End-to-end latency target: 2–4s.
6. Session torn down on channel switch or caption disable.

### Skill: `whisper-config`
Captures model selection logic, VAD settings, language-specific tuning, VTT post-processing.

---

## 9. Translation pipeline

### Why a dedicated pipeline (not Whisper's `--task translate`)
Whisper's built-in translate mode only targets **English**. For Arabic → French, English → Arabic, etc., we need a true translation model. We use Whisper only for transcription (any language → same language text) and a separate translator for the cross-language step. This costs us slightly more compute but gives us all language pairs.

**Special-case optimization:** when target language is English, the system *can* skip NLLB and use Whisper's translate task in one pass. This is implemented as an opt-in setting ("Fast English mode") because the quality vs. the two-stage pipeline is comparable and the latency win is meaningful. **Default is off** to keep behavior consistent.

### Backends

| Backend | Default | Languages | Latency on M3 | Quality | Cost | Privacy |
|---|---|---|---|---|---|---|
| **NLLB-200 distilled 600M** (local) | ✅ default | 200 | ~100–300ms / line | Very good | Free | Local |
| **NLLB-200 distilled 1.3B** (local) | opt-in upgrade | 200 | ~250–500ms / line | Excellent | Free | Local |
| **DeepL API** | opt-in | 30 | ~150ms / line | Best-in-class | $$ | Cloud |
| **Google Translate API** | opt-in | 130+ | ~100ms / line | Excellent | $ | Cloud |

Default install ships **NLLB-200 distilled 600M** via CTranslate2 with Metal backend. Cloud backends require user-provided API key (encrypted at rest in settings).

### Model conversion (one-time, at install via `scripts/convert-nllb-models.sh`)
- Download NLLB-200 distilled from Hugging Face.
- Convert to CTranslate2 format with int8 quantization for Apple Silicon.
- Cache at `~/Library/Application Support/Stream/models/nllb/`.
- Verify with a test translation in each priority language.

### VOD translation flow
1. `translate_transcript` job: `{transcript_id, target_lang, backend}`.
2. Worker loads source segments from `transcript_segments`.
3. For each segment **not already cached** in `transcript_translations` for `(target_lang, backend)`:
   - **Batch** segments (~32 per batch) to amortize NLLB inference overhead.
   - Translate via chosen backend.
   - Insert into `transcript_translations`.
4. Generate translated VTT from cached translations, write to disk, insert into `translated_vtt_files`.
5. Emit `translation.complete` event. Proxy now serves the new VTT.
6. Cache hit on second request → instant.

### Live translation flow
1. User selects translation target alongside live captions.
2. Whisper streaming emits stable source segment.
3. Segment immediately handed to NLLB (or chosen backend) — single-segment translation, ~150ms.
4. Translated cue pushed to a separate ring buffer; `captions.<src>-to-<tgt>.vtt` endpoint serves it.
5. End-to-end latency: 3–5s (whisper 2-4s + translation 100-400ms + buffering). Acceptable; comparable to professional simultaneous interpretation.

### Caching strategy
- Translations cached per `(segment_id, target_lang, backend)`. Switching back to a previously-used translation is instant.
- Source transcripts cached per `(media_id, source_lang, whisper_model)`.
- Cache invalidation: only when the underlying transcript is regenerated (e.g. model upgrade). Cached translations follow their parent transcript.

### Dual mode (source + translation displayed together)
Both VTTs are loaded simultaneously by the renderer. UI design in §10.

### Language defaults (implemented; user-overridable)
- **Source-language preference**: defaults to none (auto-detect per content).
- **Reading-language preference**: from app locale, falls back to English.
- **Sticky rule (default-on)**: "When source language is X, translate to Y." Set automatically when user first chooses a translation for a given source language; persisted in `caption_preferences` with `scope='source_lang'`. Editable in settings.
- **Per-content override**: `scope='channel'` or `scope='vod'` rows override the source-lang default.

### Skill: `translate-config`
Captures backend selection logic, batching rules, language-pair-specific notes (e.g. NLLB-200 codes are not BCP-47; mapping table required), caching strategy.

---

## 10. Video player — premium-quality UI

### Foundation: embedded libmpv (revised in Phase 5)

> **Decision update (Phase 5):** the original Vidstack + hls.js foundation was replaced with an
> embedded **libmpv** engine. Vidstack/`<video>` could not robustly play the hostile, codec-diverse
> streams typical of IPTV; libmpv (the engine IINA uses) handles them natively. On macOS it renders
> into a `CAOpenGLLayer` sublayer of the WKWebView's layer tree (`crates/mpv-player/`,
> `apps/desktop/src-tauri/src/mpv_layer.rs`). The React control bar drives it through `mpv_*` Tauri
> commands and an event-driven `mpv:state` stream. The UX spec below (controls, keyboard, caption
> modes) is engine-agnostic and still applies. **AirPlay** is the one item the switch regressed —
> there is no `AVPlayer` to surface a route picker — so it needs a parallel AVPlayer route (deferred).

- libmpv: demux, decode (hardware via VideoToolbox `hwdec=auto-safe`), and render.
- Local Axum proxy: classifies the upstream and, when needed, remuxes/transcodes via FFmpeg.
- hls.js + native `<video>`: retained only on the WebKit fallback path.

### Performance budget (mandatory)
- Time-to-first-frame on channel switch: <800ms target, <1200ms ceiling.
- VOD scrubbing: instant thumbnail preview while dragging, frame-accurate on release.
- No layout shift when controls appear/disappear.
- 120fps overlay animations on ProMotion (M3 MacBook Pros).

### Controls UX
- Auto-hide after 2.5s idle; reappear on mouse move with 200ms fade-in.
- Tap to toggle play/pause (safe-zone around controls).
- Double-tap left/right edge to skip 10s (ripple feedback).
- Volume scrubber on hover of speaker icon.
- Quality selector shows actual resolutions from manifest.
- **Captions picker** is its own section — see below.
- Settings popover uses spring animation, not linear ease.

### Keyboard shortcuts
| Key | Action |
|---|---|
| Space / K | Play/pause |
| ← / → | Seek 5s |
| Shift+← / Shift+→ | Seek 10s |
| J / L | Seek 10s back/forward |
| ↑ / ↓ | Volume ±5% |
| M | Mute toggle |
| F | Fullscreen |
| Esc | Exit fullscreen |
| C | Cycle caption mode (off → source → translation → dual → off) |
| T | Toggle translation target picker |
| , / . | Frame step (paused) |
| 0–9 | Seek to N×10% (VOD only) |

### Gestures (fullscreen, trackpad)
- Two-finger horizontal swipe: seek scrub (VOD).
- Two-finger vertical on left half: brightness.
- Two-finger vertical on right half: volume.

### Caption rendering — TWO MODES

The player has a custom caption renderer (not native `<track>`) so we can implement both styles cleanly.

#### Mode A — Standard subtitles (source language only, when no translation requested)
- Position: bottom 10% of frame
- Up to 70% frame width, 2 lines max
- 18–22px text, thin outline, no background
- Native-track-like behavior, premium typography

#### Mode B — Middle-box (translation mode, dual mode)
Used whenever a translation is being displayed. This is the signature "translation card" look.

| Property | Specification |
|---|---|
| Position | Center horizontally, vertical center ~55% from top |
| Width | Auto-fit content, max 60% of frame width, min 30% |
| Background | `rgba(10, 10, 12, 0.78)` with `backdrop-filter: blur(20px) saturate(160%)` |
| Border | `1px solid rgba(255, 255, 255, 0.08)` |
| Border radius | 12px |
| Padding | 24px horizontal, 16px vertical |
| Shadow | `0 8px 32px rgba(0, 0, 0, 0.4)` |
| Typography (Latin) | Inter Variable, 22px, weight 400, line-height 1.5, tracking -0.01em |
| Typography (Arabic) | IBM Plex Sans Arabic, 24px, weight 400, line-height 1.6, RTL |
| Typography (CJK) | Noto Sans CJK, 22px, line-height 1.6 |
| Max lines | 3 |
| Entry animation | Fade-in (200ms) + scale from 0.98 to 1.0 (cubic-bezier(0.16, 1, 0.3, 1)) |
| Exit animation | Fade-out (150ms) |
| User repositionable | Yes, drag to reposition; remembered per-content |

#### Mode C — Dual mode (source + translation)
A single middle-box card containing both:
- **Source line** (top): 16px, weight 400, color `rgba(255, 255, 255, 0.55)`, language-appropriate font.
- **Divider**: 1px line, `rgba(255, 255, 255, 0.06)`, 8px vertical margin.
- **Translated line** (bottom): 22px, weight 400, color `rgba(255, 255, 255, 0.96)`, language-appropriate font.
- Same card chrome as Mode B.
- Word-level highlighting: hovering a word in the source highlights the aligned word in the translation (uses Whisper's word timestamps + NLLB's attention alignment — best-effort; gracefully degrades if unavailable).

### Captions picker UI
A dedicated subtitle button in the player chrome. Click opens a popover with three sections:

1. **Display mode**
   - Off
   - Source language: [detected as Arabic]
   - Translate to: [picker]
   - Dual (source + translation)

2. **Translate to** (visible when translation or dual selected)
   - Quick picks: English, French, Arabic (or user's top 3, learned over time)
   - Search field for other languages

3. **Advanced**
   - Style: Standard / Middle-box (when source-only, otherwise locked to middle-box)
   - Backend (if multiple installed): NLLB / DeepL / Google
   - Auto-translate when source is...: [language pair editor]

When a translation is generating, the row shows inline progress: "French — generating, 34%…"

### Captions sticky preference
Per content (channel, VOD, episode), the chosen mode is persisted to `caption_preferences` and restored next time. There's also a per-source-language default ("When source is Arabic, translate to French") that applies to new content.

### Picture-in-picture
- Native macOS PiP via WebKit's `requestPictureInPicture()`.
- Captions in PiP: Mode A or Mode B available; Mode B is auto-tightened (smaller padding, smaller text) because PiP is small.

### AirPlay
- **Implemented (Phase 5, macOS) as a parallel AVFoundation engine** (`src-tauri/src/airplay.rs`):
  the libmpv embed has no `AVPlayer`, so AirPlay runs on a separate `AVPlayer` with
  `allowsExternalPlayback`, fed the **proxy HLS URL** so hostile IPTV becomes Apple-TV-playable. A
  native `AVRoutePickerView` is overlaid on the window `contentView` (above the WKWebView). When a
  route engages, mpv pauses and the `AVPlayer` takes over playback/position; on disconnect, mpv
  resumes at the handed-back position. Handoff is orchestrated frontend-side via the `airplay:status`
  / `airplay:picker-closed` events (`features/player/useAirPlay.ts`).
- The button is always shown (route detection is unreliable as a gate); the picker is positioned
  whenever the controls are visible and no route is engaged.
- AirPlay'd captions burn-in the source-language VTT only (AirPlay subtitle track support is inconsistent across receivers). When in translation mode and user AirPlays, show a one-time toast: "Captions will display in source language on AirPlay devices."

### Transcript companion panel
- Optional side panel synced to playback time.
- Shows source by default; if translation is active, shows source + translation in parallel columns.
- Click a line to seek there.
- Search within transcript with highlighted matches.
- This is genuinely novel for IPTV apps.

### Skill: `caption-renderer`
Captures the rendering rules, font choices per script, layout math, dual-mode alignment behavior.

### Skill: `video-player-ux`
Captures all of the above as a checklist verified during player work.

### Cinema-mode episode browser (cleanup phase)
For a series episode, the player's cinema panel shows a season selector + a strip of that season's
episodes (current one highlighted); clicking switches playback in place via `onPlayMedia`. Siblings
come from `list_player_episodes(profileId, episodeId)`, which resolves the episode's series and reuses
the same lazy Xtream fetch as `list_series_episodes`. When `ProfileSettings.autoplayNext` is on, the
player advances to the next episode (by season/episode order) on playback end.

### Window shell (cleanup phase)
The window uses macOS `titleBarStyle: "Overlay"` (floating traffic-lights, full-bleed content). The
move-handle is provided by `data-tauri-drag-region` on the persistent top chrome (browse header,
profile-picker top strip via `components/DragRegion.tsx`, settings header) — double-click maximizes;
edge-resize is native. This needs `core:window:allow-start-dragging` +
`core:window:allow-internal-toggle-maximize` in `capabilities/default.json`. Header content is left-
inset (~`pl-20`) so it clears the traffic-lights. `tauri-plugin-window-state` persists window
size/position/maximized across launches (`window-state:default` permission).

### Settings + per-profile settings store (cleanup phase)
A `SettingsModal` (header gear) with four sections — Playback & Captions, Transcription & Cache,
Sources & EPG, Profile & About. Viewing/behaviour settings live per-profile in `profile_settings`
(migration 0011, `nova_core::ProfileSettings` as a JSON blob; `get/set_profile_settings`), alongside
the existing per-profile `caption_style`. Sections reuse existing commands and add maintenance ones:
`update_profile`, `delete_source`, `set_source_auto_refresh`, `remove_whisper_model`,
`get_transcription_cache_stats`, `clear_transcription_cache`, `get_app_info`. Future sections
(appearance/UI language, shortcuts, parental controls, network/proxy, diagnostics) are deferred.

---

## 11. Engineering reference notes

Domain rules for each area are kept as short reference notes.

| Skill | Purpose |
|---|---|
| `ffmpeg-recipes` | Canonical FFmpeg commands: HLS repackaging, audio extraction (16kHz mono PCM), thumbnail sprite gen, probe-for-codec. |
| `hls-manifest` | Manifest rewriting rules: subtitle track injection (source + translations), segment URL rewriting, master playlist structure. |
| `whisper-config` | Model selection, VAD, language tuning (esp. Arabic), VTT post-processing, word-timestamp handling. |
| `translate-config` | NLLB/DeepL/Google backend selection, language-code mapping (BCP-47 ↔ NLLB-200 codes), batching, caching, error handling. |
| `caption-renderer` | Rendering rules for both modes, fonts per script, RTL handling, dual-mode alignment, repositioning behavior. |
| `xmltv-epg` | EPG parsing rules, timezone normalization, channel ID matching. |
| `frontend-design` | Design tokens, motion timing, typography scale, spacing rhythm, component patterns. |
| `video-player-ux` | The mandatory player UX checklist from §10. |
| `api-conventions` | Tauri command naming, error shapes, event payload conventions. |
| `rust-patterns` | Async patterns, error handling with `thiserror`, repository pattern, FFI safety wrappers. |
| `sqlite-patterns` | Migration conventions, query patterns, FTS5 best practices, WAL tuning. |

Each note covers scope, rules, examples and pitfalls, and is updated as the project develops.

---

## 12. MCP integration plan

Build a thin MCP server inside the app (gated to localhost, dev mode only) exposing daemon APIs as MCP tools:

| Tool | Purpose |
|---|---|
| `list_profiles` | Read profile registry |
| `list_channels(filter)` | Browse channels during dev |
| `get_transcript(media_id)` | Fetch a transcript |
| `get_translation(media_id, target_lang)` | Fetch a translation |
| `enqueue_transcription(media_id, lang)` | Trigger transcription |
| `enqueue_translation(transcript_id, target_lang, backend)` | Trigger translation |
| `list_jobs(state)` | Inspect queue |
| `search_transcripts(query, lang_filter)` | Test FTS5 across source + translations |
| `tail_logs(level, n)` | Recent log lines |
| `db_query(sql)` | Read-only query (dev mode only) |

Server at `crates/mcp`, mounted at `127.0.0.1:<mcp_port>` only when `STREAM_DEV=1`.

---

## 13. First-run experience

1. **Welcome screen**. Brand mark, single CTA "Get started".
2. **Hardware probe** (background). Chip, RAM, disk free, GPU/ANE.
3. **Create first profile**. Name + avatar color.
4. **Add source**. Paste M3U URL, Xtream credentials, or skip.
5. **Download Whisper model**. Recommended `large-v3-turbo q5_0`. Background download with progress.
6. **Download translation model**. Recommended NLLB-200 distilled 600M. Same UX. **Skippable** — translation is also useful but not required to start watching.
7. **Compile for Apple Silicon**. "Optimizing for your Mac (one-time, ~2 min)…" with progress.
8. **First import**. Streaming progress: parsed N entries, cached M logos, fetched EPG.
9. **Set reading language**. "When content isn't in [English], offer translation to [English]." Defaults from system locale; user can change.
10. **Home**. Browse view. Continue Watching empty, Recently Added populated.

Target wall-clock: under 3 minutes on a typical connection (longer if both ML models are downloaded). Models can be deferred to "Settings → Models" if user skips.

---

## 14. Build, sign, distribute

### macOS
- Apple Silicon first; universal binary later via `cargo build --target` + `lipo`.
- Code signing: Developer ID Application cert via `tauri.conf.json` `bundle.macOS.signingIdentity`.
- Notarization: `xcrun notarytool` in release workflow.
- Output: `.dmg` with custom background, drag-to-Applications.

### Windows (later)
- MSI via WiX through Tauri.
- Code signing with EV cert.

### Updates
- Tauri built-in updater pointing at GitHub Releases.
- Delta updates not necessary at this scale.

### Model distribution
- Whisper and NLLB models downloaded post-install from Hugging Face mirrors.
- Bundle does NOT include models — keeps installer to ~30MB instead of ~2GB.
- Models verified by SHA-256 against pinned manifests in `scripts/models.json`.

---

## 15. Initial implementation order

Each step ends with something runnable.

**Phase 0 — Scaffolding (1 session)**
1. Repo layout per §3.
2. `mise.toml`, `rust-toolchain.toml`, `Justfile`, `pnpm-workspace.yaml`, `Cargo.toml` workspace.
3. Tauri 2 + React + TS + Vite + Tailwind 4 + shadcn/ui.
4. Stub all crates in `crates/` (including `translate`) with empty `lib.rs`.
5. ESLint, Prettier, clippy, rustfmt. `just lint`, `just typecheck` pass.
6. CI workflow on every push.
7. Reference notes for each area in §11 (including `translate-config`, `caption-renderer`).

**Phase 1 — Profiles and storage**
1. `storage` crate with `app.db` migrations.
2. Profile CRUD.
3. Netflix-style profile picker.

**Phase 2 — M3U import (vertical slice)**
1. `m3u` crate streaming parser.
2. Xtream detection routing to `xtream` crate stub.
3. Per-profile DB on profile create.
4. UI: "Add source" form with progress.
5. Logo caching via `jobs` crate.

**Phase 3 — Browse UI**
1. Source selector, full import counts, country/category facets, and search-as-you-type. **Implemented.**
2. Live and Movie pagination for large imports. **Implemented.**
3. VOD/Series browsing and series episode detail. **Implemented.**
4. Favorites, Continue Watching, Recently Added. **Pending.**

**Phase 4 — EPG**
1. `epg` crate XMLTV parser.
2. `xtream` crate `xmltv.php` fetch.
3. Now-and-next + full guide.
4. Background refresh.

**Phase 5 — Playback (standard captions only)**
1. `hls-proxy` crate (Axum) passthrough + route-aware remux/transcode.
2. Embedded libmpv engine (`crates/mpv-player/`, CAOpenGLLayer on macOS) — replaced Vidstack.
3. All keyboard shortcuts, control chrome (incl. audio-track / quality / speed menus, frame-step).
4. Mode A caption renderer (standard subtitles).
5. Watch history + resume.
6. AirPlay via parallel AVFoundation engine (macOS) — proxy-fed `AVPlayer` + native route picker.
7. Lazy Xtream `get_series_info` episode fetch (real seasons/episodes on series open).
8. Canonical genre classification (`crates/core/src/classify.rs`) + genre chip-bar browse filter.
9. Refresh playlist (atomic re-fetch + reload) preserving favorites/watchlist/continue-watching.
   (Deferred to a follow-up: cross-platform render handlers for Windows/Linux.)

**Phase 6 — Transcription (VOD)** — 🟡 implemented (`feat/phase-6-transcription`)
1. **Decision update:** `whisper-cli` is driven as a **subprocess** (Homebrew `whisper-cpp`,
   Core ML), not linked via FFI — same external-binary pattern as ffmpeg/mpv. The `transcribe`
   crate downloads the prebuilt **GGML + Core ML** model on first run (no Python conversion; the
   UI prompts for the one-time ~500MB download). whisper.cpp has **no `--task` flag** (transcribe
   is the default); word timestamps come from `--output-json-full`.
2. **Streaming, playback-anchored captions (Phase 6.1):** rather than batch-transcribing the whole
   file (which left the user at 0% for minutes), a **caption session** transcribes ~30s windows
   starting at the playhead (`nova_media::extract_audio_window` + `transcribe_wav` +
   `shift_transcript`), emits cues incrementally, prefetches ~90s ahead, and **lazily** covers only
   what's watched. First caption appears in seconds. One session at a time
   (`apps/desktop/.../caption_session.rs`).
3. Persistence/cache: `transcripts` + `transcript_segments` (migration 0007) plus `transcript_windows`
   coverage + `complete` flag (migration 0008). Cached windows replay instantly on re-watch; the
   on-disk VTT is refreshed per window.
4. Commands: `start_caption_session` / `set_caption_playhead` / `stop_caption_session`,
   `get_model_status`, `download_whisper_model`; events `transcription:cues` / `transcription:status`,
   `model:download:progress`.
5. **Caption delivery (continues the Phase 5 libmpv switch):** cues render through the React
   `CaptionOverlay`, **not** mpv's native renderer and **not** the proxy `/captions` manifest path
   (so the Phase 7 translation card reuses the same cue stream). The proxy `/captions` endpoint
   stays a stub (relevant only to the AirPlay/AVPlayer path).

**Phase 7 — Translation (VOD)**
1. `translate` crate with NLLB-200 distilled 600M via CTranslate2.
2. Model download + conversion in first-run + Settings.
3. Translation jobs run after transcription.
4. Per-segment caching.
5. Translated VTT served by proxy.
6. **Mode B (middle-box) caption renderer**.
7. **Mode C (dual) caption renderer**.
8. Captions picker UI with display-mode + target-language sections.
9. Sticky per-content and per-source-language preferences.

**Phase 8 — Search**
1. `search` crate over FTS5 (source + translations).
2. Global search.
3. Transcript-deep-link search.

**Phase 9 — Live transcription**
1. Streaming ASR session in `transcribe`.
2. Rolling captions VTT served by proxy.
3. Toggle in player UI.

**Phase 10 — Live translation**
1. Live streaming pipeline: whisper → NLLB → VTT.
2. Player picker supports target language for live captions.
3. End-to-end latency budget verified.

**Phase 11 — Cloud translation backends (optional)**
1. DeepL backend in `translate` crate.
2. Google backend in `translate` crate.
3. Settings: API key entry, backend selection.
4. Per-job backend override.

**Phase 12 — Polish**
1. Transcript companion panel synced to playback.
2. Thumbnail sprites for scrubbing.
3. Word-level alignment in dual mode.
4. Settings: model picker, language preferences, refresh intervals.
5. About / acknowledgements / licenses.

**Phase 13 — Distribution**
1. Code signing + notarization.
2. Auto-updater wired up.
3. Beta release.

---

## 16. Open questions (defaults assumed, called out for override)

The following defaults are assumed by this document. Each can be revisited.

| Question | Default | Override how |
|---|---|---|
| Final product name | "Stream" (placeholder) | Decide before Phase 0 |
| Bundle identifier | `com.example.stream` (placeholder) | Decide before Phase 0 |
| Apple Developer Team ID | None | Required before Phase 13 |
| EPG source | Provider's Xtream `xmltv.php` if available, else user-provided URL | Per-source UI |
| Adult-content gating | PIN-protected per profile, on by default if categories detected | Per-profile setting |
| Telemetry / crash reporting | None (local-first ethos) | Could add Sentry with opt-in |
| Project license | MIT | Decide before publishing |
| Default behavior when source ≠ reading language | Stay off until user opts in once; then sticky per source-language | Settings: "auto-offer translation" toggle |
| Live captions + live translation as one toggle or two | Two distinct toggles (Captions on/off, Translation on/off with target picker) | Could collapse later |
| Dual mode availability | Yes, included from Phase 7 | Could be hidden behind "advanced" toggle |
| DeepL/Google backends | Available from Phase 11 (post-MVP) | Could be brought into MVP if needed |
| Fast-English-mode (Whisper translate task → English in one pass) | Off by default; opt-in setting | Settings toggle |

---

## 17. Notes on premium feel

- **Typography**: Inter Variable (Latin), IBM Plex Sans Arabic or Cairo (Arabic), Noto Sans CJK (CJK). One scale: 12/14/16/18/22/28/36. Two weights: 400 and 500.
- **Motion**: every state change has motion. Entries: `cubic-bezier(0.16, 1, 0.3, 1)`. Exits: `cubic-bezier(0.7, 0, 0.84, 0)`. 200ms default; 400ms for layout shifts.
- **Color**: dark mode first. Surface `#0a0a0c`, never pure black. One brand accent color.
- **Iconography**: Lucide React, outlined, 1.5px stroke. Never mixed with filled.
- **Loading**: skeletons over spinners. Spinners only after 400ms.
- **Sound**: optional, off by default. If on: subtle UI ticks at -30dB, never on hover.
- **Empty states**: every list, panel, grid has a designed empty state with one action.
- **Translation card**: the middle-box rendering is the signature visual of this app. It must look better than anything Netflix/Disney+/Apple TV+ ships.

---

## 18. Definition of done for Phase 0

When scaffolding is finished:

- `just setup` completes on a fresh clone (without model downloads; those are deferred to first-run).
- `just dev` opens a Tauri window showing a placeholder home screen.
- `just lint` passes with zero warnings.
- `just typecheck` passes.
- `just test` runs (smoke test minimum).
- CI runs the same on push and passes.
- All crates in `crates/` (including `translate`) exist as empty libraries that build.
- `README.md` documents `just setup` → `just dev`.

If any fail, Phase 0 is not done.

---

End of brief.
